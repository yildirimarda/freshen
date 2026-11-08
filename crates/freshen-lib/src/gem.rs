//! The gem step: `gem update --system`, skipped when Ruby is managed by a
//! system package manager.
//!
//! The step runs `gem update --system` whenever `gem` is available. It is
//! skipped entirely when `gem` is absent, so it is safe to register on every
//! machine. RubyGems installs into the user's Ruby environment, so it must
//! never run as root.
//!
//! When Ruby itself is managed by a system package manager (Homebrew, apt,
//! dnf, pacman, winget, Chocolatey or Scoop), `gem update --system` would
//! fight that manager over the RubyGems installation, so the step skips it.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The RubyGems step.
pub const GEM: StepId = "gem";

/// gem runs on every platform freshen supports.
const GEM_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the gem step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(GemStep)
}

struct GemStep;

impl GemStep {
    /// Upgrades RubyGems itself, unless Ruby is managed by a system package
    /// manager.
    fn update(&self, failures: &mut Vec<StepError>) {
        if gem_installed_with_system_package_manager() {
            return;
        }
        human!("gem: update --system");
        if let Err(error) = CommandSpec::new("gem")
            .args(["update", "--system"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(GEM, &error));
        }
    }
}

impl Step for GemStep {
    fn id(&self) -> StepId {
        GEM
    }

    fn display_name(&self) -> &str {
        "gem"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_gem())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // RubyGems has no read-only "what is outdated" query for the system,
        // so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &GEM_PLATFORMS
    }

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

/// Whether the `gem` binary is discoverable on `PATH`.
fn detect_gem() -> Detection {
    if crate::executable::command_exists("gem") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Whether Ruby is owned by a system package manager, in which case that
/// manager's own upgrade keeps RubyGems current and `gem update --system` is
/// skipped so the two do not fight over the Ruby installation. A missing probe
/// command is treated as "that manager is not involved", so only managers
/// actually present on `PATH` are consulted.
fn gem_installed_with_system_package_manager() -> bool {
    gem_installed_with_brew()
        || gem_installed_with_dpkg()
        || gem_installed_with_dnf()
        || gem_installed_with_pacman()
        || gem_installed_with_winget()
        || gem_installed_with_choco()
        || gem_installed_with_scoop()
}

/// Whether Homebrew owns the Ruby installation (`brew list --formula ruby`).
fn gem_installed_with_brew() -> bool {
    command_exists("brew")
        && CommandSpec::new("brew")
            .args(["list", "--formula", "ruby"])
            .run()
            .is_ok()
}

/// Whether dpkg (and therefore apt on Debian/Ubuntu) owns the Ruby installation.
fn gem_installed_with_dpkg() -> bool {
    command_exists("dpkg") && CommandSpec::new("dpkg").args(["-s", "ruby"]).run().is_ok()
}

/// Whether dnf owns the Ruby installation.
fn gem_installed_with_dnf() -> bool {
    command_exists("dnf")
        && CommandSpec::new("dnf")
            .args(["list", "installed", "ruby"])
            .run()
            .is_ok()
}

/// Whether pacman owns the Ruby installation.
fn gem_installed_with_pacman() -> bool {
    command_exists("pacman")
        && CommandSpec::new("pacman")
            .args(["-Q", "ruby"])
            .run()
            .is_ok()
}

/// Whether winget owns the Ruby installation.
fn gem_installed_with_winget() -> bool {
    command_exists("winget")
        && CommandSpec::new("winget")
            .args(["list", "ruby"])
            .run()
            .is_ok()
}

/// Whether Chocolatey owns the Ruby installation.
fn gem_installed_with_choco() -> bool {
    command_exists("choco")
        && CommandSpec::new("choco")
            .args(["list", "--local-only", "ruby"])
            .run()
            .is_ok()
}

/// Whether Scoop owns the Ruby installation.
fn gem_installed_with_scoop() -> bool {
    command_exists("scoop")
        && CommandSpec::new("scoop")
            .args(["list", "ruby"])
            .run()
            .is_ok()
}

fn command_exists(program: &str) -> bool {
    crate::executable::command_exists(program)
}
