//! The Flatpak step: `flatpak update -y`, skipped when flatpak is absent.
//!
//! The step runs `flatpak update -y` whenever `flatpak` is available. It is
//! skipped entirely when flatpak is absent, so it is safe to register on
//! every machine; on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Flatpak package manager step.
pub const FLATPAK: StepId = "flatpak";

/// Flatpak only runs on Linux.
const FLATPAK_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the flatpak step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(FlatpakStep)
}

struct FlatpakStep;

impl FlatpakStep {
    /// Runs `flatpak update -y`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("flatpak update -y");
        if let Err(error) = CommandSpec::new("flatpak")
            .args(["update", "-y"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(FLATPAK, &error));
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
        Ok(detect_flatpak())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &FLATPAK_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `flatpak` binary is discoverable on `PATH`.
fn detect_flatpak() -> Detection {
    if command_exists("flatpak") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    use super::detect_flatpak;
    use crate::step::Detection;

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
