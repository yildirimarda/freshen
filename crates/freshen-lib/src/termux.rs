//! The Termux step: `pkg update` followed by `pkg upgrade`, skipped when not
//! running on Termux.
//!
//! Termux is an Android terminal environment that ships the `pkg` package
//! manager. It is detected through the `TERMUX_VERSION` environment variable
//! and a `PREFIX` that lives under `/data/data/com.termux`. It never needs
//! root, so `pkg update` and `pkg upgrade` run as the current user. It is
//! safe to register on every machine; on hosts that are not Termux it is never
//! detected.

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Termux package manager step.
pub const TERMUX: StepId = "termux";

/// Termux only runs on Linux.
const TERMUX_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// The Android data directory Termux installs into.
const TERMUX_PREFIX_ROOT: &str = "/data/data/com.termux";

/// Registers the Termux step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(TermuxStep)
}

struct TermuxStep;

impl TermuxStep {
    /// Runs `pkg update` and then `pkg upgrade`, collecting the first failure
    /// so both commands are attempted even when update fails.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("pkg update");
        if let Err(error) = CommandSpec::new("pkg")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(TERMUX, &error));
        }

        human!("pkg upgrade");
        if let Err(error) = CommandSpec::new("pkg")
            .args(["upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(TERMUX, &error));
        }
    }
}

impl Step for TermuxStep {
    fn id(&self) -> StepId {
        TERMUX
    }

    fn display_name(&self) -> &str {
        "termux"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_termux())
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
        &TERMUX_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether this process runs on Termux.
///
/// Termux sets `TERMUX_VERSION` and a `PREFIX` that lives under
/// `/data/data/com.termux`. Both must be present; `TERMUX_VERSION` alone is
/// not enough because a foreign shell could define it.
fn detect_termux() -> Detection {
    let env: Vec<(String, String)> = std::env::vars().collect();
    detect_termux_from_env(&env)
}

/// Detects Termux from an explicit list of environment variables, so the
/// decision can be tested without mutating the process environment.
fn detect_termux_from_env(env: &[(String, String)]) -> Detection {
    let version = env
        .iter()
        .find(|(key, _)| key == "TERMUX_VERSION")
        .map(|(_, value)| value.as_str());
    let prefix = env
        .iter()
        .find(|(key, _)| key == "PREFIX")
        .map(|(_, value)| value.as_str());

    let Some(version) = version else {
        return Detection::NotInstalled;
    };
    if version.is_empty() {
        return Detection::NotInstalled;
    }
    let Some(prefix) = prefix else {
        return Detection::NotInstalled;
    };
    if prefix.starts_with(TERMUX_PREFIX_ROOT) {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_termux_from_env, Detection, TERMUX_PREFIX_ROOT};

    #[test]
    fn detects_termux_when_version_and_prefix_are_set() {
        assert_eq!(
            detect_termux_from_env(&[
                ("TERMUX_VERSION".to_string(), "1.117.0".to_string()),
                (
                    "PREFIX".to_string(),
                    "/data/data/com.termux/usr".to_string()
                ),
            ]),
            Detection::Installed
        );
    }

    #[test]
    fn does_not_detect_without_termux_version() {
        assert_eq!(
            detect_termux_from_env(&[(
                "PREFIX".to_string(),
                "/data/data/com.termux/usr".to_string()
            )]),
            Detection::NotInstalled
        );
    }

    #[test]
    fn does_not_detect_without_prefix() {
        assert_eq!(
            detect_termux_from_env(&[("TERMUX_VERSION".to_string(), "1.117.0".to_string())]),
            Detection::NotInstalled
        );
    }

    #[test]
    fn does_not_detect_when_prefix_is_outside_termux() {
        assert_eq!(
            detect_termux_from_env(&[
                ("TERMUX_VERSION".to_string(), "1.117.0".to_string()),
                ("PREFIX".to_string(), "/usr/local".to_string()),
            ]),
            Detection::NotInstalled
        );
    }

    #[test]
    fn does_not_detect_when_version_is_empty() {
        assert_eq!(
            detect_termux_from_env(&[
                ("TERMUX_VERSION".to_string(), "".to_string()),
                (
                    "PREFIX".to_string(),
                    "/data/data/com.termux/usr".to_string()
                ),
            ]),
            Detection::NotInstalled
        );
    }

    #[test]
    fn prefix_must_be_under_termux_root() {
        assert!("/data/data/com.termux/usr".starts_with(TERMUX_PREFIX_ROOT));
        assert!(!"/usr/local".starts_with(TERMUX_PREFIX_ROOT));
    }
}
