//! The oh-my-zsh step: `omz update --unattended`, skipped when `omz` is absent.
//!
//! The step runs `omz update --unattended` whenever `omz` is available. It is
//! skipped entirely when `omz` is absent, so it is safe to register on every
//! machine. oh-my-zsh manages user-level shell config, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The oh-my-zsh step.
pub const OH_MY_ZSH: StepId = "oh-my-zsh";

/// oh-my-zsh runs on every platform freshen supports.
const OH_MY_ZSH_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the oh-my-zsh step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(OhMyZshStep)
}

struct OhMyZshStep;

impl OhMyZshStep {
    /// Runs `omz update --unattended`.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("omz update --unattended");
        if let Err(error) = CommandSpec::new("omz")
            .args(["update", "--unattended"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(OH_MY_ZSH, &error));
        }
    }
}

impl Step for OhMyZshStep {
    fn id(&self) -> StepId {
        OH_MY_ZSH
    }

    fn display_name(&self) -> &str {
        "oh-my-zsh"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_oh_my_zsh())
    }

    /// oh-my-zsh has no read-only "what is outdated" query, so report up to date.
    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &OH_MY_ZSH_PLATFORMS
    }

    /// oh-my-zsh manages user-level shell config, so it must never run as root.
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

/// Whether the `omz` binary is discoverable on `PATH`.
fn detect_oh_my_zsh() -> Detection {
    if crate::executable::command_exists("omz") {
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
