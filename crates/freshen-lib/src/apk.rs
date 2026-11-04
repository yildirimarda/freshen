//! The apk step: `apk update` followed by `apk upgrade`, skipped when apk is
//! absent.
//!
//! The step runs `apk update` and then `apk upgrade` whenever `apk` is
//! available, collecting the first failure so both commands are attempted.
//! It is skipped entirely when apk is absent, so it is safe to register on
//! every machine; on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
use crate::wsl::inside_command;

/// The apk package manager step (Alpine Linux).
pub const APK: StepId = "apk";

/// apk runs on Linux natively and, on Windows, inside a selected WSL
/// distribution via `wsl -d <distro> apk ...`.
const APK_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// Registers the apk step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ApkStep)
}

struct ApkStep;

impl ApkStep {
    /// Runs `apk update` and then `apk upgrade`, collecting the first failure
    /// so both commands are attempted even when update fails. On Windows the
    /// commands run inside the selected WSL distribution via `wsl -d <distro>`.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn upgrade(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        human!("apk update");
        if let Err(error) = inside_command(
            options,
            CommandSpec::new("apk")
                .args(["update"])
                .retry_transient_network(NETWORK_RETRIES),
        )
        .run()
        {
            failures.push(StepError::run_from_process(APK, &error));
        }

        human!("apk upgrade");
        if let Err(error) = inside_command(
            options,
            CommandSpec::new("apk")
                .args(["upgrade"])
                .retry_transient_network(NETWORK_RETRIES),
        )
        .run()
        {
            failures.push(StepError::run_from_process(APK, &error));
        }
    }
}

impl Step for ApkStep {
    fn id(&self) -> StepId {
        APK
    }

    fn display_name(&self) -> &str {
        "apk"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        #[cfg(target_os = "linux")]
        return Ok(detect_apk());
        #[cfg(target_os = "windows")]
        return Ok(detect_wsl());
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        return Ok(Detection::NotInstalled);
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        #[cfg(target_os = "windows")]
        {
            let Some(_distro) = crate::wsl::selected_distro(options) else {
                human!("apk: no WSL distribution selected; pass --wsl-distro <distro>");
                return Ok(());
            };
            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);
            finish(failures)
        }

        #[cfg(target_os = "linux")]
        {
            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);
            finish(failures)
        }

        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            // apk is not available on macOS; nothing to run.
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &APK_PLATFORMS
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `apk` binary is discoverable on `PATH`.
#[cfg(target_os = "linux")]
fn detect_apk() -> Detection {
    if command_exists("apk") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Whether the `wsl` binary is discoverable on `PATH` (Windows only).
#[cfg(target_os = "windows")]
fn detect_wsl() -> Detection {
    if command_exists("wsl") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}
