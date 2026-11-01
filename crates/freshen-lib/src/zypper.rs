//! The zypper step: `zypper refresh` followed by `zypper update -y`, skipped
//! when zypper is absent.
//!
//! The step runs `zypper refresh` and then `zypper update -y` whenever `zypper`
//! is available, collecting the first failure so both commands are attempted.
//! It is skipped entirely when zypper is absent, so it is safe to register on
//! every machine; on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::services_needing_restart;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The zypper package manager step.
pub const ZYPPER: StepId = "zypper";

/// zypper only runs on Linux.
const ZYPPER_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the zypper step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ZypperStep)
}

struct ZypperStep;

impl ZypperStep {
    /// Runs `zypper refresh` and then `zypper update -y`, collecting the
    /// first failure so both commands are attempted even when refresh fails.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("zypper refresh");
        if let Err(error) = CommandSpec::new("zypper")
            .args(["refresh"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ZYPPER, &error));
        }

        human!("zypper update -y");
        if let Err(error) = CommandSpec::new("zypper")
            .args(["update", "-y"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ZYPPER, &error));
        }
    }
}

impl Step for ZypperStep {
    fn id(&self) -> StepId {
        ZYPPER
    }

    fn display_name(&self) -> &str {
        "zypper"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_zypper())
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
        if failures.is_empty() {
            self.services_needing_restart();
        }
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &ZYPPER_PLATFORMS
    }
}

impl ZypperStep {
    /// Runs `zypper ps -s` after a successful upgrade and records the services
    /// that need a restart. A failure to list services does not fail the step:
    /// the upgrade itself succeeded.
    fn services_needing_restart(&self) {
        human!("zypper ps -s");
        let output = match CommandSpec::new("zypper").args(["ps", "-s"]).run() {
            Ok(output) => output,
            Err(_error) => {
                // The upgrade succeeded; not being able to list services is not
                // a step failure.
                return;
            }
        };
        for service in parse_zypper_ps_services(output.stdout()) {
            services_needing_restart::record(service);
        }
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `zypper` binary is discoverable on `PATH`.
fn detect_zypper() -> Detection {
    if command_exists("zypper") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses the output of `zypper ps -s` into the names of services that need a
/// restart.
///
/// `zypper ps -s` prints one service per line (a systemd unit name such as
/// `ssh.service`). Header and separator lines are ignored; every remaining
/// non-empty line is treated as a service name.
fn parse_zypper_ps_services(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.contains("PID") && !line.contains("Service"))
        .filter(|line| {
            !line
                .chars()
                .all(|c| c == '-' || c == '+' || c == '|' || c.is_whitespace())
        })
        .map(ToOwned::to_owned)
        .collect()
}
