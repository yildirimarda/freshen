//! The Mamba step: `micromamba self-update` upgrades micromamba itself, and
//! the check reports the other environments without touching them.
//!
//! The step runs `micromamba self-update` whenever `micromamba` is available.
//! It is skipped entirely when micromamba is absent, so it is safe to register
//! on every machine. The read-only check lists the environments other than
//! `base` so the user can see which environments the micromamba self-update
//! does not cover; it never installs or upgrades anything.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The micromamba step.
pub const MAMBA: StepId = "mamba";

/// micromamba runs on every platform freshen supports.
const MAMBA_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the mamba step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(MambaStep)
}

struct MambaStep;

impl MambaStep {
    /// Upgrades micromamba itself.
    fn update_micromamba(&self, failures: &mut Vec<StepError>) {
        human!("mamba: micromamba self-update");
        if let Err(error) = CommandSpec::new("micromamba")
            .args(["self-update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(MAMBA, &error));
        }
    }

    /// Reports the micromamba environments other than `base` without
    /// upgrading anything.
    fn other_environments(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("micromamba")
            .args(["env", "list"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(MAMBA, &error))?;
        Ok(CheckReport::from_items(parse_mamba_envs(output.stdout())))
    }
}

impl Step for MambaStep {
    fn id(&self) -> StepId {
        MAMBA
    }

    fn display_name(&self) -> &str {
        "mamba"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_micromamba())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.other_environments()
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_micromamba(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &MAMBA_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `micromamba` binary is discoverable on `PATH`.
fn detect_micromamba() -> Detection {
    if crate::executable::command_exists("micromamba") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses `micromamba env list` output.
///
/// Every line that is not a comment or blank names one environment; the first
/// whitespace-separated token is the environment name. The `base` environment
/// is excluded because `micromamba self-update` already covers it.
fn parse_mamba_envs(output: &str) -> Vec<OutdatedItem> {
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
    use super::parse_mamba_envs;

    #[test]
    fn lists_environments_other_than_base() {
        let items = parse_mamba_envs(
            "# micromamba environments:\n\
             #\n\
             base                  *  /opt/micromamba\n\
             foo                     /opt/micromamba/envs/foo\n\
             bar                    /opt/micromamba/envs/bar\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "foo");
        assert_eq!(items[1].name(), "bar");
    }

    #[test]
    fn ignores_base_and_empty_output() {
        assert!(parse_mamba_envs("base  *  /opt/micromamba\n").is_empty());
        assert!(parse_mamba_envs("").is_empty());
        assert!(parse_mamba_envs("   \n").is_empty());
    }
}
