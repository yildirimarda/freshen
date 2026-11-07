//! The cargo-update step: `cargo install-update -a`, skipped when cargo-update
//! is absent.
//!
//! The step runs `cargo install-update -a` whenever `cargo-update` is
//! available. It is skipped entirely when cargo-update is absent, so it is
//! safe to register on every machine. cargo-update manages user-level crates,
//! so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The cargo-update crates step.
pub const CARGO_UPDATE: StepId = "cargo-update";

/// cargo-update runs on every platform freshen supports.
const CARGO_UPDATE_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the cargo-update step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(CargoUpdateStep)
}

struct CargoUpdateStep;

impl CargoUpdateStep {
    /// Runs `cargo install-update -a`.
    fn install_update(&self, failures: &mut Vec<StepError>) {
        human!("cargo-update: install-update -a");
        if let Err(error) = CommandSpec::new("cargo")
            .args(["install-update", "-a"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(CARGO_UPDATE, &error));
        }
    }
}

impl Step for CargoUpdateStep {
    fn id(&self) -> StepId {
        CARGO_UPDATE
    }

    fn display_name(&self) -> &str {
        "cargo-update"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_cargo_update())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // cargo-update has no read-only "what is outdated" query, so report up
        // to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.install_update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &CARGO_UPDATE_PLATFORMS
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

/// Whether the `cargo-install-update` binary is discoverable on `PATH`.
///
/// `cargo install-update` works because the `cargo-update` crate installs a
/// `cargo-install-update` executable that cargo dispatches to.
fn detect_cargo_update() -> Detection {
    if crate::executable::command_exists("cargo-install-update") {
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
