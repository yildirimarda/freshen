//! The `freshen self-update` subcommand: upgrades freshen itself through the
//! package manager that installed it.
//!
//! freshen keeps itself current by delegating the upgrade to the package
//! manager that owns its installation. When Homebrew installed freshen
//! (`brew list --formula freshen` succeeds), `brew upgrade freshen` is run.
//! Otherwise self-update reports that it cannot determine how freshen was
//! installed and does nothing.

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
}
