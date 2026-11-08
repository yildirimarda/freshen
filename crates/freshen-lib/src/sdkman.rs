//! The SDKMAN step: `sdk selfupdate` followed by `sdk upgrade`, skipped when
//! `sdk` is absent.
//!
//! The step runs `sdk selfupdate` and then `sdk upgrade` whenever `sdk` is
//! available. Every child freshen spawns gets a closed standard input, so both
//! commands run in a non-interactive shell and never block on a prompt. The
//! step is skipped entirely when `sdk` is absent, so it is safe to register on
//! every machine; on Windows it is never detected. SDKMAN manages user-level
//! SDKs, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The SDKMAN step.
pub const SDKMAN: StepId = "sdkman";

/// SDKMAN runs on Linux and macOS, where the `sdk` command is available.
const SDKMAN_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Macos];

/// Registers the SDKMAN step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(SdkmanStep)
}

struct SdkmanStep;

impl SdkmanStep {
    /// Runs `sdk selfupdate` and then `sdk upgrade`, collecting the first
    /// failure so both commands are attempted.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("sdk selfupdate");
        if let Err(error) = CommandSpec::new("sdk")
            .args(["selfupdate"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(SDKMAN, &error));
        }

        human!("sdk upgrade");
        if let Err(error) = CommandSpec::new("sdk")
            .args(["upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(SDKMAN, &error));
        }
    }
}

impl Step for SdkmanStep {
    fn id(&self) -> StepId {
        SDKMAN
    }

    fn display_name(&self) -> &str {
        "SDKMAN"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_sdkman())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // SDKMAN has no read-only "what is outdated" query, so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &SDKMAN_PLATFORMS
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

/// Whether the `sdk` binary is discoverable on `PATH`.
fn detect_sdkman() -> Detection {
    if crate::executable::command_exists("sdk") {
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
