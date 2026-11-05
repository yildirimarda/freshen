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
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Chocolatey package manager step.
pub const CHOCOLATEY: StepId = "chocolatey";

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

    /// Placeholder read-only check: Chocolatey is treated as up to date until
    /// the `choco outdated` check is implemented.
    fn check_with_options(&self, _options: &RunOptions) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
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

#[cfg(test)]
mod tests {
    use super::{ChocolateyStep, CHOCOLATEY};
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
}
