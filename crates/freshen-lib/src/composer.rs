//! The Composer step: `composer self-update` followed by `composer global
//! update`, skipped when composer is absent.
//!
//! The step runs `composer self-update` (a best-effort self-update whose
//! failure is only a warning, like uv's and mise's self-update) and then
//! `composer global update` to upgrade globally installed PHP packages. The
//! step is skipped entirely when composer is absent, so it is safe to register
//! on every machine. Composer manages user-level packages, so it must never
//! run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Composer step.
pub const COMPOSER: StepId = "composer";

/// Composer runs on every platform freshen supports.
const COMPOSER_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the composer step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ComposerStep)
}

struct ComposerStep;

impl ComposerStep {
    /// Runs `composer self-update`. A failure is only a warning: the installed
    /// composer can still be used to update global packages.
    fn self_update(&self) {
        human!("composer: self-update");
        if let Err(error) = CommandSpec::new("composer")
            .args(["self-update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            human!("warning: composer self-update skipped/failed: {error}");
        }
    }

    /// Runs `composer global update` to upgrade globally installed packages.
    fn update_global(&self, failures: &mut Vec<StepError>) {
        human!("composer: global update");
        if let Err(error) = CommandSpec::new("composer")
            .args(["global", "update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(COMPOSER, &error));
        }
    }
}

impl Step for ComposerStep {
    fn id(&self) -> StepId {
        COMPOSER
    }

    fn display_name(&self) -> &str {
        "composer"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_composer())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // Composer has no read-only "what is outdated" query for globally
        // installed packages, so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.self_update();
        self.update_global(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &COMPOSER_PLATFORMS
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

/// Whether the `composer` binary is discoverable on `PATH`.
fn detect_composer() -> Detection {
    if crate::executable::command_exists("composer") {
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
