//! The packer.nvim step: `nvim --headless "+PackerSync" +qa`, skipped when nvim
//! is absent.
//!
//! Syncing plugins is opt-in: without `--install-nvim-packer` the step reports
//! that the sync is available but changes nothing. With the flag the step runs
//! `nvim --headless "+PackerSync" +qa` to install and update every plugin
//! managed by packer.nvim. There is no read-only way to tell whether plugins
//! need syncing, so `check()` always reports the step up to date once `nvim`
//! is detected.
//!
//! The step runs on every platform freshen supports, because `nvim` exists on
//! macOS, Linux and Windows. packer.nvim manages user-level config, so it must
//! never run as root.

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The packer.nvim plugin sync step.
pub const NVIM_PACKER: StepId = "nvim-packer";

/// nvim runs on every platform freshen supports.
const NVIM_PACKER_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the packer.nvim step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(NvimPackerStep)
}

/// The packer.nvim plugin sync step.
struct NvimPackerStep;

impl NvimPackerStep {
    /// Whether the opt-in flag that permits `nvim --headless "+PackerSync"
    /// +qa` is set.
    fn sync_enabled(options: &RunOptions) -> bool {
        options.install_nvim_packer()
    }

    /// Runs `nvim --headless "+PackerSync" +qa`.
    fn sync_plugins(&self, failures: &mut Vec<StepError>) {
        human!("nvim-packer: sync plugins");
        if let Err(error) = CommandSpec::new("nvim")
            .args(["--headless", "+PackerSync", "+qa"])
            .run()
        {
            failures.push(StepError::run_from_process(NVIM_PACKER, &error));
        }
    }
}

impl Step for NvimPackerStep {
    fn id(&self) -> StepId {
        NVIM_PACKER
    }

    fn display_name(&self) -> &str {
        "packer"
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

    /// Runs `nvim --headless "+PackerSync" +qa` only when the opt-in flag is
    /// set. Without it the step reports that the sync is available and changes
    /// nothing.
    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if !Self::sync_enabled(options) {
            human!(
                "nvim-packer: plugin sync available; nothing synced (use --install-nvim-packer)"
            );
            return Ok(());
        }

        let mut failures = Vec::new();
        self.sync_plugins(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &NVIM_PACKER_PLATFORMS
    }

    /// packer.nvim manages user-level config, so it must never run as root.
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
