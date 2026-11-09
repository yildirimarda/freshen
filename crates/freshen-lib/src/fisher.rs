//! The Fisher step: `fisher update` for the Fish shell plugin manager,
//! skipped when `fisher` is absent.
//!
//! The step runs `fisher update`, which updates every installed plugin in one
//! command. It is skipped entirely when `fisher` is absent, so it is safe to
//! register on every machine. Fisher manages plugins in the user's Fish
//! configuration, so the step must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Fisher step.
pub const FISHER: StepId = "fisher";

/// Fisher runs on every platform freshen supports, wherever Fish is available.
const FISHER_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the Fisher step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(FisherStep)
}

struct FisherStep;

impl FisherStep {
    /// Updates every installed plugin with `fisher update`, collecting the
    /// failure so the step reports it instead of aborting.
    fn update_all(&self, failures: &mut Vec<StepError>) {
        human!("fisher update");
        if let Err(error) = CommandSpec::new("fisher")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(FISHER, &error));
        }
    }
}

impl Step for FisherStep {
    fn id(&self) -> StepId {
        FISHER
    }

    fn display_name(&self) -> &str {
        "Fisher"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_fisher())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // `fisher` does not expose a read-only "which plugins are outdated"
        // query, so there is no reliable read-only check. Report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_all(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &FISHER_PLATFORMS
    }

    fn is_user_level(&self) -> bool {
        true
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `fisher` binary is discoverable on `PATH`.
fn detect_fisher() -> Detection {
    if crate::executable::command_exists("fisher") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }
}
