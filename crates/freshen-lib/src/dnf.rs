//! The dnf step: `dnf upgrade -y`, skipped when dnf is absent, and
//! `dnf autoremove -y` when `--cleanup` is set.
//!
//! The step runs `dnf upgrade -y` whenever `dnf` is available. When dnf is
//! absent but `yum` is available, it runs `yum update -y` instead, so the
//! step is skipped only when neither is installed. It is safe to register
//! on every machine; on macOS and Windows it is never detected.

use crate::cleanup;
use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::restart_needed;
use crate::services_needing_restart;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The dnf package manager step (falls back to `yum` when dnf is absent).
pub const DNF: StepId = "dnf";

/// dnf only runs on Linux.
const DNF_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the dnf step, after the apt step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(DnfStep)
}

struct DnfStep;

/// Which package manager binary the step uses.
enum DnfBinary {
    Dnf,
    Yum,
}

impl DnfBinary {
    fn name(&self) -> &'static str {
        match self {
            Self::Dnf => "dnf",
            Self::Yum => "yum",
        }
    }

    /// Arguments for the upgrade command (`dnf upgrade -y` / `yum update -y`).
    fn upgrade_args(&self) -> &'static [&'static str] {
        match self {
            Self::Dnf => &["upgrade", "-y"],
            Self::Yum => &["update", "-y"],
        }
    }
}

/// The binary to run: `dnf` when present, otherwise `yum`.
fn dnf_binary() -> Option<DnfBinary> {
    if command_exists("dnf") {
        Some(DnfBinary::Dnf)
    } else if command_exists("yum") {
        Some(DnfBinary::Yum)
    } else {
        None
    }
}

impl DnfStep {
    /// Runs `dnf upgrade -y`, or `yum update -y` when dnf is absent.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        let Some(binary) = dnf_binary() else {
            return;
        };
        human!("{} {}", binary.name(), binary.upgrade_args().join(" "));
        if let Err(error) = CommandSpec::new(binary.name())
            .args(binary.upgrade_args())
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(DNF, &error));
        }
    }

    /// After a successful upgrade, records the services that need a restart
    /// (reported by `needs-restarting`) and whether a restart is required.
    fn services_needing_restart(&self) {
        let mut needed = false;
        human!("needs-restarting");
        if let Ok(output) = CommandSpec::new(NEEDS_RESTARTING).run() {
            let services = parse_needs_restarting_services(output.stdout());
            needed = !services.is_empty();
            for service in services {
                services_needing_restart::record(service);
            }
        }
        if needed {
            restart_needed::record();
        }
    }
}

impl Step for DnfStep {
    fn id(&self) -> StepId {
        DNF
    }

    fn display_name(&self) -> &str {
        "dnf"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_dnf())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);

        if let Some(binary) = dnf_binary() {
            if let Err(error) = cleanup::run(
                DNF,
                "dnf: autoremove",
                CommandSpec::new(binary.name())
                    .args(["autoremove", "-y"])
                    .retry_transient_network(NETWORK_RETRIES),
                options,
            ) {
                failures.push(error);
            }
        }

        if failures.is_empty() {
            self.services_needing_restart();
        }
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &DNF_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether `dnf` or `yum` is discoverable on `PATH`.
fn detect_dnf() -> Detection {
    match dnf_binary() {
        Some(_) => Detection::Installed,
        None => Detection::NotInstalled,
    }
}

/// `needs-restarting` (from `dnf-utils`) reports which running services need a
/// restart after a dnf/yum upgrade. It lists one service per line when run
/// without `-r`. When the tool is not installed, or cannot be run, no services
/// are reported.
const NEEDS_RESTARTING: &str = "needs-restarting";

/// Parses the output of `needs-restarting` into the names of services that need
/// a restart. Each non-empty, non-header line is treated as a service name.
fn parse_needs_restarting_services(output: &str) -> Vec<String> {
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
