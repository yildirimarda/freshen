//! The Nix steps: `nix channel --update`, `nix profile upgrade --all` and
//! `home-manager switch`.
//!
//! The channel step runs `nix channel --update` whenever `nix` is available.
//! The profile step runs `nix profile upgrade --all` whenever `nix` is
//! available. Both are skipped entirely when nix is absent, so they are safe
//! to register on every machine; on macOS and Windows they are never detected.
//!
//! The home-manager step runs `home-manager switch` only when a home-manager
//! configuration file is detected, so it is opt-in by nature.

use std::env;
use std::path::{Path, PathBuf};

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Nix channel step.
pub const NIX: StepId = "nix";
/// The Nix profile upgrade step.
pub const NIX_PROFILE: StepId = "nix-profile";
/// The home-manager step.
pub const HOME_MANAGER: StepId = "home-manager";

/// Nix only runs on Linux.
const NIX_PLATFORMS: [Platform; 1] = [Platform::Linux];
/// Nix profile upgrades only run on Linux.
const NIX_PROFILE_PLATFORMS: [Platform; 1] = [Platform::Linux];
/// home-manager runs on Linux and macOS.
const HOME_MANAGER_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Macos];

/// The home-manager configuration file, relative to the config home
/// (`$XDG_CONFIG_HOME/nixpkgs/home.nix`, else `~/.config/nixpkgs/home.nix`).
const HOME_MANAGER_CONFIG: &str = "nixpkgs/home.nix";

/// Registers the nix steps: channel update, profile upgrade, then home-manager.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry
        .with(NixStep)
        .with(NixProfileStep)
        .with(HomeManagerStep)
}

struct NixStep;

impl NixStep {
    /// Runs `nix channel --update`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("nix channel --update");
        if let Err(error) = CommandSpec::new("nix")
            .args(["channel", "--update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(NIX, &error));
        }
    }
}

impl Step for NixStep {
    fn id(&self) -> StepId {
        NIX
    }

    fn display_name(&self) -> &str {
        "nix"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_nix())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &NIX_PLATFORMS
    }
}

struct NixProfileStep;

impl NixProfileStep {
    /// Runs `nix profile upgrade --all`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("nix profile upgrade --all");
        if let Err(error) = CommandSpec::new("nix")
            .args(["profile", "upgrade", "--all"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(NIX_PROFILE, &error));
        }
    }
}

impl Step for NixProfileStep {
    fn id(&self) -> StepId {
        NIX_PROFILE
    }

    fn display_name(&self) -> &str {
        "nix profile"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_nix())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &NIX_PROFILE_PLATFORMS
    }
}

/// The home-manager step: `home-manager switch`.
///
/// It runs only when a home-manager configuration file is detected, so it is
/// opt-in by nature. Detection looks for
/// `$XDG_CONFIG_HOME/nixpkgs/home.nix` (else `~/.config/nixpkgs/home.nix`).
struct HomeManagerStep;

impl HomeManagerStep {
    /// Runs `home-manager switch`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("home-manager switch");
        if let Err(error) = CommandSpec::new("home-manager").args(["switch"]).run() {
            failures.push(StepError::run_from_process(HOME_MANAGER, &error));
        }
    }
}

impl Step for HomeManagerStep {
    fn id(&self) -> StepId {
        HOME_MANAGER
    }

    fn display_name(&self) -> &str {
        "home-manager"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_home_manager(&home_manager_config_path()))
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOME_MANAGER_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `nix` binary is discoverable on `PATH`.
fn detect_nix() -> Detection {
    if command_exists("nix") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// The path to the home-manager configuration file.
///
/// Honours `XDG_CONFIG_HOME` and falls back to `~/.config` (or
/// `%APPDATA%` on Windows).
fn home_manager_config_path() -> PathBuf {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .or_else(|| env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .unwrap_or_else(env::temp_dir);
    base.join(HOME_MANAGER_CONFIG)
}

/// Whether a home-manager configuration exists at `path`.
fn home_manager_config_exists(path: &Path) -> bool {
    path.is_file()
}

/// Detects home-manager from its configuration file at `path`.
///
/// Returns [`Detection::Installed`] when the configuration file is present,
/// otherwise [`Detection::NotInstalled`]. The path is a parameter so tests can
/// point it at a temporary location instead of the real config home.
fn detect_home_manager(path: &Path) -> Detection {
    if home_manager_config_exists(path) {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_home_manager, detect_nix, home_manager_config_exists};
    use crate::step::Detection;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn detect_nix_reports_not_installed_when_absent_from_path() {
        // `nix` is not expected on the test machine; this just exercises the
        // detection function against a name that is absent.
        assert!(!detect_nix().is_installed());
    }

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }

    /// A unique temp config directory for the home-manager detection tests, so
    /// parallel tests do not collide and never touch the real config home.
    fn temp_config_dir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-home-manager-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
        dir.join(".config").join("nixpkgs")
    }

    #[test]
    fn detect_home_manager_reports_installed_when_config_exists() {
        let config_dir = temp_config_dir();
        fs::create_dir_all(&config_dir)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", config_dir.display()));
        let config = config_dir.join("home.nix");
        fs::write(&config, "{ config, ... }: {}\n")
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", config.display()));

        assert!(detect_home_manager(&config).is_installed());

        let _ = fs::remove_dir_all(
            config_dir
                .parent()
                .and_then(|parent| parent.parent().map(|grand| grand.to_path_buf()))
                .unwrap_or_else(|| panic!("config dir has no grandparent: {config_dir:?}")),
        );
    }

    #[test]
    fn detect_home_manager_reports_not_installed_when_config_absent() {
        let config_dir = temp_config_dir();
        let missing = config_dir.join("home.nix");

        assert!(!detect_home_manager(&missing).is_installed());

        let _ = fs::remove_dir_all(
            config_dir
                .parent()
                .and_then(|parent| parent.parent().map(|grand| grand.to_path_buf()))
                .unwrap_or_else(|| panic!("config dir has no grandparent: {config_dir:?}")),
        );
    }

    #[test]
    fn home_manager_config_exists_reports_file_presence() {
        let config_dir = temp_config_dir();
        fs::create_dir_all(&config_dir)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", config_dir.display()));
        let present = config_dir.join("home.nix");
        fs::write(&present, "{ config, ... }: {}\n")
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", present.display()));
        let absent = config_dir.join("absent.nix");

        assert!(home_manager_config_exists(&present));
        assert!(!home_manager_config_exists(&absent));

        let _ = fs::remove_dir_all(
            config_dir
                .parent()
                .and_then(|parent| parent.parent().map(|grand| grand.to_path_buf()))
                .unwrap_or_else(|| panic!("config dir has no grandparent: {config_dir:?}")),
        );
    }
}
