//! The pyenv step: `pyenv update` to update pyenv and its plugins, plus a
//! read-only report of newer Python versions available without installing
//! them.
//!
//! The step runs `pyenv update` whenever `pyenv` is available. It is skipped
//! entirely when pyenv is absent, so it is safe to register on every machine.
//! pyenv manages user-level Python versions, so it must never run as root.
//!
//! The read-only `check()` runs `pyenv global` to learn the active Python
//! version and `pyenv install --list` to learn the available stable releases,
//! reporting a newer version when one exists. Nothing is ever installed: the
//! active Python version is left untouched.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The pyenv Python version manager step.
pub const PYENV: StepId = "pyenv";

/// pyenv runs on every platform freshen supports.
const PYENV_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the pyenv step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(PyenvStep)
}

struct PyenvStep;

impl PyenvStep {
    /// Runs `pyenv update` to update pyenv and its plugins.
    ///
    /// Best effort: a failure prints a warning and never fails the pyenv step.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("pyenv: update");
        if let Err(error) = CommandSpec::new("pyenv")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(PYENV, &error));
        }
    }
}

impl Step for PyenvStep {
    fn id(&self) -> StepId {
        PYENV
    }

    fn display_name(&self) -> &str {
        "pyenv"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_pyenv())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let mut items = Vec::new();
        if let Some(item) = python_outdated_item() {
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
        &PYENV_PLATFORMS
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

/// Whether the `pyenv` binary is discoverable on `PATH`.
fn detect_pyenv() -> Detection {
    if crate::executable::command_exists("pyenv") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// The active Python version from `pyenv global`, when one is set.
///
/// Returns `None` when pyenv reports `system` (no Python managed by pyenv is
/// active), in which case there is nothing to compare against.
fn active_python_version() -> Option<String> {
    let Ok(output) = CommandSpec::new("pyenv")
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

/// Whether `line` is a stable CPython release such as `3.12.0`.
///
/// Pre-releases (`3.14.0-dev`, `3.13.0-rc1`) and other implementations
/// (`pypy3.10-7.3.13`) are ignored.
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

/// The latest stable Python release available from `pyenv install --list`.
///
/// Best effort: output without any stable release yields `None`.
#[must_use]
fn latest_python_version(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| looks_like_stable_release(line))
        .max_by_key(|version| version_key(version))
        .map(str::to_owned)
}

/// A sortable key for a version string like `3.12.0`.
#[must_use]
fn version_key(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// Whether a newer stable Python version exists than the active one, without
/// installing anything.
///
/// Runs `pyenv global` to learn the active version and `pyenv install --list`
/// to learn the available releases. When the latest available release is newer
/// than the active version, returns an outdated item naming it. Best effort:
/// a missing `pyenv`, no active version or unparseable output yields `None`
/// and never fails the check.
fn python_outdated_item() -> Option<OutdatedItem> {
    let active = active_python_version()?;
    let active_key = version_key(&active);

    let Ok(output) = CommandSpec::new("pyenv")
        .args(["install", "--list"])
        .neutral_locale()
        .run()
    else {
        return None;
    };

    let latest = latest_python_version(output.stdout())?;
    let latest_key = version_key(&latest);

    (latest_key > active_key).then_some(
        OutdatedItem::new("python")
            .with_current(active)
            .with_latest(latest)
            .with_reason("newer Python version available"),
    )
}

#[cfg(test)]
mod tests {
    use super::{latest_python_version, looks_like_stable_release, version_key};

    #[test]
    fn version_key_parses_major_minor_patch() {
        assert_eq!(version_key("3.12.0"), (3, 12, 0));
        assert_eq!(version_key("3.13.1"), (3, 13, 1));
        assert_eq!(version_key("3.12"), (3, 12, 0));
        assert_eq!(version_key("not-a-version"), (0, 0, 0));
    }

    #[test]
    fn looks_like_stable_release_accepts_three_components() {
        assert!(looks_like_stable_release("3.12.0"));
        assert!(looks_like_stable_release("3.13.1"));
        assert!(!looks_like_stable_release("3.14.0-dev"));
        assert!(!looks_like_stable_release("pypy3.10-7.3.13"));
        assert!(!looks_like_stable_release("3.12"));
        assert!(!looks_like_stable_release(""));
    }

    #[test]
    fn latest_python_version_picks_the_highest_stable_release() {
        let output = "  3.12.0\n  3.12.1\n  3.13.0\n  3.14.0-dev\n  pypy3.10-7.3.13\n";
        assert_eq!(latest_python_version(output), Some("3.13.0".to_owned()));
    }

    #[test]
    fn latest_python_version_ignores_non_releases() {
        assert_eq!(latest_python_version("3.14.0-dev\npypy3.10-7.3.13\n"), None);
        assert_eq!(latest_python_version(""), None);
    }
}
