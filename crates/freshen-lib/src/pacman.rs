//! The pacman step: `pacman -Syu --noconfirm`, skipped when pacman is absent.
//!
//! The step runs `pacman -Syu --noconfirm` whenever `pacman` is available.
//! With `--cleanup` it also cleans the package cache (`pacman -Sc`) and
//! removes orphaned packages (`pacman -Qtdq` then `pacman -Rns`). It is
//! skipped entirely when pacman is absent, so it is safe to register on
//! every machine; on macOS and Windows it is never detected.

use crate::cleanup;
#[cfg(target_os = "linux")]
use crate::containers::{ContainerRuntime, NativeDockerRuntime};
use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
#[cfg(target_os = "linux")]
use crate::pacman_findings;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
use crate::wsl::inside_command;

/// The pacman package manager step.
pub const PACMAN: StepId = "pacman";

/// pacman runs on Linux natively and, on Windows, inside a selected WSL
/// distribution via `wsl -d <distro> pacman ...`.
const PACMAN_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// Docker packages whose upgrade requires the Docker Engine to be restarted.
#[cfg(target_os = "linux")]
const DOCKER_PACKAGES: [&str; 1] = ["docker"];

/// Registers the pacman step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(PacmanStep)
}

struct PacmanStep;

impl PacmanStep {
    /// Runs `pacman -Syu --noconfirm`. On Windows the command runs inside the
    /// selected WSL distribution via `wsl -d <distro> pacman ...`.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn upgrade(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        human!("pacman -Syu --noconfirm");
        if let Err(error) = inside_command(
            options,
            CommandSpec::new("pacman")
                .args(["-Syu", "--noconfirm"])
                .retry_transient_network(NETWORK_RETRIES),
        )
        .run()
        {
            failures.push(StepError::run_from_process(PACMAN, &error));
        }
    }

    /// Cleans the package cache and removes orphaned packages, running only
    /// when `--cleanup` is enabled.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn cleanup(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        self.clean_package_cache(options, failures);
        self.remove_orphans(options, failures);
    }

    /// Whether `docker` has a pending upgrade, as reported by `pacman -Qu`.
    #[cfg(target_os = "linux")]
    fn docker_packages_are_pending() -> bool {
        let Ok(output) = CommandSpec::new("pacman")
            .args(["-Qu"])
            .neutral_locale()
            .run()
        else {
            return false;
        };
        output.stdout().lines().any(|line| {
            let mut fields = line.split_whitespace();
            DOCKER_PACKAGES
                .iter()
                .any(|pkg| fields.next() == Some(*pkg))
        })
    }

    /// Removes cached packages that are no longer installed and unused sync
    /// databases.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn clean_package_cache(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        if let Err(error) = cleanup::run(
            PACMAN,
            "pacman: package cache",
            inside_command(
                options,
                CommandSpec::new("pacman")
                    .args(["-Sc", "--noconfirm"])
                    .retry_transient_network(NETWORK_RETRIES),
            ),
            options,
        ) {
            failures.push(error);
        }
    }

    /// Removes orphaned packages (packages no longer required by anything).
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn remove_orphans(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        if !options.cleanup_enabled() {
            human!(
                "pacman: orphans: skipped destructive cleanup (pass {} to enable)",
                cleanup::ENABLE_FLAG
            );
            return;
        }

        let Ok(listing) = inside_command(options, CommandSpec::new("pacman").args(["-Qtdq"])).run()
        else {
            return;
        };
        let orphans: Vec<&str> = listing
            .stdout()
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        if orphans.is_empty() {
            human!("pacman: orphans: nothing to remove");
            return;
        }

        if let Err(error) = cleanup::run(
            PACMAN,
            "pacman: orphans",
            inside_command(
                options,
                CommandSpec::new("pacman")
                    .args(["-Rns", "--noconfirm"])
                    .args(orphans.as_slice())
                    .retry_transient_network(NETWORK_RETRIES),
            ),
            options,
        ) {
            failures.push(error);
        }
    }
}

impl Step for PacmanStep {
    fn id(&self) -> StepId {
        PACMAN
    }

    fn display_name(&self) -> &str {
        "pacman"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        #[cfg(target_os = "linux")]
        return Ok(detect_pacman());
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
                human!("pacman: no WSL distribution selected; pass --wsl-distro <distro>");
                return Ok(());
            };
            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);
            self.cleanup(options, &mut failures);
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
                    return Err(StepError::run_from_process(PACMAN, &error));
                }
            }

            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);

            if docker_upgrading {
                if let Err(error) = NativeDockerRuntime::new().start() {
                    failures.push(StepError::run_from_process(PACMAN, &error));
                }
            }

            // Report `.pacnew` and `.pacsave` files left behind by a successful
            // upgrade so the user knows which configuration files need merging.
            if failures.is_empty() {
                for finding in pacman_findings::pacman_findings() {
                    pacman_findings::record(finding);
                }
            }

            self.cleanup(options, &mut failures);

            finish(failures)
        }

        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            // pacman is not available on macOS; nothing to run.
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &PACMAN_PLATFORMS
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `pacman` binary is discoverable on `PATH`.
#[cfg(target_os = "linux")]
fn detect_pacman() -> Detection {
    if command_exists("pacman") {
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
