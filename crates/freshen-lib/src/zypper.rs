//! The zypper step: `zypper refresh` followed by `zypper update -y`, skipped
//! when zypper is absent.
//!
//! The step runs `zypper refresh` and then `zypper update -y` whenever `zypper`
//! is available, collecting the first failure so both commands are attempted.
//! It is skipped entirely when zypper is absent, so it is safe to register on
//! every machine; on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The zypper package manager step.
pub const ZYPPER: StepId = "zypper";

/// zypper only runs on Linux.
const ZYPPER_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the zypper step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ZypperStep)
}

struct ZypperStep;

impl ZypperStep {
    /// Runs `zypper refresh` and then `zypper update -y`, collecting the
    /// first failure so both commands are attempted even when refresh fails.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("zypper refresh");
        if let Err(error) = CommandSpec::new("zypper")
            .args(["refresh"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ZYPPER, &error));
        }

        human!("zypper update -y");
        if let Err(error) = CommandSpec::new("zypper")
            .args(["update", "-y"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ZYPPER, &error));
        }
    }
}

impl Step for ZypperStep {
    fn id(&self) -> StepId {
        ZYPPER
    }

    fn display_name(&self) -> &str {
        "zypper"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_zypper())
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
        &ZYPPER_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `zypper` binary is discoverable on `PATH`.
fn detect_zypper() -> Detection {
    if command_exists("zypper") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}
