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

        if failures.is_empty() && dnf_restart_needed() {
            restart_needed::record();
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

/// Whether a restart is needed after the dnf upgrade.
///
/// `needs-restarting -r` (from `dnf-utils`) reports whether running services
/// or the kernel itself require a reboot. It exits 0 when no restart is
/// needed and 1 when one is. When the tool is not installed, or cannot be
/// run, no restart is reported.
fn dnf_restart_needed() -> bool {
    if !command_exists("needs-restarting") {
        return false;
    }
    match CommandSpec::new("needs-restarting").args(["-r"]).run() {
        Ok(_) => false, // exit 0: no restart needed
        Err(crate::process::ProcessError::Failed { status, .. }) => !status.success(), // non-zero: restart needed
        Err(_) => false, // could not run: assume no restart
    }
}
