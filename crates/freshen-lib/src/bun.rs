//! The bun step: `bun update -g`, skipped when bun is absent.
//!
//! The step runs `bun update -g` whenever `bun` is available. It is skipped
//! entirely when bun is absent, so it is safe to register on every machine.
//! bun manages user-level global packages, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The bun global-packages step.
pub const BUN: StepId = "bun";

/// bun runs on every platform freshen supports.
const BUN_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the bun step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(BunStep)
}

struct BunStep;

impl BunStep {
    /// Runs `bun update -g`.
    fn update_global_packages(&self, failures: &mut Vec<StepError>) {
        human!("bun: update -g");
        if let Err(error) = CommandSpec::new("bun")
            .args(["update", "-g"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(BUN, &error));
        }
    }
}

impl Step for BunStep {
    fn id(&self) -> StepId {
        BUN
    }

    fn display_name(&self) -> &str {
        "bun"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_bun())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // bun has no read-only "what is outdated" query, so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_global_packages(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &BUN_PLATFORMS
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

/// Whether `bun` is discoverable on `PATH`.
fn detect_bun() -> Detection {
    if crate::executable::command_exists("bun") {
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
