//! The Windows Update step: reports available Windows Updates through the
//! PSWindowsUpdate PowerShell module when it is installed.
//!
//! Detection checks whether PowerShell (`pwsh` preferred, then `powershell`)
//! exists and the PSWindowsUpdate module is available. `check()` runs
//! `Get-WindowsUpdate` through PowerShell and parses the JSON output into
//! outdated items. Neither `detect` nor `check` changes anything, so the step
//! is safe in `--check` mode. `run()` reports the same items without
//! installing them — this is a reporting-only step. The step is only available
//! on Windows.

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The Windows Update reporting step.
pub const WINDOWS_UPDATE: StepId = "windows-update";

/// Windows Update only exists on Windows.
const WINDOWS_UPDATE_PLATFORMS: [Platform; 1] = [Platform::Windows];

/// Registers the Windows Update step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(WindowsUpdateStep)
}

struct WindowsUpdateStep;

impl WindowsUpdateStep {
    /// Reports the Windows Updates `Get-WindowsUpdate` lists as outdated,
    /// honouring the package include/exclude list.
    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        let Some(powershell) = powershell_command() else {
            return Ok(CheckReport::up_to_date());
        };
        let output = CommandSpec::new(powershell)
            .args([
                "-NoProfile",
                "-Command",
                "Import-Module PSWindowsUpdate; Get-WindowsUpdate | Select-Object Title, KB, Version | ConvertTo-Json",
            ])
            .neutral_locale()
            .run()
            .map_err(|error| StepError::check_from_process(WINDOWS_UPDATE, &error))?;
        let items = parse_windows_update(output.stdout());
        let filter = options.filters().packages();
        let report = CheckReport::from_items(
            items
                .into_iter()
                .filter(|item| filter.includes(item.name())),
        );
        Ok(report)
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
        let Some(powershell) = powershell_command() else {
            return Ok(Detection::NotInstalled);
        };
        let output = CommandSpec::new(powershell)
            .args([
                "-NoProfile",
                "-Command",
                "Get-Module -ListAvailable PSWindowsUpdate | Select-Object -ExpandProperty Name",
            ])
            .neutral_locale()
            .run();
        match output {
            Ok(output) if !output.stdout().trim().is_empty() => Ok(Detection::Installed),
            _ => Ok(Detection::NotInstalled),
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
        let report = self.check()?;
        if report.is_up_to_date() {
            human!("windows-update: no Windows Updates available");
        } else {
            human!("windows-update: {} update(s) available", report.pending());
            for item in report.items() {
                human!("  - {item}");
            }
        }
        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &WINDOWS_UPDATE_PLATFORMS
    }
}

/// Returns the PowerShell executable to use: `pwsh` (PowerShell 7) when it
/// exists, otherwise `powershell` (Windows PowerShell 5.1). Returns `None`
/// when neither is on `PATH`.
fn powershell_command() -> Option<&'static str> {
    if crate::executable::command_exists("pwsh") {
        return Some("pwsh");
    }
    if crate::executable::command_exists("powershell") {
        return Some("powershell");
    }
    None
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

#[cfg(test)]
mod tests {
    use super::{parse_windows_update, WindowsUpdateStep, WINDOWS_UPDATE};
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::step::{Detection, OutdatedItem, Step};

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
            let script = "@echo off\r\n\
                 >>\"%FRESHEN_WINDOWS_UPDATE_LOG%\" echo powershell %*\r\n\
                 if /I \"%~1\"==\"-NoProfile\" (\r\n\
                   if /I \"%~3\"==\"ListAvailable\" (\r\n\
                     type \"%~dp0detect-output.txt\"\r\n\
                   ) else if /I \"%~3\"==\"Get-WindowsUpdate\" (\r\n\
                     type \"%~dp0check-output.txt\"\r\n\
                   )\r\n\
                 )\r\n\
                 exit /b 0\r\n";
            let stub = dir.join("pwsh.cmd");
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

        let report = WindowsUpdateStep
            .check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous);
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

        let detection = WindowsUpdateStep
            .detect()
            .unwrap_or_else(|error| panic!("detect should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous);
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

        let detection = WindowsUpdateStep
            .detect()
            .unwrap_or_else(|error| panic!("detect should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous);
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

        let result = WindowsUpdateStep.run();

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINDOWS_UPDATE_LOG", previous);
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
        let report = WindowsUpdateStep
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
}
