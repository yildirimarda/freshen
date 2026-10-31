//! The AUR helpers step: `yay -Syu --noconfirm` or `paru -Syu --noconfirm`,
//! skipped when neither helper is installed.
//!
//! The step runs the upgrade command of whichever AUR helper is installed
//! (`yay` is preferred, `paru` is the fallback). It only runs on Linux and
//! only when a helper is present, so it is safe to register on every machine;
//! on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The AUR helpers step.
pub const AUR: StepId = "aur";

/// AUR helpers only run on Linux.
const AUR_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// The AUR helpers, in preference order: `yay` first, `paru` as the fallback.
const AUR_HELPERS: [&str; 2] = ["yay", "paru"];

/// Registers the AUR helpers step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(AurStep)
}

struct AurStep;

impl AurStep {
    /// Runs the upgrade command of the installed AUR helper (`yay` is
    /// preferred, `paru` is the fallback).
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        let Some(helper) = Self::installed_helper() else {
            return;
        };
        human!("{helper} -Syu --noconfirm");
        if let Err(error) = CommandSpec::new(helper)
            .args(["-Syu", "--noconfirm"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(AUR, &error));
        }
    }

    /// The AUR helper that is installed on `PATH`, preferring `yay`.
    fn installed_helper() -> Option<&'static str> {
        AUR_HELPERS
            .iter()
            .copied()
            .find(|helper| command_exists(helper))
    }
}

impl Step for AurStep {
    fn id(&self) -> StepId {
        AUR
    }

    fn display_name(&self) -> &str {
        "aur"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(if Self::installed_helper().is_some() {
            Detection::Installed
        } else {
            Detection::NotInstalled
        })
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
        &AUR_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
