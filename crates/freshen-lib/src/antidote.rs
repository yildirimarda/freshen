//! The antidote step: `antidote update`, skipped when `antidote` is absent.
//!
//! The step runs `antidote update` whenever `antidote` is available. It is
//! skipped entirely when `antidote` is absent, so it is safe to register on
//! every machine. antidote manages user-level shell plugins, so it must never
//! run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The antidote step.
pub const ANTIDOTE: StepId = "antidote";

/// antidote runs on every platform freshen supports.
const ANTIDOTE_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the antidote step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(AntidoteStep)
}

struct AntidoteStep;

impl AntidoteStep {
    /// Runs `antidote update`.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("antidote update");
        if let Err(error) = CommandSpec::new("antidote")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ANTIDOTE, &error));
        }
    }
}

impl Step for AntidoteStep {
    fn id(&self) -> StepId {
        ANTIDOTE
    }

    fn display_name(&self) -> &str {
        "antidote"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_antidote())
    }

    /// antidote has no read-only "what is outdated" query, so report up to date.
    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &ANTIDOTE_PLATFORMS
    }

    /// antidote manages user-level shell plugins, so it must never run as root.
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

/// Whether the `antidote` binary is discoverable on `PATH`.
fn detect_antidote() -> Detection {
    if crate::executable::command_exists("antidote") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    use crate::step::Detection;

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }
}
