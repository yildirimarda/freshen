//! The volta step: report when a newer Node LTS exists than the one in use,
//! and update volta itself only when it is not managed by brew or mise.
//!
//! volta has no `self-update` command, so upgrading volta itself means
//! re-running its install script. That is only attempted when neither
//! Homebrew nor mise owns the Node toolchain, since those managers keep
//! volta/current node current themselves.

use crate::executable::command_exists;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The volta Node version manager step.
pub const VOLTA: StepId = "volta";

/// volta runs on every platform freshen supports.
const VOLTA_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the volta step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(VoltaStep)
}

struct VoltaStep;

impl VoltaStep {
    /// Whether Homebrew or mise owns the Node toolchain, in which case
    /// `volta` itself is kept current by that manager and is not self-updated.
    fn volta_is_managed() -> bool {
        volta_installed_with_brew() || volta_managed_by_mise()
    }

    /// Upgrades volta itself by re-running its install script.
    ///
    /// Best effort: a failure prints a warning and never fails the volta step.
    ///
    /// The install script is a Unix shell script, so self-update is only
    /// attempted on Unix. On Windows volta is maintained by a different
    /// toolchain and is not self-updated here.
    #[cfg(unix)]
    fn update_volta_itself(&self, failures: &mut Vec<StepError>) {
        human!("volta: self-update");
        let script = format!("curl -fsSL {} | bash", VOLTA_INSTALL_SCRIPT);
        if let Err(error) = CommandSpec::new("sh")
            .arg("-c")
            .arg(script)
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(VOLTA, &error));
            human!("warning: volta self-update skipped/failed: {error}");
        }
    }

    /// volta's self-update re-runs the Unix install script, which is not
    /// applicable on Windows, so there is nothing to do here.
    #[cfg(not(unix))]
    fn update_volta_itself(&self, _failures: &mut Vec<StepError>) {
        human!("warning: volta self-update skipped: not supported on this platform");
    }
}

impl Step for VoltaStep {
    fn id(&self) -> StepId {
        VOLTA
    }

    fn display_name(&self) -> &str {
        "volta"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_volta())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let mut items = Vec::new();
        if let Some(item) = node_lts_outdated_item() {
            items.push(item);
        }
        Ok(CheckReport::from_items(items))
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        if Self::volta_is_managed() {
            human!("warning: volta self-update skipped: managed by brew or mise");
        } else {
            self.update_volta_itself(&mut failures);
        }
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &VOLTA_PLATFORMS
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

/// Whether the `volta` binary is discoverable on `PATH`.
fn detect_volta() -> Detection {
    if command_exists("volta") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Whether Homebrew owns the volta installation (`brew list --formula volta`).
fn volta_installed_with_brew() -> bool {
    command_exists("brew")
        && CommandSpec::new("brew")
            .args(["list", "--formula", "volta"])
            .run()
            .is_ok()
}

/// The active Node version from `volta current node`, when node is in use.
fn active_volta_node_version() -> Option<String> {
    let Ok(output) = CommandSpec::new("volta")
        .args(["current", "node"])
        .neutral_locale()
        .run()
    else {
        return None;
    };
    let version = output.stdout().trim().to_owned();
    let version = version.strip_prefix('v').unwrap_or(&version);
    (!version.is_empty() && version != "none").then_some(version.to_owned())
}

/// Whether mise manages Node (`mise current node` returns a version), in
/// which case mise is the Node toolchain manager and volta is not self-updated.
fn volta_managed_by_mise() -> bool {
    command_exists("mise") && active_mise_node_version().is_some_and(|version| !version.is_empty())
}

/// The active Node version from `mise current node`, when mise manages node.
fn active_mise_node_version() -> Option<String> {
    let Ok(output) = CommandSpec::new("mise")
        .args(["current", "node"])
        .neutral_locale()
        .run()
    else {
        return None;
    };
    let version = output.stdout().trim().to_owned();
    let version = version.strip_prefix('v').unwrap_or(&version);
    (!version.is_empty() && version != "none").then_some(version.to_owned())
}

/// Whether a newer Node LTS major exists than the one in use, without
/// switching to it.
///
/// Runs `volta current node` to learn the active version and `volta list` to
/// list available Node versions. When the newest LTS major is higher than the
/// active major, returns an outdated item naming the latest version. Best
/// effort: a missing `volta`, an absent node install or unparseable output
/// yields `None` and never fails the check.
fn node_lts_outdated_item() -> Option<OutdatedItem> {
    let active = active_volta_node_version()?;
    let active_major = major_version(&active)?;

    let Ok(output) = CommandSpec::new("volta")
        .args(["list"])
        .neutral_locale()
        .retry_transient_network(NETWORK_RETRIES)
        .run()
    else {
        return None;
    };

    let latest = latest_lts_version(output.stdout())?;
    let latest_major = major_version(&latest)?;

    (latest_major > active_major).then_some(
        OutdatedItem::new("node")
            .with_current(active)
            .with_latest(latest)
            .with_reason("newer Node LTS major available"),
    )
}

/// The leading numeric component of a version string, e.g. `20` from `20.18.0`
/// or `v20.18.0`.
#[must_use]
fn major_version(version: &str) -> Option<u64> {
    version
        .strip_prefix('v')
        .unwrap_or(version)
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// The latest version from `volta list` output.
#[must_use]
fn latest_lts_version(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .max_by_key(|line| version_key(line))
        .map(|line| line.strip_prefix('v').unwrap_or(line).to_owned())
}

/// A sortable key for a version string like `20.18.0`.
#[must_use]
fn version_key(version: &str) -> (u64, u64, u64) {
    let mut parts = version.strip_prefix('v').unwrap_or(version).split('.');
    let major = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// The volta install script, used to upgrade volta itself when neither
/// Homebrew nor mise owns the installation. The script is a Unix shell script,
/// so it is only referenced on Unix platforms.
#[cfg(unix)]
const VOLTA_INSTALL_SCRIPT: &str =
    "https://raw.githubusercontent.com/volta-cli/volta/master/install.sh";

#[cfg(test)]
mod tests {
    use super::{latest_lts_version, major_version, version_key};

    #[test]
    fn major_version_parses_major_minor_patch() {
        assert_eq!(major_version("20.18.0"), Some(20));
        assert_eq!(major_version("v20.18.0"), Some(20));
        assert_eq!(major_version("none"), None);
        assert_eq!(major_version(""), None);
    }

    #[test]
    fn version_key_parses_major_minor_patch() {
        assert_eq!(version_key("20.18.0"), (20, 18, 0));
        assert_eq!(version_key("v22.12.0"), (22, 12, 0));
        assert_eq!(version_key("not-a-version"), (0, 0, 0));
    }

    #[test]
    fn latest_lts_version_picks_the_highest_lts() {
        assert_eq!(
            latest_lts_version("18.20.0\n20.18.0\n22.12.0\n"),
            Some("22.12.0".to_owned())
        );
        assert_eq!(
            latest_lts_version("v18.20.0\nv20.18.0\nv22.12.0\n"),
            Some("22.12.0".to_owned())
        );
        assert_eq!(latest_lts_version(""), None);
        assert_eq!(latest_lts_version("\n\n"), None);
    }
}
