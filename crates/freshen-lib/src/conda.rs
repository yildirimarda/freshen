//! The Conda step: `conda update -n base conda` upgrades conda in the base
//! environment, and the check reports the other environments without touching
//! them.
//!
//! The step runs `conda update -n base conda` whenever `conda` is available.
//! It is skipped entirely when conda is absent, so it is safe to register on
//! every machine. The read-only check lists the environments other than
//! `base` so the user can see which environments the base conda update does
//! not cover; it never installs or upgrades anything.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The conda package manager step.
pub const CONDA: StepId = "conda";

/// conda runs on every platform freshen supports.
const CONDA_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the conda step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(CondaStep)
}

struct CondaStep;

impl CondaStep {
    /// Upgrades conda itself in the base environment.
    fn update_base_conda(&self, failures: &mut Vec<StepError>) {
        human!("conda: update -n base conda");
        if let Err(error) = CommandSpec::new("conda")
            .args(["update", "-n", "base", "conda"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(CONDA, &error));
        }
    }

    /// Reports the conda environments other than `base` without upgrading
    /// anything.
    fn other_environments(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("conda")
            .args(["env", "list"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(CONDA, &error))?;
        Ok(CheckReport::from_items(parse_conda_envs(output.stdout())))
    }
}

impl Step for CondaStep {
    fn id(&self) -> StepId {
        CONDA
    }

    fn display_name(&self) -> &str {
        "conda"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_conda())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.other_environments()
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_base_conda(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &CONDA_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `conda` binary is discoverable on `PATH`.
fn detect_conda() -> Detection {
    if crate::executable::command_exists("conda") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses `conda env list` output.
///
/// Every line that is not a comment or blank names one environment; the first
/// whitespace-separated token is the environment name. The `base` environment
/// is excluded because `conda update -n base conda` already covers it.
fn parse_conda_envs(output: &str) -> Vec<OutdatedItem> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let name = line.split_whitespace().next()?;
            if name == "base" {
                return None;
            }
            Some(OutdatedItem::new(name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_conda_envs;

    #[test]
    fn lists_environments_other_than_base() {
        let items = parse_conda_envs(
            "# conda environments:\n\
             #\n\
             base                  *  /opt/conda\n\
             foo                     /opt/conda/envs/foo\n\
             bar                    /opt/conda/envs/bar\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "foo");
        assert_eq!(items[1].name(), "bar");
    }

    #[test]
    fn ignores_base_and_empty_output() {
        assert!(parse_conda_envs("base  *  /opt/conda\n").is_empty());
        assert!(parse_conda_envs("").is_empty());
        assert!(parse_conda_envs("   \n").is_empty());
    }
}
