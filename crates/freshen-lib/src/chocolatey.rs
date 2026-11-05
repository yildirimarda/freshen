//! The Chocolatey step: detection for the Chocolatey package manager on
//! Windows.
//!
//! Detection runs `choco --version`. On Windows `choco` is resolved through
//! `PATHEXT`, so `choco.exe` is found even when only the executable is on
//! `PATH`. Chocolatey modifies the system, so the step declares that it needs
//! administrator rights ([`Step::needs_administrator`]); the runner skips it
//! with [`SkipReason::NeedsAdministrator`] when the process is not elevated.
//! The step is only available on Windows.
//!
//! The read-only check and the upgrade itself are intentionally stubs here:
//! they are added by later plan items.

use crate::options::RunOptions;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
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

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        // The Chocolatey upgrade is implemented by a later plan item.
        Ok(())
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
}
