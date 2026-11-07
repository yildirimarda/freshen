//! The rustup step: `rustup update`, skipped when rustup is absent.
//!
//! The step runs `rustup update` whenever `rustup` is available. It is
//! skipped entirely when rustup is absent, so it is safe to register on every
//! machine. rustup manages user-level toolchains, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The rustup toolchain step.
pub const RUSTUP: StepId = "rustup";

/// rustup runs on every platform freshen supports.
const RUSTUP_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the rustup step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(RustupStep)
}

struct RustupStep;

impl RustupStep {
    /// Runs `rustup update`.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("rustup: update");
        if let Err(error) = CommandSpec::new("rustup")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(RUSTUP, &error));
        }
    }
}

impl Step for RustupStep {
    fn id(&self) -> StepId {
        RUSTUP
    }

    fn display_name(&self) -> &str {
        "rustup"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_rustup())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // rustup has no read-only "what is outdated" query, so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &RUSTUP_PLATFORMS
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

/// Whether the `rustup` binary is discoverable on `PATH`.
fn detect_rustup() -> Detection {
    if crate::executable::command_exists("rustup") {
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
