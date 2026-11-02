//! The rpm-ostree step: `rpm-ostree upgrade`, a staged upgrade that requires
//! a reboot to take effect.
//!
//! `rpm-ostree upgrade` stages a new deployment on Fedora Silverblue/Kinoite
//! (and other rpm-ostree systems). The upgrade is not applied until the
//! machine reboots, so a successful run announces that a reboot is pending.
//! The step is skipped when `rpm-ostree` is absent, so it is safe to register
//! on every machine; on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::restart_needed;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The rpm-ostree step.
pub const RPM_OSTREE: StepId = "rpm-ostree";

/// rpm-ostree only runs on Linux.
const RPM_OSTREE_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the rpm-ostree step, after the other Linux package manager steps.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(RpmOstreeStep)
}

struct RpmOstreeStep;

impl RpmOstreeStep {
    /// Runs `rpm-ostree upgrade`. The upgrade is staged and requires a reboot
    /// to take effect, so a successful run records that a restart is needed.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("rpm-ostree upgrade");
        if let Err(error) = CommandSpec::new("rpm-ostree")
            .args(["upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(RPM_OSTREE, &error));
        } else {
            restart_needed::record();
        }
    }
}

impl Step for RpmOstreeStep {
    fn id(&self) -> StepId {
        RPM_OSTREE
    }

    fn display_name(&self) -> &str {
        "rpm-ostree"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_rpm_ostree())
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
        &RPM_OSTREE_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `rpm-ostree` binary is discoverable on `PATH`.
fn detect_rpm_ostree() -> Detection {
    if command_exists("rpm-ostree") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}
