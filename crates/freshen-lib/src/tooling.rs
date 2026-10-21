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

use crate::cleanup;
use crate::homebrew::BREW;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::snapshot::{SnapshotCommand, SnapshotSection};
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};
use crate::warnings::{prunable_tool_versions, warn_about_stale_tool_processes};

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

impl UvStep {
    /// Upgrades the installed tools, honouring the package filter.
    ///
    /// Without a filter every outdated tool is upgraded in one command, which
    /// is what the zsh script does. With a filter each selected tool is
    /// upgraded on its own, so a tool that is not in the lists stays untouched.
    fn upgrade_tools(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        let filter = options.filters().packages();
        if filter.is_empty() {
            human!("uv: tool upgrade --all");
            if let Err(error) = CommandSpec::new("uv")
                .args(["tool", "upgrade", "--all"])
                .retry_transient_network(NETWORK_RETRIES)
                .run()
            {
                failures.push(StepError::run_from_process(UV, &error));
            }
            return;
        }

        let output = match CommandSpec::new("uv")
            .args(["tool", "list", "--outdated"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            Ok(output) => output,
            Err(error) => {
                failures.push(StepError::run_from_process(UV, &error));
                return;
            }
        };

        let tools: Vec<String> = parse_uv_outdated(output.stdout())
            .into_iter()
            .filter(|item| filter.includes(item.name()))
            .map(|item| item.name().to_owned())
            .collect();

        if tools.is_empty() {
            human!("No outdated uv tools left after applying the include/exclude lists.");
            return;
        }

        for tool in tools {
            human!("uv: tool upgrade {tool}");
            if let Err(error) = CommandSpec::new("uv")
                .args(["tool", "upgrade"])
                .arg(&tool)
                .retry_transient_network(NETWORK_RETRIES)
                .run()
            {
                failures.push(StepError::run_from_process(UV, &error));
            }
        }
    }
}

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
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("uv")
            .args(["tool", "list", "--outdated"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(UV, &error))?;
        let report = CheckReport::from_items(parse_uv_outdated(output.stdout()));
        Ok(report.filtered(|name| options.filters().packages().includes(name)))
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();

        self.upgrade_tools(options, &mut failures);

        if let Err(error) = cleanup::run(
            UV,
            "uv: cache prune",
            CommandSpec::new("uv").args(["cache", "prune"]),
            options,
        ) {
            failures.push(error);
        }

        finish(failures)
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(SnapshotSection::new(
            UV,
            vec![SnapshotCommand::new("uv", ["tool", "list"])],
        ))
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
        let output = CommandSpec::new("mise")
            .arg("outdated")
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(MISE, &error))?;
        Ok(CheckReport::from_items(parse_mise_outdated(
            output.stdout(),
        )))
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();

        if !mise_installed_with_brew() {
            human!("mise: self-update");
            if let Err(error) = CommandSpec::new("mise")
                .args(["self-update", "-y"])
                .retry_transient_network(NETWORK_RETRIES)
                .run()
            {
                human!("warning: mise self-update skipped/failed: {error}");
            }
        }

        human!("mise: upgrade (node and other tools)");
        if let Err(error) = CommandSpec::new("mise")
            .arg("upgrade")
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(MISE, &error));
        }

        let prunable = prunable_tool_versions();

        if let Err(error) = cleanup::run(
            MISE,
            "mise: prune (remove unused old versions)",
            CommandSpec::new("mise").args(["prune", "-y"]),
            options,
        ) {
            failures.push(error);
        }

        warn_about_stale_tool_processes(&prunable);

        finish(failures)
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(SnapshotSection::new(
            MISE,
            vec![SnapshotCommand::new("mise", ["ls"])],
        ))
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[BREW]
    }

    fn platforms(&self) -> &'static [Platform] {
        &TOOLING_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Parses `uv tool list --outdated` output.
///
/// Each outdated tool is printed as `name vCURRENT [latest: LATEST]`, followed
/// by one `- executable` line per installed entry point. Only the tool line is
/// reported, so the executables do not become separate outdated items.
fn parse_uv_outdated(output: &str) -> Vec<OutdatedItem> {
    output.lines().filter_map(parse_uv_outdated_line).collect()
}

fn parse_uv_outdated_line(line: &str) -> Option<OutdatedItem> {
    let line = line.trim();
    let (name, rest) = line.split_once(" v")?;
    let current = rest.split_whitespace().next()?;
    let (_, latest) = rest.split_once("[latest: ")?;
    let latest = latest.split(']').next()?;

    if name.is_empty() || current.is_empty() || latest.is_empty() {
        return None;
    }

    Some(
        OutdatedItem::new(name)
            .with_current(current)
            .with_latest(latest),
    )
}

/// Parses the table printed by `mise outdated`.
///
/// The columns have been `name|tool|plugin`, `requested`, `current`, `latest`
/// and (in newer versions) `source`; only name, current and latest matter here.
/// A header row is recognised by its `requested` column and skipped. Older
/// mise versions print `MISSING` instead of a version for a tool that is not
/// installed yet, so that column is treated as unknown.
fn parse_mise_outdated(output: &str) -> Vec<OutdatedItem> {
    output
        .lines()
        .filter_map(parse_mise_outdated_line)
        .collect()
}

fn parse_mise_outdated_line(line: &str) -> Option<OutdatedItem> {
    let mut fields = line.split_whitespace();
    let name = fields.next()?;
    let requested = fields.next()?;
    let current = fields.next()?;
    let latest = fields.next()?;

    if requested.eq_ignore_ascii_case("requested") || current.eq_ignore_ascii_case("current") {
        return None;
    }

    let item = OutdatedItem::new(name).with_latest(latest);
    if current.starts_with('[') || current.eq_ignore_ascii_case("missing") {
        Some(item)
    } else {
        Some(item.with_current(current))
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
    crate::executable::command_exists(program)
}

#[cfg(test)]
mod tests {
    use super::{parse_mise_outdated, parse_uv_outdated, register, MISE, UV};
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
    fn built_in_registry_is_valid() {
        let registry = register(homebrew::register(containers::register(Registry::new())));

        assert_eq!(registry.validate(), Ok(()));
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

    #[test]
    fn parses_uv_outdated_tools_without_their_executables() {
        let items = parse_uv_outdated(
            "black v23.1.0 [latest: 26.10.1]\n\
             - black\n\
             - blackd\n\
             ruff v0.6.0 [latest: 0.7.0]\n\
             - ruff\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "black");
        assert_eq!(items[0].current(), Some("23.1.0"));
        assert_eq!(items[0].latest(), Some("26.10.1"));
        assert_eq!(items[1].name(), "ruff");
    }

    #[test]
    fn ignores_uv_output_without_outdated_tools() {
        assert!(parse_uv_outdated("No tools installed\n").is_empty());
        assert!(parse_uv_outdated("black v23.1.0\n- black\n").is_empty());
    }

    #[test]
    fn parses_the_mise_outdated_table_and_skips_its_header() {
        let items = parse_mise_outdated(
            "Plugin  Requested  Current  Latest\n\
             node    20         20.0.0   20.1.0\n\
             python  3.11       3.11.0   3.11.1\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "node");
        assert_eq!(items[0].current(), Some("20.0.0"));
        assert_eq!(items[0].latest(), Some("20.1.0"));
        assert_eq!(items[1].name(), "python");
    }

    #[test]
    fn parses_mise_rows_with_a_source_column_and_a_missing_install() {
        let items = parse_mise_outdated(
            "name  requested  current    latest  source\n\
             node  20         20.0.0     20.1.0  /home/me/src/app/mise.toml\n\
             go    1.21       [MISSING]  1.22.0  /home/me/.config/mise/config.toml\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "node");
        assert_eq!(items[1].name(), "go");
        assert_eq!(items[1].current(), None);
        assert_eq!(items[1].latest(), Some("1.22.0"));
    }
}
