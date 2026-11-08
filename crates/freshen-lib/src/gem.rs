//! The gem step: `gem update` for user-installed gems, skipping the system
//! gem directory.
//!
//! The step runs `gem update` whenever `gem` is available. It upgrades the
//! gems installed in the user's Ruby environment (GEM_HOME) and never touches
//! the system gem directory (`gem update --system` is not run). It is skipped
//! entirely when `gem` is absent, so it is safe to register on every machine.
//! RubyGems installs into the user's Ruby environment, so it must never run
//! as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The RubyGems step.
pub const GEM: StepId = "gem";

/// gem runs on every platform freshen supports.
const GEM_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the gem step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(GemStep)
}

struct GemStep;

impl GemStep {
    /// Upgrades the user-installed gems, skipping the system gem directory.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("gem: update");
        if let Err(error) = CommandSpec::new("gem")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(GEM, &error));
        }
    }
}

impl Step for GemStep {
    fn id(&self) -> StepId {
        GEM
    }

    fn display_name(&self) -> &str {
        "gem"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_gem())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // RubyGems has no read-only "what is outdated" query for the system,
        // so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &GEM_PLATFORMS
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

/// Whether the `gem` binary is discoverable on `PATH`.
fn detect_gem() -> Detection {
    if crate::executable::command_exists("gem") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}
