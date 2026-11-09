//! The Helm step: `helm repo update`, skipped when helm is absent.
//!
//! The step runs `helm repo update` whenever `helm` is available. It is
//! skipped entirely when helm is absent, so it is safe to register on every
//! machine; on macOS it is never detected.

#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::executable::command_exists;
use crate::options::RunOptions;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::output::human;
use crate::platform::Platform;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Helm package manager step.
pub const HELM: StepId = "helm";

/// Helm runs on Linux and Windows; on macOS it is never detected.
const HELM_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// Registers the helm step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(HelmStep)
}

struct HelmStep;

impl HelmStep {
    /// Runs `helm repo update`.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("helm repo update");
        if let Err(error) = CommandSpec::new("helm")
            .args(["repo", "update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(HELM, &error));
        }
    }
}

impl Step for HelmStep {
    fn id(&self) -> StepId {
        HELM
    }

    fn display_name(&self) -> &str {
        "helm"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        return Ok(detect_helm());
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        return Ok(Detection::NotInstalled);
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    #[allow(unused_variables)]
    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let mut failures = Vec::new();
            self.upgrade(&mut failures);
            finish(failures)
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            // helm is not registered on macOS; nothing to run.
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &HELM_PLATFORMS
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `helm` binary is discoverable on `PATH`.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn detect_helm() -> Detection {
    if command_exists("helm") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use super::detect_helm;
    use crate::step::Detection;

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn detect_helm_reports_not_installed_when_absent_from_path() {
        // `helm` may be installed on the machine the test runs on (CI runners
        // ship it), so detection must be exercised against a `PATH` that does
        // not contain `helm` instead of relying on the ambient environment.
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let dir = std::env::temp_dir().join(format!(
            "freshen-helm-absent-{pid}",
            pid = std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        std::env::set_var("PATH", &dir);
        assert!(
            !detect_helm().is_installed(),
            "helm must not be detected when absent from PATH"
        );
        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }
}
