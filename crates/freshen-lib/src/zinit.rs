//! The zinit step: `zinit self-update` followed by `zinit update --all`, skipped
//! when `zinit` is absent.
//!
//! The step runs `zinit self-update` and then `zinit update --all` whenever
//! `zinit` is available. It is skipped entirely when `zinit` is absent, so it
//! is safe to register on every machine. zinit manages user-level shell
//! plugins, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The zinit step.
pub const ZINIT: StepId = "zinit";

/// zinit runs on every platform freshen supports.
const ZINIT_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the zinit step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ZinitStep)
}

struct ZinitStep;

impl ZinitStep {
    /// Runs `zinit self-update` and then `zinit update --all`, collecting the
    /// first failure so both commands are attempted.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("zinit self-update");
        if let Err(error) = CommandSpec::new("zinit")
            .args(["self-update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ZINIT, &error));
        }

        human!("zinit update --all");
        if let Err(error) = CommandSpec::new("zinit")
            .args(["update", "--all"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ZINIT, &error));
        }
    }
}

impl Step for ZinitStep {
    fn id(&self) -> StepId {
        ZINIT
    }

    fn display_name(&self) -> &str {
        "zinit"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_zinit())
    }

    /// zinit has no read-only "what is outdated" query, so report up to date.
    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &ZINIT_PLATFORMS
    }

    /// zinit manages user-level shell plugins, so it must never run as root.
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

/// Whether the `zinit` binary is discoverable on `PATH`.
fn detect_zinit() -> Detection {
    if crate::executable::command_exists("zinit") {
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
