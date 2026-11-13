//! The `freshen self-update` subcommand: upgrades freshen itself through the
//! package manager that installed it.
//!
//! freshen keeps itself current by delegating the upgrade to the package
//! manager that owns its installation. When Homebrew installed freshen
//! (`brew list --formula freshen` succeeds), `brew upgrade freshen` is run.
//! Otherwise self-update reports that it cannot determine how freshen was
//! installed and does nothing.

use std::env;

use crate::output::human;
use crate::process::{CommandSpec, ProcessError};
use thiserror::Error;

/// Why freshen cannot update itself.
#[derive(Debug, Error)]
pub enum SelfUpdateError {
    /// The `brew` executable is not on `PATH`.
    #[error(
        "the `brew` executable is not on `PATH`, so freshen cannot update itself through Homebrew: {source}"
    )]
    BrewMissing {
        /// Underlying OS error from failing to start `brew`.
        #[source]
        source: std::io::Error,
    },
    /// `brew list --formula freshen` failed, so ownership could not be determined.
    #[error(
        "`brew list --formula freshen` failed, so it could not be determined whether Homebrew owns freshen: {0}"
    )]
    BrewListFailed(ProcessError),
    /// freshen is not installed through Homebrew.
    #[error(
        "freshen is not installed through Homebrew, so `freshen self-update` does not know how to upgrade it"
    )]
    NotOwnedByBrew,
    /// `brew upgrade freshen` failed.
    #[error("`brew upgrade freshen` failed: {0}")]
    BrewUpgradeFailed(ProcessError),
}

/// Environment variable that disables the update-available check entirely.
///
/// Set to any non-empty value to skip querying the package manager for
/// freshen's own updates (for example on CI or air-gapped machines).
pub const SKIP_ENV: &str = "FRESHEN_SKIP_UPDATE_CHECK";

/// Returns a hint telling the user a newer version of freshen is available,
/// or `None` when freshen is up to date or the check cannot be performed.
///
/// The check is best effort: when `brew` is unavailable, when the query fails,
/// or when freshen is not listed as outdated, this returns `None`. The hint is
/// purely informational and never blocks the run.
///
/// Setting [`SKIP_ENV`] in the environment disables the check entirely, which
/// is useful for CI or air-gapped machines that must never query the package
/// manager for freshen's own updates.
pub fn update_available_hint() -> Option<String> {
    if env::var_os(SKIP_ENV).is_some() {
        return None;
    }

    let output = match CommandSpec::new("brew")
        .args(["outdated", "--formula", "freshen"])
        .run()
    {
        Ok(output) => output,
        // `brew` is not installed or the query could not be completed: skip
        // the hint instead of failing the run.
        Err(ProcessError::Spawn { .. }) => return None,
        Err(_) => return None,
    };

    let mut available = None;
    for line in output.stdout().lines() {
        let line = line.trim();
        if line.split_whitespace().all(|token| token != "freshen") {
            continue;
        }
        available = available_version(line);
        break;
    }

    let available = available?;
    let current = crate::version();
    Some(format!(
        "freshen {current} is installed and {available} is available; run `freshen self-update` to upgrade"
    ))
}

/// Parses the version that is available from one line of `brew outdated`
/// output.
///
/// `brew outdated` prints lines such as `freshen 0.1.0 < 0.2.0`; the last
/// version-like token on the line is the one that is available. Returns `None`
/// when the line does not contain a parseable version, so the hint is skipped
/// instead of crashing on unexpected output.
fn available_version(line: &str) -> Option<String> {
    let mut last: Option<&str> = None;
    for token in line.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
        if !cleaned.is_empty() && is_version(cleaned) {
            last = Some(cleaned);
        }
    }
    last.map(str::to_owned)
}

fn is_version(token: &str) -> bool {
    let mut parts = token.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    if first.is_empty() || !first.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    parts.all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// Runs `freshen self-update`.
///
/// freshen is upgraded through whatever package manager installed it. When
/// Homebrew owns the installation (`brew list --formula freshen` succeeds),
/// `brew upgrade freshen` is run and its output is streamed live. Otherwise
/// [`SelfUpdateError::NotOwnedByBrew`] is returned.
///
/// # Errors
///
/// Returns [`SelfUpdateError`] when `brew` is unavailable, when Homebrew does
/// not own freshen, or when the upgrade command fails.
pub fn self_update() -> Result<(), SelfUpdateError> {
    let brew_owns = match CommandSpec::new("brew")
        .args(["list", "--formula", "freshen"])
        .run()
    {
        Ok(_) => true,
        Err(ProcessError::Spawn { source, .. }) => {
            return Err(SelfUpdateError::BrewMissing { source })
        }
        Err(ProcessError::Failed { .. }) => false,
        Err(error) => return Err(SelfUpdateError::BrewListFailed(error)),
    };

    if !brew_owns {
        return Err(SelfUpdateError::NotOwnedByBrew);
    }

    human!("freshen: self-update via Homebrew");
    CommandSpec::new("brew")
        .args(["upgrade", "freshen"])
        .run_streaming(|_stream, line| {
            println!("{line}");
        })
        .map_err(SelfUpdateError::BrewUpgradeFailed)?;

    human!("freshen: self-update complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brew_missing_displays_the_missing_executable() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let displayed = SelfUpdateError::BrewMissing { source: error }.to_string();
        assert!(
            displayed.contains("the `brew` executable is not on `PATH`"),
            "{displayed}"
        );
    }

    #[test]
    fn not_owned_by_brew_displays_a_clear_reason() {
        let displayed = SelfUpdateError::NotOwnedByBrew.to_string();
        assert!(
            displayed.contains("not installed through Homebrew"),
            "{displayed}"
        );
    }

    #[test]
    fn available_version_picks_the_last_version_on_the_line() {
        assert_eq!(
            available_version("freshen 0.1.0 < 0.2.0"),
            Some("0.2.0".to_owned())
        );
        assert_eq!(
            available_version("freshen (0.1.0 < 0.2.0)"),
            Some("0.2.0".to_owned())
        );
        assert_eq!(available_version("freshen 0.1.0"), Some("0.1.0".to_owned()));
        assert_eq!(
            available_version("node 18.0.0 < 20.0.0"),
            Some("20.0.0".to_owned())
        );
    }

    #[test]
    fn available_version_returns_none_for_a_line_without_a_version() {
        assert!(available_version("node").is_none());
        assert!(available_version("").is_none());
    }

    #[test]
    fn is_version_recognizes_dot_separated_numbers() {
        assert!(is_version("0.2.0"));
        assert!(is_version("1.2.3"));
        assert!(is_version("0.1"));
        assert!(!is_version(""));
        assert!(!is_version("freshen"));
        assert!(!is_version("<"));
        assert!(!is_version("1."));
    }
}
