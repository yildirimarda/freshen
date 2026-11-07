//! The yarn step: `yarn global upgrade`, classic yarn only.
//!
//! The step runs `yarn global upgrade` whenever `yarn` is available. It is
//! skipped entirely when yarn is absent, so it is safe to register on every
//! machine. Yarn manages user-level global packages, so it must never run as
//! root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The yarn global-packages step.
pub const YARN: StepId = "yarn";

/// yarn runs on every platform freshen supports.
const YARN_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the yarn step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(YarnStep)
}

struct YarnStep;

impl YarnStep {
    /// Runs `yarn global upgrade`.
    fn upgrade_global_packages(&self, failures: &mut Vec<StepError>) {
        human!("yarn: global upgrade");
        if let Err(error) = CommandSpec::new("yarn")
            .args(["global", "upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(YARN, &error));
        }
    }
}

impl Step for YarnStep {
    fn id(&self) -> StepId {
        YARN
    }

    fn display_name(&self) -> &str {
        "yarn"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_yarn())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // classic yarn has no read-only "what is outdated" query, so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade_global_packages(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &YARN_PLATFORMS
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

/// Whether `yarn` is discoverable on `PATH`.
fn detect_yarn() -> Detection {
    if crate::executable::command_exists("yarn") {
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
