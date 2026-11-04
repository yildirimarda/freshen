//! The zypper step: `zypper refresh` followed by `zypper update -y`, skipped
//! when zypper is absent.
//!
//! The step runs `zypper refresh` and then `zypper update -y` whenever `zypper`
//! is available, collecting the first failure so both commands are attempted.
//! It is skipped entirely when zypper is absent, so it is safe to register on
//! every machine; on macOS and Windows it is never detected.

#[cfg(target_os = "linux")]
use crate::containers::{ContainerRuntime, NativeDockerRuntime};
use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::services_needing_restart;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
use crate::wsl::inside_command;

/// The zypper package manager step.
pub const ZYPPER: StepId = "zypper";

/// zypper runs on Linux natively and, on Windows, inside a selected WSL
/// distribution via `wsl -d <distro> zypper ...`.
const ZYPPER_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// Docker packages whose upgrade requires the Docker Engine to be restarted.
#[cfg(target_os = "linux")]
const DOCKER_PACKAGES: [&str; 1] = ["docker"];

/// Registers the zypper step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ZypperStep)
}

struct ZypperStep;

impl ZypperStep {
    /// Runs `zypper refresh` and then `zypper update -y`, collecting the
    /// first failure so both commands are attempted even when refresh fails.
    /// On Windows the commands run inside the selected WSL distribution via
    /// `wsl -d <distro>`.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn upgrade(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        human!("zypper refresh");
        if let Err(error) = inside_command(
            options,
            CommandSpec::new("zypper")
                .args(["refresh"])
                .retry_transient_network(NETWORK_RETRIES),
        )
        .run()
        {
            failures.push(StepError::run_from_process(ZYPPER, &error));
        }

        human!("zypper update -y");
        if let Err(error) = inside_command(
            options,
            CommandSpec::new("zypper")
                .args(["update", "-y"])
                .retry_transient_network(NETWORK_RETRIES),
        )
        .run()
        {
            failures.push(StepError::run_from_process(ZYPPER, &error));
        }
    }

    /// Whether `docker` has a pending upgrade, as reported by `zypper
    /// list-updates`.
    #[cfg(target_os = "linux")]
    fn docker_packages_are_pending() -> bool {
        let Ok(output) = CommandSpec::new("zypper")
            .args(["list-updates"])
            .neutral_locale()
            .run()
        else {
            return false;
        };
        output.stdout().lines().any(|line| {
            DOCKER_PACKAGES
                .iter()
                .any(|pkg| line.split_whitespace().any(|field| field == *pkg))
        })
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
        #[cfg(target_os = "linux")]
        return Ok(detect_zypper());
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
                human!("zypper: no WSL distribution selected; pass --wsl-distro <distro>");
                return Ok(());
            };
            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);
            if failures.is_empty() {
                self.services_needing_restart();
            }
            finish(failures)
        }

        #[cfg(target_os = "linux")]
        {
            let docker_upgrading = {
                let engine = NativeDockerRuntime::new();
                engine.detect().is_installed()
                    && Self::docker_packages_are_pending()
                    && engine.is_running()
            };

            if docker_upgrading {
                if let Err(error) = NativeDockerRuntime::new().stop() {
                    return Err(StepError::run_from_process(ZYPPER, &error));
                }
            }

            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);

            if docker_upgrading {
                if let Err(error) = NativeDockerRuntime::new().start() {
                    failures.push(StepError::run_from_process(ZYPPER, &error));
                }
            }

            if failures.is_empty() {
                self.services_needing_restart();
            }
            finish(failures)
        }

        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            // zypper is not available on macOS; nothing to run.
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &ZYPPER_PLATFORMS
    }
}

impl ZypperStep {
    /// Runs `zypper ps -s` after a successful upgrade and records the services
    /// that need a restart. A failure to list services does not fail the step:
    /// the upgrade itself succeeded.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
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

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `zypper` binary is discoverable on `PATH`.
#[cfg(target_os = "linux")]
fn detect_zypper() -> Detection {
    if command_exists("zypper") {
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

/// Parses the output of `zypper ps -s` into the names of services that need a
/// restart.
///
/// `zypper ps -s` prints one service per line (a systemd unit name such as
/// `ssh.service`). Header and separator lines are ignored; every remaining
/// non-empty line is treated as a service name.
#[cfg(any(target_os = "linux", target_os = "windows"))]
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
