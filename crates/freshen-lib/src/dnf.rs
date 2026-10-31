//! The dnf step: `dnf upgrade -y`, skipped when dnf is absent.
//!
//! The step runs `dnf upgrade -y` whenever `dnf` is available. It is skipped
//! entirely when dnf is absent, so it is safe to register on every machine;
//! on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The dnf package manager step.
pub const DNF: StepId = "dnf";

/// dnf only runs on Linux.
const DNF_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the dnf step, after the apt step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(DnfStep)
}

struct DnfStep;

impl DnfStep {
    /// Runs `dnf upgrade -y`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("dnf upgrade -y");
        if let Err(error) = CommandSpec::new("dnf")
            .args(["upgrade", "-y"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(DNF, &error));
        }
    }
}

impl Step for DnfStep {
    fn id(&self) -> StepId {
        DNF
    }

    fn display_name(&self) -> &str {
        "dnf"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_dnf())
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
        &DNF_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `dnf` binary is discoverable on `PATH`.
fn detect_dnf() -> Detection {
    if command_exists("dnf") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}
