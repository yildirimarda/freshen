//! The bootc step: `bootc upgrade`, a staged upgrade that requires a reboot
//! to take effect.
//!
//! `bootc upgrade` stages a new OS deployment on Fedora CoreOS and other
//! bootc-based systems. The upgrade is not applied until the machine reboots,
//! so a successful run announces that a reboot is pending. The step is skipped
//! when `bootc` is absent, so it is safe to register on every machine; on
//! macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::restart_needed;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The bootc step.
pub const BOOTC: StepId = "bootc";

/// bootc only runs on Linux.
const BOOTC_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the bootc step, after the rpm-ostree step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(BootcStep)
}

struct BootcStep;

impl BootcStep {
    /// Runs `bootc upgrade`. The upgrade is staged and requires a reboot to
    /// take effect, so a successful run records that a restart is needed.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("bootc upgrade");
        if let Err(error) = CommandSpec::new("bootc")
            .args(["upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(BOOTC, &error));
        } else {
            restart_needed::record();
        }
    }
}

impl Step for BootcStep {
    fn id(&self) -> StepId {
        BOOTC
    }

    fn display_name(&self) -> &str {
        "bootc"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_bootc())
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
        &BOOTC_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `bootc` binary is discoverable on `PATH`.
fn detect_bootc() -> Detection {
    if command_exists("bootc") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}
