//! The Windows Update step: reports available Windows Updates through the
//! PSWindowsUpdate PowerShell module when it is installed, falling back to
//! `usoclient` when PSWindowsUpdate is missing.
//!
//! Detection checks whether PowerShell (`pwsh` preferred, then `powershell`)
//! exists and the PSWindowsUpdate module is available; when the module is
//! missing it checks whether `usoclient` exists instead. `check()` runs
//! `Get-WindowsUpdate` through PowerShell (or `usoclient StartScan` under the
//! fallback) and parses the output into outdated items. Neither `detect` nor
//! `check` changes anything, so the step is safe in `--check` mode. `run()`
//! reports the same items without installing them — this is a reporting-only
//! step. The step is only available on Windows.
//!
//! A run scans Windows Update exactly once: the check before the step fetches
//! the listing and caches it (`last_items`), and the step's own report (plus
//! the runner's after-run check) reuses the cached listing instead of
//! querying again. This mirrors how the firmware step reuses `last_list`.

use std::sync::Mutex;

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::restart_needed;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The Windows Update reporting step.
pub const WINDOWS_UPDATE: StepId = "windows-update";

/// Windows Update only exists on Windows.
const WINDOWS_UPDATE_PLATFORMS: [Platform; 1] = [Platform::Windows];

/// Registers the Windows Update step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(WindowsUpdateStep::new())
}

/// Which source the step uses to learn about pending Windows Updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateBackend {
    /// `Get-WindowsUpdate` through the PSWindowsUpdate PowerShell module.
    PsWindowsUpdate,
    /// `usoclient StartScan`, used when PSWindowsUpdate is not installed.
    Usoclient,
    /// Neither backend is available; Windows Update cannot be checked.
    None,
}

struct WindowsUpdateStep {
    /// The result of the last Windows Update scan, so a run only scans
    /// Windows Update once: the check before the step fetches and caches it
    /// here, and the step's own report (plus the runner's after-run check)
    /// reuse it instead of scanning again.
    last_items: Mutex<Option<WindowsUpdateListing>>,
    /// Which backend produced `last_items`, so the check reuses the same
    /// backend instead of re-detecting it.
    backend: Mutex<Option<UpdateBackend>>,
}

/// The result of one Windows Update scan: the pending updates and whether
/// installing them requires the machine to reboot.
#[derive(Clone, Default)]
struct WindowsUpdateListing {
    items: Vec<OutdatedItem>,
    reboot_pending: bool,
}

impl WindowsUpdateStep {
    fn new() -> Self {
        Self {
            last_items: Mutex::new(None),
            backend: Mutex::new(None),
        }
    }

    /// Resolves the backend to use, detecting it once and caching the result.
    fn backend(&self) -> Result<UpdateBackend, StepError> {
        if let Ok(b) = self.backend.lock() {
            if let Some(backend) = *b {
                return Ok(backend);
            }
        }
        let backend = self.detect_backend()?;
        if let Ok(mut b) = self.backend.lock() {
            *b = Some(backend);
        }
        Ok(backend)
    }

    /// Detects which backend is available: PSWindowsUpdate first, then usoclient.
    fn detect_backend(&self) -> Result<UpdateBackend, StepError> {
        if let Some(powershell) = powershell_command() {
            let output = CommandSpec::new(powershell)
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    "Get-Module -ListAvailable PSWindowsUpdate | Select-Object -ExpandProperty Name",
                ])
                .neutral_locale()
                .run();
            if let Ok(output) = output {
                if !output.stdout().trim().is_empty() {
                    return Ok(UpdateBackend::PsWindowsUpdate);
                }
            }
        }
        if usoclient_command().is_some() {
            return Ok(UpdateBackend::Usoclient);
        }
        Ok(UpdateBackend::None)
    }

    /// The pending Windows Updates, fetched once and cached for the run.
    fn listing(&self) -> Result<WindowsUpdateListing, StepError> {
        let cached = self.last_items.lock().ok().and_then(|last| last.clone());
        match cached {
            Some(listing) => Ok(listing),
            None => {
                let listing = self.fetch_listing()?;
                if let Ok(mut last) = self.last_items.lock() {
                    *last = Some(listing.clone());
                }
                Ok(listing)
            }
        }
    }

    /// Fetches the pending Windows Updates through the detected backend.
    fn fetch_listing(&self) -> Result<WindowsUpdateListing, StepError> {
        match self.backend()? {
            UpdateBackend::PsWindowsUpdate => {
                let Some(powershell) = powershell_command() else {
                    return Ok(WindowsUpdateListing::default());
                };
                let output = CommandSpec::new(powershell)
                    .args([
                        "-NoProfile",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-Command",
                        "Import-Module PSWindowsUpdate; Get-WindowsUpdate | Select-Object Title, KB, Version, RebootRequired | ConvertTo-Json",
                    ])
                    .neutral_locale()
                    .run()
                    .map_err(|error| StepError::check_from_process(WINDOWS_UPDATE, &error))?;
                let items = parse_windows_update(output.stdout());
                Ok(WindowsUpdateListing {
                    items,
                    reboot_pending: reboot_pending(output.stdout()),
                })
            }
            UpdateBackend::Usoclient => {
                let Some(usoclient) = usoclient_command() else {
                    return Ok(WindowsUpdateListing::default());
                };
                let output = CommandSpec::new(usoclient)
                    .args(["StartScan"])
                    .neutral_locale()
                    .run()
                    .map_err(|error| StepError::check_from_process(WINDOWS_UPDATE, &error))?;
                let items = parse_usoclient_scan(output.stdout());
                Ok(WindowsUpdateListing {
                    items,
                    reboot_pending: false,
                })
            }
            UpdateBackend::None => Ok(WindowsUpdateListing::default()),
        }
    }

    /// Reports the Windows Updates as outdated, honouring the package
    /// include/exclude list. Reuses the cached listing so `check` does not
    /// scan Windows Update a second time.
    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        let listing = self.listing()?;
        let filter = options.filters().packages();
        let report = CheckReport::from_items(
            listing
                .items
                .into_iter()
                .filter(|item| filter.includes(item.name())),
        );
        Ok(report)
    }

    /// Reports the available updates without installing them. Reuses the
    /// cached listing so `run` does not scan Windows Update a second time.
    fn report(&self) -> Result<(), StepError> {
        let listing = self.listing()?;
        let items = listing.items;
        if items.is_empty() {
            human!("windows-update: no Windows Updates available");
        } else {
            human!("windows-update: {} update(s) available", items.len());
            for item in &items {
                human!("  - {item}");
            }
        }
        Ok(())
    }
}

impl Step for WindowsUpdateStep {
    fn id(&self) -> StepId {
        WINDOWS_UPDATE
    }

    fn display_name(&self) -> &str {
        "Windows Update"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        match self.backend()? {
            UpdateBackend::PsWindowsUpdate | UpdateBackend::Usoclient => Ok(Detection::Installed),
            UpdateBackend::None => Ok(Detection::NotInstalled),
        }
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        WindowsUpdateStep::check_with_options(self, options)
    }

    /// Reports the available Windows Updates without installing them. This is a
    /// reporting-only step: `run()` prints what `check()` found and returns
    /// success, so a run never fails because updates are pending.
    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        self.report()?;
        if self.listing()?.reboot_pending {
            restart_needed::record();
            human!("windows-update: installing these updates requires a reboot");
        }
        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &WINDOWS_UPDATE_PLATFORMS
    }
}

/// Returns the PowerShell executable to use: `pwsh` (PowerShell 7) when it
/// exists, otherwise `powershell` (Windows PowerShell 5.1). Returns `None`
/// when neither is on `PATH`. The resolved absolute path is returned so the
/// exact executable found is what runs, without a second `PATH` lookup that
/// could land on a different (e.g. system-installed) PowerShell.
fn powershell_command() -> Option<std::path::PathBuf> {
    crate::executable::resolve("pwsh").or_else(|| crate::executable::resolve("powershell"))
}

/// Returns the `usoclient` executable to use when PSWindowsUpdate is missing,
/// or `None` when it is not on `PATH`. `usoclient` only exists on Windows, so
/// this resolves to `None` on every other platform.
fn usoclient_command() -> Option<std::path::PathBuf> {
    crate::executable::resolve("usoclient")
}

/// Parses the output of `usoclient StartScan` into outdated items.
///
/// `usoclient` does not enumerate individual updates, so when the scan reports
/// that updates are pending the step reports a single `Windows Update` item.
/// When the scan reports nothing pending, the report is empty.
///
/// The scan output is expected to announce pending updates on a line that
/// starts with `Updates found:` (for example `Updates found: 2`). Any other
/// output means no pending updates could be determined.
fn parse_usoclient_scan(output: &str) -> Vec<OutdatedItem> {
    let pending = output.lines().any(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with("updates found:")
    });
    if pending {
        vec![OutdatedItem::new("Windows Update").with_reason("updates pending")]
    } else {
        Vec::new()
    }
}

/// Parses the JSON output of `Get-WindowsUpdate | ... | ConvertTo-Json` into
/// outdated items. A single object or an array of objects is accepted.
///
/// Each update's `Title` is the item name, `Version` is the available version
/// (the currently installed version is not reported by `Get-WindowsUpdate`),
/// and `KB` is recorded as a reason when present.
fn parse_windows_update(output: &str) -> Vec<OutdatedItem> {
    let value: serde_json::Value = match serde_json::from_str(output) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let updates = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(_) => vec![value],
        _ => return Vec::new(),
    };
    updates
        .into_iter()
        .filter_map(|update| {
            let title = update.get("Title")?.as_str()?;
            let mut item = OutdatedItem::new(title);
            if let Some(version) = update.get("Version").and_then(|v| v.as_str()) {
                item = item.with_latest(version);
            }
            if let Some(kb) = update.get("KB").and_then(|v| v.as_str()) {
                if !kb.is_empty() {
                    item = item.with_reason(format!("KB {kb}"));
                }
            }
            Some(item)
        })
        .collect()
}

/// Whether the `Get-WindowsUpdate` output says that installing the listed
/// updates requires the machine to reboot. Each update object may carry a
/// `RebootRequired` flag; when any of them is `true`, a reboot is pending.
fn reboot_pending(output: &str) -> bool {
    let value: serde_json::Value = match serde_json::from_str(output) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let updates = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(_) => vec![value],
        _ => return false,
    };
    updates
        .into_iter()
        .any(|update| update.get("RebootRequired").and_then(|v| v.as_bool()) == Some(true))
}

#[cfg(test)]
mod tests {
    use super::{
        parse_usoclient_scan, parse_windows_update, reboot_pending, WindowsUpdateStep,
        WINDOWS_UPDATE,
    };
    use crate::options::RunOptions;
    use crate::platform::Platform;
    #[cfg(windows)]
    use crate::runner::{Runner, StepStatus};
    use crate::step::{Detection, OutdatedItem, Step};

    #[cfg(windows)]
    use serde_json::Value;

    /// Creates a stub PowerShell on a temp dir that responds to invocations
    /// based on their arguments. Returns the stub directory; the caller must
    /// restore `PATH` and remove it.
    fn stub_powershell(detect_output: &str, check_output: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-windows-update-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script = format!(
                "#!/bin/sh\n\
                 {{\n\
                   printf '%s' powershell\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 }} >> \"$FRESHEN_WINDOWS_UPDATE_LOG\"\n\
                 case \"$*\" in\n\
                   *ListAvailable*)\n\
                     printf {}; exit 0 ;;\n\
                   *Get-WindowsUpdate*)\n\
                     printf {}; exit 0 ;;\n\
                   *) exit 0 ;;\n\
                 esac\n",
                shell_quote(detect_output),
                shell_quote(check_output)
            );
            let stub = dir.join("pwsh");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let detect_file = dir.join("detect-output.txt");
            let check_file = dir.join("check-output.txt");
            std::fs::write(&detect_file, detect_output.replace('\n', "\r\n"))
                .unwrap_or_else(|error| panic!("write {}: {error}", detect_file.display()));
            std::fs::write(&check_file, check_output.replace('\n', "\r\n"))
                .unwrap_or_else(|error| panic!("write {}: {error}", check_file.display()));
            // `%~5` is the PowerShell script argument (`-NoProfile -ExecutionPolicy
            // Bypass -Command <script>`). It contains `|`, so it is only ever
            // read inside quotes, never echoed as `%*`: with several quoted
            // arguments `%*` flips the quote parity and cmd.exe runs the `|`.
            // Canned output is printed from a file with `type`.
            let script = "@echo off\r\n\
                 setlocal EnableDelayedExpansion\r\n\
                 set \"script=%~5\"\r\n\
                 if defined FRESHEN_WINDOWS_UPDATE_LOG >>\"%FRESHEN_WINDOWS_UPDATE_LOG%\" echo powershell !script!\r\n\
                 if not \"!script:ListAvailable=!\"==\"!script!\" (\r\n\
                   type \"%~dp0detect-output.txt\"\r\n\
                   exit /b 0\r\n\
                 )\r\n\
                 if not \"!script:Get-WindowsUpdate=!\"==\"!script!\" (\r\n\
                   type \"%~dp0check-output.txt\"\r\n\
                   exit /b 0\r\n\
                 )\r\n\
                 exit /b 0\r\n";
            let stub = dir.join("pwsh.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        dir
    }

    /// Creates a stub `usoclient` on a temp dir that prints `usoclient_output`
    /// for `usoclient StartScan` and records its invocations. Returns the stub
    /// directory; the caller must restore `PATH` and remove it.
    fn stub_usoclient(usoclient_output: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-usoclient-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script = format!(
                "#!/bin/sh\n\
                 {{\n\
                   printf '%s' usoclient\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 }} >> \"$FRESHEN_WINDOWS_UPDATE_LOG\"\n\
                 case \"$*\" in\n\
                   *StartScan*)\n\
                     printf {}; exit 0 ;;\n\
                   *) exit 0 ;;\n\
                 esac\n",
                shell_quote(usoclient_output)
            );
            let stub = dir.join("usoclient");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let output_file = dir.join("usoclient-output.txt");
            std::fs::write(&output_file, usoclient_output.replace('\n', "\r\n"))
                .unwrap_or_else(|error| panic!("write {}: {error}", output_file.display()));
            // `StartScan` has no `|`, so the argument is read with `%*` and
            // canned output is printed from a file with `type`.
            let script = "@echo off\r\n\
                 setlocal EnableDelayedExpansion\r\n\
                 set \"script=%~5\"\r\n\
                 if defined FRESHEN_WINDOWS_UPDATE_LOG >>\"%FRESHEN_WINDOWS_UPDATE_LOG%\" echo usoclient !script!\r\n\
                 if not \"!script:StartScan=!\"==\"!script!\" (\r\n\
                   type \"%~dp0usoclient-output.txt\"\r\n\
                   exit /b 0\r\n\
                 )\r\n\
                 exit /b 0\r\n";
            let stub = dir.join("usoclient.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        dir
    }

    /// Quotes `text` as a single shell word.
    #[cfg(unix)]
    fn shell_quote(text: &str) -> String {
        format!("'{}'", text.replace('\'', "'\\''"))
    }

    #[test]
    fn parses_json_array_of_updates() {
        let items = parse_windows_update(
            r#"[
  {
    "Title": "Security Update for Windows",
    "KB": "KB5012345",
    "Version": "10.0.19041.1234"
  },
  {
    "Title": "Cumulative Update for Windows",
    "KB": "KB5012346",
    "Version": "10.0.19041.1235"
  }
]"#,
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "Security Update for Windows");
        assert_eq!(items[0].current(), None);
        assert_eq!(items[0].latest(), Some("10.0.19041.1234"));
        assert_eq!(items[0].reason(), Some("KB KB5012345"));
        assert_eq!(items[1].name(), "Cumulative Update for Windows");
        assert_eq!(items[1].latest(), Some("10.0.19041.1235"));
        assert_eq!(items[1].reason(), Some("KB KB5012346"));
    }

    #[test]
    fn parses_single_object() {
        let items = parse_windows_update(
            r#"{
  "Title": "Security Update for Windows",
  "KB": "KB5012345",
  "Version": "10.0.19041.1234"
}"#,
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "Security Update for Windows");
        assert_eq!(items[0].latest(), Some("10.0.19041.1234"));
        assert_eq!(items[0].reason(), Some("KB KB5012345"));
    }

    #[test]
    fn empty_output_yields_no_items() {
        assert!(parse_windows_update("").is_empty());
        assert!(parse_windows_update("   \n").is_empty());
    }

    #[test]
    fn invalid_json_yields_no_items() {
        assert!(parse_windows_update("not json").is_empty());
        assert!(parse_windows_update("Get-WindowsUpdate : The term is not recognized").is_empty());
    }

    #[test]
    fn update_without_kb_or_version_is_still_reported() {
        let items = parse_windows_update(r#"[{"Title": "Feature Update to Windows 11"}]"#);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "Feature Update to Windows 11");
        assert_eq!(items[0].current(), None);
        assert_eq!(items[0].latest(), None);
        assert_eq!(items[0].reason(), None);
    }

    #[test]
    fn reboot_pending_is_true_when_any_update_requires_reboot() {
        assert!(reboot_pending(
            r#"[
  {"Title":"Security Update","KB":"KB1","RebootRequired":true},
  {"Title":"Cumulative Update","KB":"KB2","RebootRequired":false}
]"#
        ));
        assert!(reboot_pending(
            r#"{"Title":"Feature Update","RebootRequired":true}"#
        ));
    }

    #[test]
    fn reboot_pending_is_false_when_no_update_requires_reboot() {
        assert!(!reboot_pending(
            r#"[
  {"Title":"Security Update","KB":"KB1","RebootRequired":false},
  {"Title":"Cumulative Update","KB":"KB2"}
]"#
        ));
        assert!(!reboot_pending(
            r#"{"Title":"Feature Update","RebootRequired":false}"#
        ));
        assert!(!reboot_pending("[]"));
        assert!(!reboot_pending("{}"));
    }

    #[test]
    fn reboot_pending_is_false_for_invalid_output() {
        assert!(!reboot_pending("not json"));
        assert!(!reboot_pending(
            "Get-WindowsUpdate : The term is not recognized"
        ));
        assert!(!reboot_pending(""));
        assert!(!reboot_pending("   \n"));
    }

    #[test]
    fn check_returns_items_from_stub() {
        let dir = stub_powershell(
            "PSWindowsUpdate",
            r#"[{"Title":"Security Update","KB":"KB5012345","Version":"10.0.19041.1234"}]"#,
        );
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINDOWS_UPDATE_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", dir.join("calls.log"));

        let report = WindowsUpdateStep::new()
            .check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous_log) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous_log);
        } else {
            std::env::remove_var("FRESHEN_WINDOWS_UPDATE_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(report.pending(), 1);
        assert_eq!(report.items()[0].name(), "Security Update");
        assert_eq!(
            format_report_item(&report.items()[0]),
            "Security Update -> 10.0.19041.1234 (KB KB5012345)"
        );
    }

    /// `Get-WindowsUpdate` is queried exactly once per run: the check before
    /// the step fetches and caches the listing, and `run()` plus a second
    /// check (the runner's after-run diff) reuse the cache instead of
    /// scanning again. Covered on every OS through the stubbed PowerShell.
    #[test]
    fn get_windows_update_is_queried_once_per_run() {
        let dir = stub_powershell(
            "PSWindowsUpdate",
            r#"[{"Title":"Security Update","KB":"KB5012345","Version":"10.0.19041.1234"}]"#,
        );
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINDOWS_UPDATE_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", dir.join("calls.log"));

        let step = WindowsUpdateStep::new();
        step.check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));
        step.run_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("run should succeed: {error}"));
        step.check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("after-run check should succeed: {error}"));

        let log = std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default();
        let get_windows_update_calls = log
            .lines()
            .filter(|line| line.contains("Get-WindowsUpdate"))
            .count();
        assert_eq!(
            get_windows_update_calls, 1,
            "expected exactly one Get-WindowsUpdate call per run, log:\n{log}"
        );

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous_log) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous_log);
        } else {
            std::env::remove_var("FRESHEN_WINDOWS_UPDATE_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_returns_installed_when_module_is_available() {
        let dir = stub_powershell("PSWindowsUpdate", "[]");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINDOWS_UPDATE_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", dir.join("calls.log"));

        let detection = WindowsUpdateStep::new()
            .detect()
            .unwrap_or_else(|error| panic!("detect should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous_log) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous_log);
        } else {
            std::env::remove_var("FRESHEN_WINDOWS_UPDATE_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(detection, Detection::Installed);
    }

    #[test]
    fn detect_returns_not_installed_when_module_is_missing() {
        let dir = stub_powershell("", "[]");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINDOWS_UPDATE_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", dir.join("calls.log"));

        let detection = WindowsUpdateStep::new()
            .detect()
            .unwrap_or_else(|error| panic!("detect should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous_log) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous_log);
        } else {
            std::env::remove_var("FRESHEN_WINDOWS_UPDATE_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(detection, Detection::NotInstalled);
    }

    /// `run` reports the updates without failing when updates are pending.
    #[test]
    fn run_reports_updates_without_failing() {
        let dir = stub_powershell(
            "PSWindowsUpdate",
            r#"[{"Title":"Security Update","KB":"KB5012345","Version":"10.0.19041.1234"}]"#,
        );
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINDOWS_UPDATE_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", dir.join("calls.log"));

        let result = WindowsUpdateStep::new().run();

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous_log) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous_log);
        } else {
            std::env::remove_var("FRESHEN_WINDOWS_UPDATE_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            result.is_ok(),
            "run should succeed even with pending updates"
        );
    }

    #[test]
    fn check_filter_applies_include_exclude_lists() {
        let dir = stub_powershell(
            "PSWindowsUpdate",
            r#"[
  {"Title":"Update A","KB":"KB1","Version":"1.0"},
  {"Title":"Update B","KB":"KB2","Version":"2.0"}
]"#,
        );
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let options = RunOptions::new().with_filters(
            crate::policy::ItemFilters::new()
                .with_packages(crate::policy::NameFilter::new().with_exclude(["Update B"])),
        );
        let report = WindowsUpdateStep::new()
            .check_with_options(&options)
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(report.pending(), 1);
        assert_eq!(report.items()[0].name(), "Update A");
    }

    #[test]
    fn the_step_only_runs_on_windows() {
        let registry = super::register(crate::registry::Registry::new());

        assert_eq!(registry.validate(), Ok(()));
        let step = registry
            .steps()
            .find(|step| step.id() == WINDOWS_UPDATE)
            .unwrap_or_else(|| panic!("{WINDOWS_UPDATE} is not registered"));
        assert_eq!(step.platforms(), [Platform::Windows]);
    }

    #[test]
    fn the_step_is_not_part_of_a_run_on_other_platforms() {
        let registry = super::register(crate::registry::Registry::new());

        for platform in [Platform::Macos, Platform::Linux] {
            let ids: Vec<&str> = registry
                .ordered_for(platform)
                .unwrap_or_else(|error| panic!("registry order failed: {error}"))
                .iter()
                .map(|step| step.id())
                .collect();
            assert!(!ids.contains(&WINDOWS_UPDATE), "{platform}");
        }
    }

    /// Formats one item like the `Display` impl: `name -> latest (reason)`.
    fn format_report_item(item: &OutdatedItem) -> String {
        let mut s = item.name().to_owned();
        if let Some(latest) = item.latest() {
            s.push_str(&format!(" -> {latest}"));
        }
        if let Some(reason) = item.reason() {
            s.push_str(&format!(" ({reason})"));
        }
        s
    }

    #[test]
    fn parses_usoclient_scan_reports_pending_updates() {
        let items =
            parse_usoclient_scan("USO Client started scanning for updates.\nUpdates found: 2\n");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "Windows Update");
        assert_eq!(items[0].reason(), Some("updates pending"));
    }

    #[test]
    fn parses_usoclient_scan_reports_none_when_no_updates() {
        assert!(parse_usoclient_scan(
            "USO Client started scanning for updates.\nNo updates found.\n"
        )
        .is_empty());
    }

    #[test]
    fn parses_usoclient_scan_empty_output() {
        assert!(parse_usoclient_scan("").is_empty());
        assert!(parse_usoclient_scan("   \n").is_empty());
    }

    #[test]
    fn detect_falls_back_to_usocliter_when_pswindowsupdate_missing() {
        let dir = stub_usoclient("updates found: 2\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINDOWS_UPDATE_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", dir.join("calls.log"));

        let detection = WindowsUpdateStep::new()
            .detect()
            .unwrap_or_else(|error| panic!("detect should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous_log) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous_log);
        } else {
            std::env::remove_var("FRESHEN_WINDOWS_UPDATE_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(detection, Detection::Installed);
    }

    #[test]
    fn check_reports_updates_via_usocliter_when_pswindowsupdate_missing() {
        let dir = stub_usoclient("updates found: 2\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINDOWS_UPDATE_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", dir.join("calls.log"));

        let report = WindowsUpdateStep::new()
            .check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous_log) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous_log);
        } else {
            std::env::remove_var("FRESHEN_WINDOWS_UPDATE_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(report.pending(), 1);
        assert_eq!(report.items()[0].name(), "Windows Update");
        assert_eq!(report.items()[0].reason(), Some("updates pending"));
    }

    #[test]
    fn check_reports_none_via_usocliter_when_no_updates() {
        let dir = stub_usoclient("No updates found.\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let report = WindowsUpdateStep::new()
            .check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(report.pending(), 0);
    }

    /// `usoclient StartScan` is run once per run: the check before the step
    /// fetches and caches the listing, and `run` plus a second check reuse it
    /// instead of scanning again.
    #[test]
    fn usocliter_is_used_once_per_run() {
        let dir = stub_usoclient("updates found: 1\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINDOWS_UPDATE_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", dir.join("calls.log"));

        let step = WindowsUpdateStep::new();
        step.check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));
        step.run_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("run should succeed: {error}"));
        step.check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("after-run check should succeed: {error}"));

        let log = std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default();
        let usoclient_calls = log
            .lines()
            .filter(|line| line.contains("usoclient"))
            .count();
        assert_eq!(
            usoclient_calls, 1,
            "expected exactly one usoclient call per run, log:\n{log}"
        );

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous_log) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous_log);
        } else {
            std::env::remove_var("FRESHEN_WINDOWS_UPDATE_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs the Windows Update step against a stubbed PowerShell and returns
    /// the run summary, restoring PATH and removing the stub directory
    /// afterwards.
    #[cfg(windows)]
    fn run_with_stub(check_output: &str) -> crate::runner::RunSummary {
        let dir = stub_powershell("PSWindowsUpdate", check_output);
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let step = WindowsUpdateStep::new();
        let summary = Runner::with_options([&step as &dyn Step], RunOptions::new()).run();

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        summary
    }

    /// A pending reboot reported by Windows Update is recorded for the step
    /// and shows up in the human summary, the JSON report and the Markdown
    /// report.
    #[cfg(windows)]
    #[test]
    fn a_pending_reboot_is_recorded_and_shown_in_the_summary_and_reports() {
        let summary = run_with_stub(
            r#"[{"Title":"Security Update","KB":"KB5012345","Version":"10.0.19041.1234","RebootRequired":true}]"#,
        );
        let result = &summary.steps()[0];
        assert_eq!(result.status(), StepStatus::Ok);
        assert!(result.restart_needed());

        let json = summary.to_json();
        let value: Value = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("invalid JSON report: {error}: {json}"));
        assert_eq!(value["steps"][0]["restart_needed"], true);

        let markdown = summary.to_markdown();
        assert!(markdown.contains("## Restart needed"));
        assert!(markdown.contains("A restart is required after this step."));
    }

    /// When no update requires a reboot, `restart_needed` stays false and no
    /// restart section is emitted.
    #[cfg(windows)]
    #[test]
    fn no_pending_reboot_means_restart_is_not_recorded() {
        let summary = run_with_stub(
            r#"[{"Title":"Security Update","KB":"KB5012345","Version":"10.0.19041.1234"}]"#,
        );
        let result = &summary.steps()[0];
        assert_eq!(result.status(), StepStatus::Ok);
        assert!(!result.restart_needed());

        let json = summary.to_json();
        let value: Value = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("invalid JSON report: {error}: {json}"));
        assert_eq!(value["steps"][0]["restart_needed"], false);

        let markdown = summary.to_markdown();
        assert!(!markdown.contains("## Restart needed"));
    }
}
