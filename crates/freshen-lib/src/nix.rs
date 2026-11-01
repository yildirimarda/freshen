//! The Nix channel step: `nix-channel --update`, skipped when nix is absent.
//!
//! The step runs `nix channel --update` whenever `nix` is available. It is
//! skipped entirely when nix is absent, so it is safe to register on every
//! machine; on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Nix package manager step.
pub const NIX: StepId = "nix";

/// Nix only runs on Linux.
const NIX_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the nix step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(NixStep)
}

struct NixStep;

impl NixStep {
    /// Runs `nix channel --update`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("nix channel --update");
        if let Err(error) = CommandSpec::new("nix")
            .args(["channel", "--update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(NIX, &error));
        }
    }
}

impl Step for NixStep {
    fn id(&self) -> StepId {
        NIX
    }

    fn display_name(&self) -> &str {
        "nix"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_nix())
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
        &NIX_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `nix` binary is discoverable on `PATH`.
fn detect_nix() -> Detection {
    if command_exists("nix") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    use super::detect_nix;
    use crate::step::Detection;

    #[test]
    fn detect_nix_reports_not_installed_when_absent_from_path() {
        // `nix` is not expected on the test machine; this just exercises the
        // detection function against a name that is absent.
        assert!(!detect_nix().is_installed());
    }

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }
}
