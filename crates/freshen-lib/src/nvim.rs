//! The lazy.nvim step: `nvim --headless "+Lazy! sync" +qa`, skipped when nvim
//! is absent.
//!
//! Syncing plugins is opt-in: without `--install-nvim-lazy` the step reports
//! that the sync is available but changes nothing. With the flag the step runs
//! `nvim --headless "+Lazy! sync" +qa` to install and update every plugin
//! declared in the lazy.nvim lockfile. There is no read-only way to tell
//! whether plugins are out of date, so `check()` always reports the step up to
//! date once `nvim` is detected.
//!
//! The step runs on every platform freshen supports, because `nvim` exists on
//! macOS, Linux and Windows. lazy.nvim manages user-level config, so it must
//! never run as root.

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The lazy.nvim plugin sync step.
pub const NVIM_LAZY: StepId = "nvim-lazy";

/// nvim runs on every platform freshen supports.
const NVIM_LAZY_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the lazy.nvim step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(NvimLazyStep)
}

/// The lazy.nvim plugin sync step.
struct NvimLazyStep;

impl NvimLazyStep {
    /// Whether the opt-in flag that permits `nvim --headless "+Lazy! sync"
    /// +qa` is set.
    fn sync_enabled(options: &RunOptions) -> bool {
        options.install_nvim_lazy()
    }

    /// Runs `nvim --headless "+Lazy! sync" +qa`.
    fn sync_plugins(&self, failures: &mut Vec<StepError>) {
        human!("nvim-lazy: sync plugins");
        if let Err(error) = CommandSpec::new("nvim")
            .args(["--headless", "+Lazy! sync", "+qa"])
            .run()
        {
            failures.push(StepError::run_from_process(NVIM_LAZY, &error));
        }
    }
}

impl Step for NvimLazyStep {
    fn id(&self) -> StepId {
        NVIM_LAZY
    }

    fn display_name(&self) -> &str {
        "lazy.nvim"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_nvim())
    }

    /// Reports the step up to date: there is no read-only query that tells
    /// whether plugins need syncing, so the step cannot report pending work
    /// without changing anything. The listing is left to `--list-steps`.
    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    /// Runs `nvim --headless "+Lazy! sync" +qa` only when the opt-in flag is
    /// set. Without it the step reports that the sync is available and changes
    /// nothing.
    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if !Self::sync_enabled(options) {
            human!("nvim-lazy: plugin sync available; nothing synced (use --install-nvim-lazy)");
            return Ok(());
        }

        let mut failures = Vec::new();
        self.sync_plugins(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &NVIM_LAZY_PLATFORMS
    }

    /// lazy.nvim manages user-level config, so it must never run as root.
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

/// Whether `nvim` is discoverable on `PATH`.
fn detect_nvim() -> Detection {
    if crate::executable::command_exists("nvim") {
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
