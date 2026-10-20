//! uv and mise steps.
//!
//! The zsh implementation updates uv's installed tools and prunes its cache,
//! then self-updates mise unless Homebrew owns it, upgrades every mise-managed
//! tool and prunes unused versions. Both blocks keep going after a failed
//! command and record the failure at the end, which is what the two steps here
//! do: every command runs, and the step reports the first failure.
//!
//! mise is skipped when it is installed through Homebrew because the formula
//! upgrade already replaced it. `brew list --formula mise` is the same probe
//! the zsh script uses; when Homebrew itself is missing the self-update runs.

use std::env;
use std::path::Path;

use crate::homebrew::BREW;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
use crate::warnings::warn_about_stale_node_processes;

/// Installed uv tools and the uv cache.
pub const UV: StepId = "uv";
/// mise itself and every tool it manages.
pub const MISE: StepId = "mise";

const TOOLING_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the uv and mise steps, after the Homebrew and container steps.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(UvStep).with(MiseStep)
}

struct UvStep;

impl Step for UvStep {
    fn id(&self) -> StepId {
        UV
    }

    fn display_name(&self) -> &str {
        "uv"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_tool("uv"))
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();

        println!("uv: tool upgrade --all");
        if let Err(error) = CommandSpec::new("uv")
            .args(["tool", "upgrade", "--all"])
            .run()
        {
            failures.push(error.to_string());
        }

        println!("uv: cache prune");
        if let Err(error) = CommandSpec::new("uv").args(["cache", "prune"]).run() {
            failures.push(error.to_string());
        }

        finish(UV, failures)
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[BREW]
    }

    fn platforms(&self) -> &'static [Platform] {
        &TOOLING_PLATFORMS
    }
}

struct MiseStep;

impl Step for MiseStep {
    fn id(&self) -> StepId {
        MISE
    }

    fn display_name(&self) -> &str {
        "mise"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_tool("mise"))
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();

        if !mise_installed_with_brew() {
            println!("mise: self-update");
            if let Err(error) = CommandSpec::new("mise").args(["self-update", "-y"]).run() {
                println!("warning: mise self-update skipped/failed: {error}");
            }
        }

        println!("mise: upgrade (node and other tools)");
        if let Err(error) = CommandSpec::new("mise").arg("upgrade").run() {
            failures.push(error.to_string());
        }

        println!("mise: prune (remove unused old versions)");
        if let Err(error) = CommandSpec::new("mise").args(["prune", "-y"]).run() {
            failures.push(error.to_string());
        }

        warn_about_stale_node_processes();

        finish(MISE, failures)
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[BREW]
    }

    fn platforms(&self) -> &'static [Platform] {
        &TOOLING_PLATFORMS
    }
}

fn finish(step: StepId, failures: Vec<String>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(message) => Err(StepError::run(step, message)),
        None => Ok(()),
    }
}

fn mise_installed_with_brew() -> bool {
    command_exists("brew")
        && CommandSpec::new("brew")
            .args(["list", "--formula", "mise"])
            .run()
            .is_ok()
}

fn detect_tool(tool: &str) -> Detection {
    if command_exists(tool) {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

fn command_exists(program: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|directory| is_executable(&directory.join(program)))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::{register, MISE, UV};
    use crate::containers;
    use crate::homebrew;
    use crate::platform::Platform;
    use crate::registry::Registry;
    use crate::step::StepId;

    fn ordered_ids(platform: Platform) -> Vec<StepId> {
        register(homebrew::register(containers::register(Registry::new())))
            .ordered_for(platform)
            .unwrap_or_else(|error| panic!("registry order failed: {error}"))
            .iter()
            .map(|step| step.id())
            .collect()
    }

    #[test]
    fn registry_runs_uv_and_mise_last() {
        for platform in [Platform::Macos, Platform::Linux] {
            assert_eq!(
                ordered_ids(platform),
                [
                    "brew",
                    "colima-stop",
                    "brew-formulae",
                    "colima-start",
                    "brew-casks",
                    "brew-autoremove",
                    "brew-cleanup",
                    "brew-reopen",
                    "docker",
                    "uv",
                    "mise",
                ],
                "{platform}"
            );
        }
        assert_eq!(
            ordered_ids(Platform::Windows),
            ["uv", "mise"],
            "Homebrew and container steps do not run on Windows"
        );
    }

    #[test]
    fn uv_and_mise_declare_brew_as_a_prerequisite() {
        let registry = register(Registry::new());
        let prerequisite_ids = |id: StepId| {
            registry
                .steps()
                .find(|step| step.id() == id)
                .unwrap_or_else(|| panic!("{id} is not registered"))
                .prerequisites()
                .to_vec()
        };

        assert_eq!(prerequisite_ids(UV), ["brew"]);
        assert_eq!(prerequisite_ids(MISE), ["brew"]);
    }

    #[test]
    fn uv_and_mise_run_on_every_supported_platform() {
        let registry = register(Registry::new());
        for id in [UV, MISE] {
            let step = registry
                .steps()
                .find(|step| step.id() == id)
                .unwrap_or_else(|| panic!("{id} is not registered"));
            assert_eq!(step.platforms(), Platform::ALL.as_slice(), "{id}");
        }
    }
}
