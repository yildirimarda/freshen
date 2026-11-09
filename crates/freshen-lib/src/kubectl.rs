//! The kubectl plugins step: `kubectl krew upgrade`, skipped when `kubectl`
//! is absent.
//!
//! The step runs `kubectl krew upgrade` whenever `kubectl` is available. It
//! is skipped entirely when `kubectl` is absent, so it is safe to register on
//! every machine. kubectl plugins installed through krew are per-user, so the
//! step must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The kubectl plugins step.
pub const KUBECTL: StepId = "kubectl";

/// kubectl and krew run on every platform freshen supports.
const KUBECTL_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the kubectl plugins step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(KubectlStep)
}

struct KubectlStep;

impl KubectlStep {
    /// Runs `kubectl krew upgrade` to upgrade every installed kubectl plugin.
    fn upgrade_plugins(&self, failures: &mut Vec<StepError>) {
        human!("kubectl krew upgrade");
        if let Err(error) = CommandSpec::new("kubectl")
            .args(["krew", "upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(KUBECTL, &error));
        }
    }
}

impl Step for KubectlStep {
    fn id(&self) -> StepId {
        KUBECTL
    }

    fn display_name(&self) -> &str {
        "kubectl plugins"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_kubectl())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // `kubectl krew` has no read-only "which plugins are outdated" query,
        // so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade_plugins(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &KUBECTL_PLATFORMS
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

/// Whether `kubectl` is discoverable on `PATH`.
fn detect_kubectl() -> Detection {
    if crate::executable::command_exists("kubectl") {
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
