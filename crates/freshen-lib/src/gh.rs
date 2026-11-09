//! The GitHub CLI extensions step: `gh extension upgrade --all`, skipped when
//! the CLI is absent.
//!
//! The step runs `gh extension upgrade --all` whenever the `gh` CLI is
//! available. It is skipped entirely when `gh` is absent, so it is safe to
//! register on every machine. GitHub CLI extensions are per-user, so the step
//! must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The GitHub CLI extension step.
pub const GH: StepId = "gh";

/// GitHub CLI runs on every platform freshen supports.
const GH_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the GitHub CLI extensions step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(GhStep)
}

struct GhStep;

impl GhStep {
    /// Runs `gh extension upgrade --all`.
    fn upgrade_extensions(&self, failures: &mut Vec<StepError>) {
        human!("gh: upgrade extensions");
        if let Err(error) = CommandSpec::new("gh")
            .args(["extension", "upgrade", "--all"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(GH, &error));
        }
    }
}

impl Step for GhStep {
    fn id(&self) -> StepId {
        GH
    }

    fn display_name(&self) -> &str {
        "GitHub CLI"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_gh())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // `gh` has no read-only "which extensions are outdated" query, so
        // report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade_extensions(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &GH_PLATFORMS
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

/// Whether the `gh` CLI is discoverable on `PATH`.
fn detect_gh() -> Detection {
    if crate::executable::command_exists("gh") {
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
