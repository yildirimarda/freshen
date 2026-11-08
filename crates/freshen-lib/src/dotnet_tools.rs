//! The .NET global tools step: `dotnet tool update --global` for each
//! installed tool, skipped when `dotnet` is absent.
//!
//! The step lists the globally installed tools with `dotnet tool list
//! --global` and then runs `dotnet tool update --global <tool>` for every one
//! of them. It is skipped entirely when `dotnet` is absent, so it is safe to
//! register on every machine. .NET global tools are user-level, so the step
//! must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The .NET global tools step.
pub const DOTNET_TOOLS: StepId = "dotnet-tools";

/// The .NET SDK runs on every platform freshen supports.
const DOTNET_TOOLS_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the .NET global tools step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(DotnetToolsStep)
}

struct DotnetToolsStep;

impl DotnetToolsStep {
    /// Lists the globally installed tools and updates each one, collecting the
    /// first failure so every tool is still attempted.
    fn update_all(&self, failures: &mut Vec<StepError>) {
        human!("dotnet tool list --global");
        let output = match CommandSpec::new("dotnet")
            .args(["tool", "list", "--global"])
            .run()
        {
            Ok(output) => output.stdout().to_owned(),
            Err(error) => {
                failures.push(StepError::run_from_process(DOTNET_TOOLS, &error));
                return;
            }
        };
        let tools = list_global_tools(&output);
        if tools.is_empty() {
            return;
        }
        for tool in &tools {
            human!("dotnet tool update --global {tool}");
            if let Err(error) = CommandSpec::new("dotnet")
                .args(["tool", "update", "--global", tool])
                .retry_transient_network(NETWORK_RETRIES)
                .run()
            {
                failures.push(StepError::run_from_process(DOTNET_TOOLS, &error));
            }
        }
    }
}

impl Step for DotnetToolsStep {
    fn id(&self) -> StepId {
        DOTNET_TOOLS
    }

    fn display_name(&self) -> &str {
        ".NET global tools"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_dotnet())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // `dotnet tool list --global` reports only the versions that are
        // installed, not which updates are available, so there is no reliable
        // read-only "what is outdated" query. Report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_all(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &DOTNET_TOOLS_PLATFORMS
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

/// Whether the `dotnet` SDK is discoverable on `PATH`.
fn detect_dotnet() -> Detection {
    if crate::executable::command_exists("dotnet") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses the tool ids from the output of `dotnet tool list --global`.
///
/// The table's header line (`Package Id ...`) and the separator line (only
/// dashes and spaces) are skipped; the first whitespace-delimited token of
/// every remaining line is a tool id.
fn list_global_tools(output: &str) -> Vec<String> {
    let mut tools = Vec::new();
    let mut in_table = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("Package Id") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if trimmed.chars().all(|c| c == '-' || c == ':' || c == ' ') {
            continue;
        }
        if let Some(id) = trimmed.split_whitespace().next() {
            tools.push(id.to_owned());
        }
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_ids_from_a_recorded_list_output() {
        let output = "\
Package Id         Versions
--------------------------------------
tool-one           1.0.0
tool-two           2.0.0      2.1.0
";
        assert_eq!(list_global_tools(output), ["tool-one", "tool-two"]);
    }

    #[test]
    fn ignores_lines_before_the_header() {
        let output = "Welcome to .NET 8.0!\n\nPackage Id         Versions\n--------------------------------------\ntool-one           1.0.0\n";
        assert_eq!(list_global_tools(output), ["tool-one"]);
    }

    #[test]
    fn an_empty_list_has_no_tools() {
        assert!(list_global_tools(
            "Package Id         Versions\n--------------------------------------\n"
        )
        .is_empty());
    }

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }
}
