//! The tmux plugin manager step: `~/.tmux/plugins/tpm/bin/update_plugins all`,
//! skipped when tpm is absent.
//!
//! The step runs `~/.tmux/plugins/tpm/bin/update_plugins all` whenever tpm is
//! installed. It is skipped entirely when tpm is absent, so it is safe to
//! register on every machine. tpm manages user-level tmux plugins, so it must
//! never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
use std::path::{Path, PathBuf};

/// The tmux plugin manager step.
pub const TPM: StepId = "tpm";

/// tpm runs on every platform freshen supports; detection is by the presence of
/// its `update_plugins` script, so it is simply skipped on machines without tpm.
const TPM_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the tmux plugin manager step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(TpmStep)
}

/// The path to tpm's `update_plugins` script, relative to the home directory.
const TPM_UPDATE_PLUGINS: &str = ".tmux/plugins/tpm/bin/update_plugins";

struct TpmStep;

impl Step for TpmStep {
    fn id(&self) -> StepId {
        TPM
    }

    fn display_name(&self) -> &str {
        "tmux plugin manager"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_tpm())
    }

    /// tpm has no read-only "what is outdated" query, so report up to date.
    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &TPM_PLATFORMS
    }

    /// tpm manages user-level tmux plugins, so it must never run as root.
    fn is_user_level(&self) -> bool {
        true
    }
}

impl TpmStep {
    /// Runs `~/.tmux/plugins/tpm/bin/update_plugins all`.
    fn update(&self, failures: &mut Vec<StepError>) {
        let Some(path) = tpm_update_plugins_path() else {
            return;
        };
        human!("tpm: update plugins");
        if let Err(error) = CommandSpec::new(path)
            .args(["all"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(TPM, &error));
        }
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The absolute path to tpm's `update_plugins` script, when it exists.
fn tpm_update_plugins_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let path = Path::new(&home).join(TPM_UPDATE_PLUGINS);
    path.is_file().then_some(path)
}

/// Whether the `~/.tmux/plugins/tpm/bin/update_plugins` script is installed.
fn detect_tpm() -> Detection {
    if tpm_update_plugins_path().is_some() {
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
