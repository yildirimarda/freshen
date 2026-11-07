//! The pipx step: `pipx upgrade-all`, skipped when pipx is absent.
//!
//! The step runs `pipx upgrade-all` whenever `pipx` is available. It is
//! skipped entirely when pipx is absent, so it is safe to register on every
//! machine. pipx manages user-level packages, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The pipx package manager step.
pub const PIPX: StepId = "pipx";

/// pipx runs on every platform freshen supports.
const PIPX_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the pipx step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(PipxStep)
}

struct PipxStep;

impl PipxStep {
    /// Runs `pipx upgrade-all`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("pipx: upgrade-all");
        if let Err(error) = CommandSpec::new("pipx")
            .args(["upgrade-all"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(PIPX, &error));
        }
    }
}

impl Step for PipxStep {
    fn id(&self) -> StepId {
        PIPX
    }

    fn display_name(&self) -> &str {
        "pipx"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_pipx())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // pipx has no read-only "what is outdated" query in the basic form;
        // the report-only `pip list --user --outdated` check is a separate
        // step, so report up to date here.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &PIPX_PLATFORMS
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

/// Whether the `pipx` binary is discoverable on `PATH`.
fn detect_pipx() -> Detection {
    if crate::executable::command_exists("pipx") {
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
