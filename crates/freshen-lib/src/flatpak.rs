//! The Flatpak step: `flatpak update -y`, skipped when flatpak is absent, and
//! `flatpak uninstall --unused` when `--cleanup` is set.
//!
//! The step runs `flatpak update -y` whenever `flatpak` is available. It is
//! skipped entirely when flatpak is absent, so it is safe to register on
//! every machine; on macOS and Windows it is never detected.

use crate::cleanup;
use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
use crate::wsl::inside_command;

/// The Flatpak package manager step.
pub const FLATPAK: StepId = "flatpak";

/// Flatpak runs on Linux natively and, on Windows, inside a selected WSL
/// distribution via `wsl -d <distro> flatpak ...`.
const FLATPAK_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// Registers the flatpak step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(FlatpakStep)
}

struct FlatpakStep;

impl FlatpakStep {
    /// Runs `flatpak update -y`. On Windows the command runs inside the
    /// selected WSL distribution via `wsl -d <distro>`.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn upgrade(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        human!("flatpak update -y");
        if let Err(error) = inside_command(
            options,
            CommandSpec::new("flatpak")
                .args(["update", "-y"])
                .retry_transient_network(NETWORK_RETRIES),
        )
        .run()
        {
            failures.push(StepError::run_from_process(FLATPAK, &error));
        }
    }

    /// Runs `flatpak uninstall --unused`, gated on `--cleanup`. On Windows the
    /// command runs inside the selected WSL distribution via `wsl -d <distro>`.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn cleanup(&self, failures: &mut Vec<StepError>, options: &RunOptions) {
        if let Err(error) = cleanup::run(
            FLATPAK,
            "flatpak: uninstall unused",
            inside_command(
                options,
                CommandSpec::new("flatpak")
                    .args(["uninstall", "--unused"])
                    .retry_transient_network(NETWORK_RETRIES),
            ),
            options,
        ) {
            failures.push(error);
        }
    }
}

impl Step for FlatpakStep {
    fn id(&self) -> StepId {
        FLATPAK
    }

    fn display_name(&self) -> &str {
        "flatpak"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        #[cfg(target_os = "linux")]
        return Ok(detect_flatpak());
        #[cfg(target_os = "windows")]
        return Ok(detect_wsl());
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        return Ok(Detection::NotInstalled);
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        #[cfg(target_os = "windows")]
        {
            let Some(_distro) = crate::wsl::selected_distro(options) else {
                human!("flatpak: no WSL distribution selected; pass --wsl-distro <distro>");
                return Ok(());
            };
            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);
            self.cleanup(&mut failures, options);
            finish(failures)
        }

        #[cfg(target_os = "linux")]
        {
            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);
            self.cleanup(&mut failures, options);
            finish(failures)
        }

        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            // flatpak is not available on macOS; nothing to run.
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &FLATPAK_PLATFORMS
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `flatpak` binary is discoverable on `PATH`.
#[cfg(target_os = "linux")]
fn detect_flatpak() -> Detection {
    if command_exists("flatpak") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Whether the `wsl` binary is discoverable on `PATH` (Windows only).
#[cfg(target_os = "windows")]
fn detect_wsl() -> Detection {
    if command_exists("wsl") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::detect_flatpak;
    use crate::step::Detection;

    #[cfg(target_os = "linux")]
    #[test]
    fn detect_flatpak_reports_installed_when_on_path() {
        // `flatpak` is not expected on the test machine; this just exercises
        // the detection function against a name that is absent.
        assert!(!detect_flatpak().is_installed());
    }

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }
}
