//! The rbenv step: `rbenv update` to update rbenv and ruby-build, plus a
//! read-only report of newer Ruby versions available without installing
//! them.
//!
//! The step runs `rbenv update` whenever `rbenv` is available. It is skipped
//! entirely when rbenv is absent, so it is safe to register on every machine.
//! rbenv manages user-level Ruby versions, so it must never run as root.
//!
//! The read-only `check()` runs `rbenv global` to learn the active Ruby
//! version and `rbenv install --list` to learn the available stable releases,
//! reporting a newer version when one exists. Nothing is ever installed: the
//! active Ruby version is left untouched.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The rbenv Ruby version manager step.
pub const RBENV: StepId = "rbenv";

/// rbenv runs on every platform freshen supports.
const RBENV_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the rbenv step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(RbenvStep)
}

struct RbenvStep;

impl RbenvStep {
    /// Runs `rbenv update` to update rbenv and its plugins, including
    /// ruby-build.
    ///
    /// Best effort: a failure prints a warning and never fails the rbenv step.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("rbenv: update");
        if let Err(error) = CommandSpec::new("rbenv")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(RBENV, &error));
        }
    }
}

impl Step for RbenvStep {
    fn id(&self) -> StepId {
        RBENV
    }

    fn display_name(&self) -> &str {
        "rbenv"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_rbenv())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let mut items = Vec::new();
        if let Some(item) = ruby_outdated_item() {
            items.push(item);
        }
        Ok(CheckReport::from_items(items))
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &RBENV_PLATFORMS
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

/// Whether the `rbenv` binary is discoverable on `PATH`.
fn detect_rbenv() -> Detection {
    if crate::executable::command_exists("rbenv") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// The active Ruby version from `rbenv global`, when one is set.
///
/// Returns `None` when rbenv reports `system` (no Ruby managed by rbenv is
/// active), in which case there is nothing to compare against.
fn active_ruby_version() -> Option<String> {
    let Ok(output) = CommandSpec::new("rbenv")
        .args(["global"])
        .neutral_locale()
        .run()
    else {
        return None;
    };
    let version = output.stdout().split_whitespace().next()?;
    if version == "system" {
        return None;
    }
    (!version.is_empty()).then_some(version.to_owned())
}

/// Whether `line` is a stable CRuby release such as `3.2.0`.
///
/// Pre-releases and other implementations (`jruby-9.4.0`,
/// `truffleruby-22.1.0`) are ignored.
fn looks_like_stable_release(line: &str) -> bool {
    let mut parts = line.split('.');
    let Some(major) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return false;
    };
    let Some(_minor) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return false;
    };
    let Some(_patch) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return false;
    };
    parts.next().is_none() && major > 0
}

/// The latest stable Ruby release available from `rbenv install --list`.
///
/// Best effort: output without any stable release yields `None`.
#[must_use]
fn latest_ruby_version(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| looks_like_stable_release(line))
        .max_by_key(|version| version_key(version))
        .map(str::to_owned)
}

/// A sortable key for a version string like `3.2.0`.
#[must_use]
fn version_key(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// Whether a newer stable Ruby version exists than the active one, without
/// installing anything.
///
/// Runs `rbenv global` to learn the active version and `rbenv install --list`
/// to learn the available releases. When the latest available release is newer
/// than the active version, returns an outdated item naming it. Best effort:
/// a missing `rbenv`, no active version or unparseable output yields `None`
/// and never fails the check.
fn ruby_outdated_item() -> Option<OutdatedItem> {
    let active = active_ruby_version()?;
    let active_key = version_key(&active);

    let Ok(output) = CommandSpec::new("rbenv")
        .args(["install", "--list"])
        .neutral_locale()
        .run()
    else {
        return None;
    };

    let latest = latest_ruby_version(output.stdout())?;
    let latest_key = version_key(&latest);

    (latest_key > active_key).then_some(
        OutdatedItem::new("ruby")
            .with_current(active)
            .with_latest(latest)
            .with_reason("newer Ruby version available"),
    )
}

#[cfg(test)]
mod tests {
    use super::{latest_ruby_version, looks_like_stable_release, version_key};

    #[test]
    fn version_key_parses_major_minor_patch() {
        assert_eq!(version_key("3.2.0"), (3, 2, 0));
        assert_eq!(version_key("3.3.1"), (3, 3, 1));
        assert_eq!(version_key("3.2"), (3, 2, 0));
        assert_eq!(version_key("not-a-version"), (0, 0, 0));
    }

    #[test]
    fn looks_like_stable_release_accepts_three_components() {
        assert!(looks_like_stable_release("3.2.0"));
        assert!(looks_like_stable_release("3.3.1"));
        assert!(!looks_like_stable_release("jruby-9.4.0"));
        assert!(!looks_like_stable_release("truffleruby-22.1.0"));
        assert!(!looks_like_stable_release("3.2"));
        assert!(!looks_like_stable_release(""));
    }

    #[test]
    fn latest_ruby_version_picks_the_highest_stable_release() {
        let output = "  3.2.0\n  3.2.1\n  3.3.0\n  jruby-9.4.0.0\n  truffleruby-22.1.0\n";
        assert_eq!(latest_ruby_version(output), Some("3.3.0".to_owned()));
    }

    #[test]
    fn latest_ruby_version_ignores_non_releases() {
        assert_eq!(
            latest_ruby_version("jruby-9.4.0\ntruffleruby-22.1.0\n"),
            None
        );
        assert_eq!(latest_ruby_version(""), None);
    }
}
