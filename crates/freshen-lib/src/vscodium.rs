//! The VSCodium step: `codium --update-extensions`, skipped when the CLI is absent.
//!
//! The step runs `codium --update-extensions` whenever the `codium` CLI is
//! available. It is skipped entirely when `codium` is absent, so it is safe to
//! register on every machine. VSCodium extensions are per-user, so the step
//! must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The VSCodium extension step.
pub const VSCODIUM: StepId = "vscodium";

/// VSCodium runs on every platform freshen supports.
const VSCODIUM_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the VSCodium step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(VscodiumStep)
}

struct VscodiumStep;

impl VscodiumStep {
    /// Runs `codium --update-extensions`.
    fn update_extensions(&self, failures: &mut Vec<StepError>) {
        human!("vscodium: update extensions");
        if let Err(error) = CommandSpec::new("codium")
            .args(["--update-extensions"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(VSCODIUM, &error));
        }
    }
}

impl Step for VscodiumStep {
    fn id(&self) -> StepId {
        VSCODIUM
    }

    fn display_name(&self) -> &str {
        "VSCodium"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_codium())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // `codium` has no read-only "which extensions are outdated" query, so
        // report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_extensions(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &VSCODIUM_PLATFORMS
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

/// Whether the `codium` CLI is discoverable on `PATH`.
fn detect_codium() -> Detection {
    if crate::executable::command_exists("codium") {
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
