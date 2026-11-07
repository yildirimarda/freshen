//! The asdf step: `asdf plugin update --all`, skipped when asdf is absent.
//!
//! The step runs `asdf plugin update --all` whenever `asdf` is available. It
//! is skipped entirely when asdf is absent, so it is safe to register on every
//! machine. asdf manages user-level tools, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The asdf plugin step.
pub const ASDF: StepId = "asdf";

/// asdf runs on every platform freshen supports.
const ASDF_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the asdf step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(AsdfStep)
}

struct AsdfStep;

impl AsdfStep {
    /// Runs `asdf plugin update --all`.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("asdf: plugin update --all");
        if let Err(error) = CommandSpec::new("asdf")
            .args(["plugin", "update", "--all"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ASDF, &error));
        }
    }
}

impl Step for AsdfStep {
    fn id(&self) -> StepId {
        ASDF
    }

    fn display_name(&self) -> &str {
        "asdf"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_asdf())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // asdf has no read-only "what is outdated" query, so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &ASDF_PLATFORMS
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

/// Whether the `asdf` binary is discoverable on `PATH`.
fn detect_asdf() -> Detection {
    if crate::executable::command_exists("asdf") {
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
