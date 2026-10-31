//! The pacman step: `pacman -Syu --noconfirm`, skipped when pacman is absent.
//!
//! The step runs `pacman -Syu --noconfirm` whenever `pacman` is available.
//! It is skipped entirely when pacman is absent, so it is safe to register
//! on every machine; on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The pacman package manager step.
pub const PACMAN: StepId = "pacman";

/// pacman only runs on Linux.
const PACMAN_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the pacman step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(PacmanStep)
}

struct PacmanStep;

impl PacmanStep {
    /// Runs `pacman -Syu --noconfirm`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("pacman -Syu --noconfirm");
        if let Err(error) = CommandSpec::new("pacman")
            .args(["-Syu", "--noconfirm"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(PACMAN, &error));
        }
    }
}

impl Step for PacmanStep {
    fn id(&self) -> StepId {
        PACMAN
    }

    fn display_name(&self) -> &str {
        "pacman"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_pacman())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &PACMAN_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `pacman` binary is discoverable on `PATH`.
fn detect_pacman() -> Detection {
    if command_exists("pacman") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}
