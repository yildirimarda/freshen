//! The apk step: `apk update` followed by `apk upgrade`, skipped when apk is
//! absent.
//!
//! The step runs `apk update` and then `apk upgrade` whenever `apk` is
//! available, collecting the first failure so both commands are attempted.
//! It is skipped entirely when apk is absent, so it is safe to register on
//! every machine; on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The apk package manager step (Alpine Linux).
pub const APK: StepId = "apk";

/// apk only runs on Linux.
const APK_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the apk step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ApkStep)
}

struct ApkStep;

impl ApkStep {
    /// Runs `apk update` and then `apk upgrade`, collecting the first failure
    /// so both commands are attempted even when update fails.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("apk update");
        if let Err(error) = CommandSpec::new("apk")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(APK, &error));
        }

        human!("apk upgrade");
        if let Err(error) = CommandSpec::new("apk")
            .args(["upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(APK, &error));
        }
    }
}

impl Step for ApkStep {
    fn id(&self) -> StepId {
        APK
    }

    fn display_name(&self) -> &str {
        "apk"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_apk())
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
        &APK_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `apk` binary is discoverable on `PATH`.
fn detect_apk() -> Detection {
    if command_exists("apk") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}
