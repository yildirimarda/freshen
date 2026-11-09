//! The Mason step: `nvim --headless "+MasonUpdate" +qa`, skipped when nvim
//! is absent.
//!
//! Updating Mason packages is opt-in: without `--install-nvim-mason` the step
//! reports that the update is available but changes nothing. With the flag the
//! step runs `nvim --headless "+MasonUpdate" +qa` to update every Mason
//! package. There is no read-only way to tell whether Mason packages are out
//! of date, so `check()` always reports the step up to date once `nvim` is
//! detected.
//!
//! The step runs on every platform freshen supports, because `nvim` exists on
//! macOS, Linux and Windows. Mason manages user-level config, so it must
//! never run as root.

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Mason package update step.
pub const NVIM_MASON: StepId = "nvim-mason";

/// nvim runs on every platform freshen supports.
const NVIM_MASON_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the Mason step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(NvimMasonStep)
}

/// The Mason package update step.
struct NvimMasonStep;

impl NvimMasonStep {
    /// Whether the opt-in flag that permits `nvim --headless "+MasonUpdate"
    /// +qa` is set.
    fn sync_enabled(options: &RunOptions) -> bool {
        options.install_nvim_mason()
    }

    /// Runs `nvim --headless "+MasonUpdate" +qa`.
    fn update_packages(&self, failures: &mut Vec<StepError>) {
        human!("nvim-mason: update packages");
        if let Err(error) = CommandSpec::new("nvim")
            .args(["--headless", "+MasonUpdate", "+qa"])
            .run()
        {
            failures.push(StepError::run_from_process(NVIM_MASON, &error));
        }
    }
}

impl Step for NvimMasonStep {
    fn id(&self) -> StepId {
        NVIM_MASON
    }

    fn display_name(&self) -> &str {
        "Mason"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_nvim())
    }

    /// Reports the step up to date: there is no read-only query that tells
    /// whether Mason packages need updating, so the step cannot report pending
    /// work without changing anything. The listing is left to `--list-steps`.
    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    /// Runs `nvim --headless "+MasonUpdate" +qa` only when the opt-in flag is
    /// set. Without it the step reports that the update is available and
    /// changes nothing.
    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if !Self::sync_enabled(options) {
            human!(
                "nvim-mason: package update available; nothing updated (use --install-nvim-mason)"
            );
            return Ok(());
        }

        let mut failures = Vec::new();
        self.update_packages(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &NVIM_MASON_PLATFORMS
    }

    /// Mason manages user-level config, so it must never run as root.
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
