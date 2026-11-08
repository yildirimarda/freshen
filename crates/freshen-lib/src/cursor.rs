//! The Cursor step: `cursor --update-extensions`, skipped when the CLI is absent.
//!
//! The step runs `cursor --update-extensions` whenever the `cursor` CLI is
//! available. It is skipped entirely when `cursor` is absent, so it is safe to
//! register on every machine. Cursor extensions are per-user, so the step
//! must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Cursor extension step.
pub const CURSOR: StepId = "cursor";

/// Cursor runs on every platform freshen supports.
const CURSOR_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the Cursor step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(CursorStep)
}

struct CursorStep;

impl CursorStep {
    /// Runs `cursor --update-extensions`.
    fn update_extensions(&self, failures: &mut Vec<StepError>) {
        human!("cursor: update extensions");
        if let Err(error) = CommandSpec::new("cursor")
            .args(["--update-extensions"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(CURSOR, &error));
        }
    }
}

impl Step for CursorStep {
    fn id(&self) -> StepId {
        CURSOR
    }

    fn display_name(&self) -> &str {
        "Cursor"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_cursor())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // `cursor` has no read-only "which extensions are outdated" query, so
        // report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_extensions(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &CURSOR_PLATFORMS
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

/// Whether the `cursor` CLI is discoverable on `PATH`.
fn detect_cursor() -> Detection {
    if crate::executable::command_exists("cursor") {
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
