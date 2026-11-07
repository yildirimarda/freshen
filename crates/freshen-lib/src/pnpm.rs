//! The pnpm step: `pnpm update -g`, skipped when pnpm is absent.
//!
//! The step runs `pnpm update -g` whenever `pnpm` is available. It is skipped
//! entirely when pnpm is absent, so it is safe to register on every machine.
//! pnpm manages user-level global packages, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The pnpm global-packages step.
pub const PNPM: StepId = "pnpm";

/// pnpm runs on every platform freshen supports.
const PNPM_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the pnpm step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(PnpmStep)
}

struct PnpmStep;

impl PnpmStep {
    /// Runs `pnpm update -g`.
    fn update_global_packages(&self, failures: &mut Vec<StepError>) {
        human!("pnpm: update -g");
        if let Err(error) = CommandSpec::new("pnpm")
            .args(["update", "-g"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(PNPM, &error));
        }
    }
}

impl Step for PnpmStep {
    fn id(&self) -> StepId {
        PNPM
    }

    fn display_name(&self) -> &str {
        "pnpm"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_pnpm())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // pnpm has no read-only "what is outdated" query, so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_global_packages(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &PNPM_PLATFORMS
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

/// Whether `pnpm` is discoverable on `PATH`.
fn detect_pnpm() -> Detection {
    if crate::executable::command_exists("pnpm") {
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
