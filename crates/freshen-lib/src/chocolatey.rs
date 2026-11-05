//! The Chocolatey step: detection, a read-only check and the upgrade for the
//! Chocolatey package manager on Windows.
//!
//! Detection runs `choco --version`. On Windows `choco` is resolved through
//! `PATHEXT`, so `choco.exe` is found even when only the `.exe` is on
//! `PATH`. Chocolatey modifies the system, so the step declares that it needs
//! administrator rights ([`Step::needs_administrator`]); the runner skips it
//! with [`SkipReason::NeedsAdministrator`] when the process is not elevated.
//! The step is only available on Windows.
//!
//! `check()` runs `choco outdated --limit-output` and reports outdated
//! packages. Pinned packages are held back instead of reported as outdated.
//!
//! `run()` first upgrades Chocolatey itself with `choco upgrade chocolatey`,
//! then upgrades every outdated package individually with `choco upgrade
//! <name>`, streaming output and recording per-package failures. Pinned
//! packages are skipped and recorded as held back.

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::skipped_items;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The Chocolatey package manager step.
pub const CHOCOLATEY: StepId = "chocolatey";

/// The reason shown for a Chocolatey package the user pinned with `choco pin`.
const PINNED_REASON: &str = "held back (pinned)";

/// Chocolatey only runs on Windows.
const CHOCOLATEY_PLATFORMS: [Platform; 1] = [Platform::Windows];

/// Registers the Chocolatey step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ChocolateyStep)
}

struct ChocolateyStep;

impl ChocolateyStep {
    /// Reports whether Chocolatey is installed by running `choco --version`.
    ///
    /// The program name is resolved through `PATHEXT` on Windows, so
    /// `choco.exe` is found even when only the `.exe` is on `PATH`.
    fn detect_choco() -> Result<Detection, StepError> {
        if CommandSpec::new("choco").args(["--version"]).run().is_ok() {
            Ok(Detection::Installed)
        } else {
            Ok(Detection::NotInstalled)
        }
    }

    /// Reports the packages `choco outdated --limit-output` lists as
    /// outdated. Pinned packages are kept in the report with a "held back
    /// (pinned)" reason instead of being reported as plainly outdated.
    fn check_with_options(&self, _options: &RunOptions) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("choco")
            .args(["outdated", "--limit-output"])
            .neutral_locale()
            .run()
            .map_err(|error| StepError::check_from_process(CHOCOLATEY, &error))?;
        let packages = parse_chocolatey_outdated(output.stdout());
        let report = CheckReport::from_items(packages.into_iter().map(|package| {
            let mut item = OutdatedItem::new(&package.name)
                .with_current(&package.current)
                .with_latest(&package.available);
            if package.pinned {
                item = item.with_reason(PINNED_REASON);
            }
            item
        }));
        Ok(report)
    }
}

impl Step for ChocolateyStep {
    fn id(&self) -> StepId {
        CHOCOLATEY
    }

    fn display_name(&self) -> &str {
        "Chocolatey"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        ChocolateyStep::detect_choco()
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        ChocolateyStep::check_with_options(self, options)
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();

        // 1. Upgrade Chocolatey itself first.
        human!("choco upgrade chocolatey");
        if let Err(error) = CommandSpec::new("choco")
            .args(["upgrade", "chocolatey"])
            .neutral_locale()
            .run_streaming(|_, line| stream_line(line))
        {
            failures.push(StepError::run_from_process(CHOCOLATEY, &error));
        }

        // 2. List the outdated packages so each can be upgraded individually,
        //    which lets us record per-package failures instead of a single
        //    opaque failure from `choco upgrade all`.
        let output = match CommandSpec::new("choco")
            .args(["outdated", "--limit-output"])
            .neutral_locale()
            .run()
        {
            Ok(output) => output,
            Err(error) => {
                failures.push(StepError::run_from_process(CHOCOLATEY, &error));
                return finish(failures, &[]);
            }
        };
        let packages = parse_chocolatey_outdated(output.stdout());

        // 3. Upgrade every non-pinned package individually.
        let filter = options.filters().packages();
        let mut failed_packages = Vec::new();
        for package in &packages {
            if package.pinned {
                human!("{}: held back (pinned)", package.name);
                skipped_items::record(package.name.clone());
                continue;
            }
            if !filter.includes(&package.name) {
                human!("{}: excluded", package.name);
                skipped_items::record(package.name.clone());
                continue;
            }
            human!("choco upgrade {}", package.name);
            if let Err(error) = CommandSpec::new("choco")
                .args(["upgrade", &package.name])
                .neutral_locale()
                .run_streaming(|_, line| stream_line(line))
            {
                failed_packages.push(package.name.clone());
                failures.push(StepError::run_from_process(CHOCOLATEY, &error));
            }
        }

        finish(failures, &failed_packages)
    }

    fn platforms(&self) -> &'static [Platform] {
        &CHOCOLATEY_PLATFORMS
    }

    /// Chocolatey installs packages system-wide, so it requires administrator
    /// rights.
    fn needs_administrator(&self) -> bool {
        true
    }
}

/// Forwards a non-empty line of command output to the human-readable stream.
fn stream_line(line: &str) {
    if !line.is_empty() {
        human!("  {line}");
    }
}

/// Combines the recorded failures into a single step error. When packages
/// failed, the error lists them by name so the summary shows per-package
/// failures. Otherwise the first process error is returned.
fn finish(failures: Vec<StepError>, failed_packages: &[String]) -> Result<(), StepError> {
    if failures.is_empty() {
        return Ok(());
    }
    if !failed_packages.is_empty() {
        let message = format!(
            "failed to upgrade {} package(s): {}",
            failed_packages.len(),
            failed_packages.join(", ")
        );
        return Err(StepError::run(CHOCOLATEY, message));
    }
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// A Chocolatey package as reported by `choco outdated --limit-output`.
///
/// Each line is `name|current|available|pinned`, where the last field is
/// either empty or `pinned`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChocolateyPackage {
    name: String,
    current: String,
    available: String,
    pinned: bool,
}

/// Parses the output of `choco outdated --limit-output` into packages.
///
/// Blank lines and lines without three pipe-separated fields are ignored. A
/// package whose fourth field is `pinned` is reported as held back instead of
/// plainly outdated.
fn parse_chocolatey_outdated(output: &str) -> Vec<ChocolateyPackage> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut fields = line.split('|');
            let name = fields.next()?.trim();
            let current = fields.next()?.trim();
            let available = fields.next()?.trim();
            let pinned = fields
                .next()
                .map(|field| field.trim() == "pinned")
                .unwrap_or(false);
            if name.is_empty() || current.is_empty() || available.is_empty() {
                return None;
            }
            Some(ChocolateyPackage {
                name: name.to_owned(),
                current: current.to_owned(),
                available: available.to_owned(),
                pinned,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_chocolatey_outdated, ChocolateyStep, CHOCOLATEY};
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::step::{Detection, Step};

    /// Creates a stub `choco` on a temp dir that exits 0 for `choco
    /// --version`. Returns the stub directory; the caller restores `PATH` and
    /// removes it.
    fn stub_choco() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-chocolatey-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script = "#!/bin/sh\nexit 0\n";
            let stub = dir.join("choco");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            // A `.cmd` stub never echoes `%*`; it only needs to signal that
            // `choco` exists by exiting 0.
            let script = "@echo off\r\nexit /b 0\r\n";
            let stub = dir.join("choco.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        dir
    }

    #[test]
    fn detect_reports_installed_when_choco_is_on_path() {
        let dir = stub_choco();
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let detection = ChocolateyStep
            .detect()
            .unwrap_or_else(|error| panic!("detect should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(detection, Detection::Installed);
    }

    #[test]
    fn detect_reports_not_installed_when_choco_is_absent() {
        let dir = std::env::temp_dir().join(format!(
            "freshen-chocolatey-absent-{pid}",
            pid = std::process::id(),
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });

        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let detection = ChocolateyStep
            .detect()
            .unwrap_or_else(|error| panic!("detect should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(detection, Detection::NotInstalled);
    }

    #[test]
    fn chocolatey_needs_administrator_rights() {
        assert!(ChocolateyStep.needs_administrator());
    }

    #[test]
    fn chocolatey_is_windows_only() {
        assert_eq!(ChocolateyStep.platforms(), &[Platform::Windows]);
    }

    #[test]
    fn chocolatey_step_id_and_name() {
        assert_eq!(ChocolateyStep.id(), CHOCOLATEY);
        assert_eq!(ChocolateyStep.display_name(), "Chocolatey");
    }

    /// Creates a stub `choco` that prints `output` for `choco outdated
    /// --limit-output` and exits 0 for `choco --version`. Returns the stub
    /// directory; the caller restores `PATH` and removes it.
    fn stub_choco_with_outdated_output(output: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-chocolatey-check-{pid}-{id}",
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
                 case \"$1\" in\n\
                   --version) exit 0 ;;\n\
                   outdated)\n\
                     if [ \"$2\" = --limit-output ]; then\n\
                       printf '%s' {}\n\
                     fi\n\
                     exit 0\n\
                     ;;\n                   *) exit 0 ;;\n                 esac\n",
                shell_quote(output)
            );
            let stub = dir.join("choco");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            // The output is printed from a file: a multi-line `echo` would turn
            // every line after the first into a separate `cmd` command.
            let output_file = dir.join("outdated.txt");
            let table = output.replace('\n', "\r\n");
            std::fs::write(&output_file, table)
                .unwrap_or_else(|error| panic!("write {}: {error}", output_file.display()));
            let script = "@echo off\r\n\
                 setlocal EnableDelayedExpansion\r\n\
                 >>\"%FRESHEN_STUB_LOG%\" echo choco %*\r\n\
                 if \"%~1\"==\"--version\" exit /b 0\r\n\
                 if \"%~1 %~2\"==\"outdated --limit-output\" type \"%~dp0outdated.txt\"\r\n\
                 exit /b 0\r\n";
            let stub = dir.join("choco.cmd");
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
    fn parse_chocolatey_outdated_parses_name_current_available_and_pinned() {
        let packages = parse_chocolatey_outdated(
            "git|2.45.2|2.46.0|\n\
             python|3.11.0|3.11.1|pinned\n\
             node|20.0.0|20.1.0|\n",
        );

        assert_eq!(packages.len(), 3);
        assert_eq!(packages[0].name, "git");
        assert_eq!(packages[0].current, "2.45.2");
        assert_eq!(packages[0].available, "2.46.0");
        assert!(!packages[0].pinned);
        assert_eq!(packages[1].name, "python");
        assert_eq!(packages[1].current, "3.11.0");
        assert_eq!(packages[1].available, "3.11.1");
        assert!(packages[1].pinned);
        assert_eq!(packages[2].name, "node");
        assert_eq!(packages[2].current, "20.0.0");
        assert_eq!(packages[2].available, "20.1.0");
        assert!(!packages[2].pinned);
    }

    #[test]
    fn parse_chocolatey_outdated_ignores_blank_and_short_lines() {
        let packages = parse_chocolatey_outdated("\ngit|2.45.2|2.46.0|\n\nshort\n");
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "git");
    }

    #[test]
    fn check_reports_outdated_packages_and_holds_back_pinned_ones() {
        let dir =
            stub_choco_with_outdated_output("git|2.45.2|2.46.0|\npython|3.11.0|3.11.1|pinned\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let report = ChocolateyStep
            .check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(report.pending(), 2);
        let git = report
            .items()
            .iter()
            .find(|item| item.name() == "git")
            .unwrap_or_else(|| panic!("git should be reported"));
        assert_eq!(git.current(), Some("2.45.2"));
        assert_eq!(git.latest(), Some("2.46.0"));
        assert_eq!(git.reason(), None);
        let python = report
            .items()
            .iter()
            .find(|item| item.name() == "python")
            .unwrap_or_else(|| panic!("python should be reported"));
        assert_eq!(python.current(), Some("3.11.0"));
        assert_eq!(python.latest(), Some("3.11.1"));
        assert_eq!(python.reason(), Some("held back (pinned)"));
    }

    #[test]
    fn check_reports_nothing_when_no_outdated_packages() {
        let dir = stub_choco_with_outdated_output("");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let report = ChocolateyStep
            .check_with_options(&RunOptions::new())
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(report.is_up_to_date());
    }

    /// Creates a stub `choco` for run tests that exits 0 for `--version` and
    /// `upgrade chocolatey`, prints `outdated_output` for `outdated
    /// --limit-output`, and optionally fails the upgrade of one package (via
    /// `FRESHEN_CHOCO_FAIL_PACKAGE`) or of Chocolatey itself (via
    /// `FRESHEN_CHOCO_FAIL_CHOCOLATEY=1`).
    fn stub_choco_for_run(outdated_output: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-chocolatey-run-{pid}-{id}",
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
                 case \"$1\" in\n\
                   --version) exit 0 ;;\n\
                   upgrade)\n\
                     if [ \"$2\" = chocolatey ]; then\n\
                       if [ \"${{FRESHEN_CHOCO_FAIL_CHOCOLATEY:-0}}\" = 1 ]; then\n\
                         exit 1\n\
                       fi\n\
                       exit 0\n\
                     fi\n\
                     if [ \"$2\" = ${{FRESHEN_CHOCO_FAIL_PACKAGE:-__none__}} ]; then\n\
                       exit 1\n\
                     fi\n\
                     exit 0\n\
                     ;;\n\
                   outdated)\n\
                     if [ \"$2\" = --limit-output ]; then\n\
                       printf '%s' {}\n\
                     fi\n\
                     exit 0\n\
                     ;;\n                   *) exit 0 ;;\n                 esac\n",
                shell_quote(outdated_output)
            );
            let stub = dir.join("choco");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let output_file = dir.join("outdated.txt");
            let table = outdated_output.replace('\n', "\r\n");
            std::fs::write(&output_file, &table)
                .unwrap_or_else(|error| panic!("write {}: {error}", output_file.display()));
            let script = "@echo off\r\n\
                 setlocal EnableDelayedExpansion\r\n\
                 if \"%~1\"==\"--version\" exit /b 0\r\n\
                 if \"%~1 %~2\"==\"upgrade chocolatey\" (\r\n\
                   set \"code=%FRESHEN_CHOCO_FAIL_CHOCOLATEY%\"\r\n\
                   if not defined code set code=0\r\n\
                   if \"!code!\"==\"1\" exit /b 1\r\n\
                   exit /b 0\r\n\
                 )\r\n\
                 if \"%~1\"==\"upgrade\" (\r\n\
                   set \"code=%FRESHEN_CHOCO_FAIL_PACKAGE%\"\r\n\
                   if not defined code set code=0\r\n\
                   if \"!code!\"==\"%~2\" exit /b 1\r\n\
                   exit /b 0\r\n\
                 )\r\n\
                 if \"%~1 %~2\"==\"outdated --limit-output\" type \"%~dp0outdated.txt\"\r\n\
                 exit /b 0\r\n";
            let stub = dir.join("choco.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        dir
    }

    #[test]
    fn run_upgrades_chocolatey_itself_first_then_each_package() {
        let dir = stub_choco_for_run("git|2.45.2|2.46.0|\npython|3.11.0|3.11.1|\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let result = ChocolateyStep.run_with_options(&RunOptions::new());

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_ok(), "run should succeed: {result:?}");
    }

    #[test]
    fn run_reports_failed_when_a_package_upgrade_fails() {
        let dir = stub_choco_for_run("git|2.45.2|2.46.0|\npython|3.11.0|3.11.1|\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_CHOCO_FAIL_PACKAGE", "python");

        let result = ChocolateyStep.run_with_options(&RunOptions::new());

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        std::env::remove_var("FRESHEN_CHOCO_FAIL_PACKAGE");
        let _ = std::fs::remove_dir_all(&dir);

        let error = match result {
            Err(error) => error,
            Ok(()) => panic!("run should have failed, got Ok"),
        };
        assert_eq!(error.step(), CHOCOLATEY);
        assert_eq!(error.operation(), crate::step::Operation::Run);
        assert!(
            error.message().contains("python"),
            "error message should name python: {}",
            error.message()
        );
    }

    #[test]
    fn run_skips_pinned_packages() {
        let dir = stub_choco_for_run("git|2.45.2|2.46.0|\npython|3.11.0|3.11.1|pinned\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);
        // If the pinned package were upgraded, this would fail, so a successful
        // run proves pinned packages are skipped.
        std::env::set_var("FRESHEN_CHOCO_FAIL_PACKAGE", "python");

        let result = ChocolateyStep.run_with_options(&RunOptions::new());

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        std::env::remove_var("FRESHEN_CHOCO_FAIL_PACKAGE");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            result.is_ok(),
            "pinned package should be skipped, not upgraded"
        );
    }

    #[test]
    fn run_reports_nothing_when_no_outdated_packages() {
        let dir = stub_choco_for_run("");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let result = ChocolateyStep.run_with_options(&RunOptions::new());

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_ok(), "run should succeed: {result:?}");
    }

    #[test]
    fn run_reports_failed_when_chocolatey_itself_fails() {
        let dir = stub_choco_for_run("git|2.45.2|2.46.0|\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_CHOCO_FAIL_CHOCOLATEY", "1");

        let result = ChocolateyStep.run_with_options(&RunOptions::new());

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        std::env::remove_var("FRESHEN_CHOCO_FAIL_CHOCOLATEY");
        let _ = std::fs::remove_dir_all(&dir);

        let error = match result {
            Err(error) => error,
            Ok(()) => panic!("run should have failed, got Ok"),
        };
        assert_eq!(error.step(), CHOCOLATEY);
        assert_eq!(error.operation(), crate::step::Operation::Run);
    }
}
