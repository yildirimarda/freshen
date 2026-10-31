//! The pacman step: `pacman -Syu --noconfirm`, skipped when pacman is absent.
//!
//! The step runs `pacman -Syu --noconfirm` whenever `pacman` is available.
//! With `--cleanup` it also cleans the package cache (`pacman -Sc`) and
//! removes orphaned packages (`pacman -Qtdq` then `pacman -Rns`). It is
//! skipped entirely when pacman is absent, so it is safe to register on
//! every machine; on macOS and Windows it is never detected.

use crate::cleanup;
use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::pacman_findings;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The pacman package manager step.
pub const PACMAN: StepId = "pacman";

/// pacman only runs on Linux.
const PACMAN_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the pacman step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(PacmanStep)
}

struct PacmanStep;

impl PacmanStep {
    /// Runs `pacman -Syu --noconfirm`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("pacman -Syu --noconfirm");
        if let Err(error) = CommandSpec::new("pacman")
            .args(["-Syu", "--noconfirm"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(PACMAN, &error));
        }
    }

    /// Cleans the package cache and removes orphaned packages, running only
    /// when `--cleanup` is enabled.
    fn cleanup(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        self.clean_package_cache(options, failures);
        self.remove_orphans(options, failures);
    }

    /// Removes cached packages that are no longer installed and unused sync
    /// databases.
    fn clean_package_cache(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        if let Err(error) = cleanup::run(
            PACMAN,
            "pacman: package cache",
            CommandSpec::new("pacman")
                .args(["-Sc", "--noconfirm"])
                .retry_transient_network(NETWORK_RETRIES),
            options,
        ) {
            failures.push(error);
        }
    }

    /// Removes orphaned packages (packages no longer required by anything).
    fn remove_orphans(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        if !options.cleanup_enabled() {
            human!(
                "pacman: orphans: skipped destructive cleanup (pass {} to enable)",
                cleanup::ENABLE_FLAG
            );
            return;
        }

        let Ok(listing) = CommandSpec::new("pacman").args(["-Qtdq"]).run() else {
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
            CommandSpec::new("pacman")
                .args(["-Rns", "--noconfirm"])
                .args(orphans.as_slice())
                .retry_transient_network(NETWORK_RETRIES),
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
        Ok(detect_pacman())
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

    fn platforms(&self) -> &'static [Platform] {
        &PACMAN_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `pacman` binary is discoverable on `PATH`.
fn detect_pacman() -> Detection {
    if command_exists("pacman") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}
