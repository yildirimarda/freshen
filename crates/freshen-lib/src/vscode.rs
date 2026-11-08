//! The VS Code step: `code --update-extensions`, skipped when the CLI is absent.
//!
//! The step runs `code --update-extensions` whenever the `code` CLI is
//! available. It is skipped entirely when `code` is absent, so it is safe to
//! register on every machine. VS Code extensions are per-user, so the step
//! must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The VS Code extension step.
pub const VSCODE: StepId = "vscode";

/// VS Code runs on every platform freshen supports.
const VSCODE_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the VS Code step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(VscodeStep)
}

struct VscodeStep;

impl VscodeStep {
    /// Runs `code --update-extensions`.
    fn update_extensions(&self, failures: &mut Vec<StepError>) {
        human!("vscode: update extensions");
        if let Err(error) = CommandSpec::new("code")
            .args(["--update-extensions"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(VSCODE, &error));
        }
    }
}

impl Step for VscodeStep {
    fn id(&self) -> StepId {
        VSCODE
    }

    fn display_name(&self) -> &str {
        "VS Code"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_code())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // `code` has no read-only "which extensions are outdated" query, so
        // report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_extensions(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &VSCODE_PLATFORMS
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

/// Whether the `code` CLI is discoverable on `PATH`.
fn detect_code() -> Detection {
    if crate::executable::command_exists("code") {
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
