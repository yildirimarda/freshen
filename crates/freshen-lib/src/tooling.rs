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
use std::fs;
use std::path::Path;

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
            .neutral_locale()
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
            .neutral_locale()
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
            CommandSpec::new("uv")
                .args(["cache", "prune"])
                .neutral_locale(),
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

    fn is_user_level(&self) -> bool {
        true
    }
}

struct MiseStep;

impl MiseStep {
    /// Upgrades the installed tools, honouring the package filter.
    ///
    /// Without a filter every outdated tool is upgraded in one command, which
    /// is what the zsh script does. With a filter each selected tool is
    /// upgraded on its own, so a tool that is not in the lists stays untouched.
    ///
    /// With `--bump`, tools requested as `lts` or `latest` are pinned to a
    /// fixed version instead of staying on a moving track, and a warning is
    /// printed first.
    fn upgrade_tools(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        let bump = options.bump();
        let filter = options.filters().packages();

        if filter.is_empty() && !bump {
            human!("mise: upgrade (node and other tools)");
            if let Err(error) = CommandSpec::new("mise")
                .arg("upgrade")
                .retry_transient_network(NETWORK_RETRIES)
                .run()
            {
                failures.push(StepError::run_from_process(MISE, &error));
            }
            return;
        }

        let output = match CommandSpec::new("mise")
            .arg("outdated")
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            Ok(output) => output,
            Err(error) => {
                failures.push(StepError::run_from_process(MISE, &error));
                return;
            }
        };

        let rows = parse_mise_outdated_rows(output.stdout());
        let tools: Vec<&MiseOutdatedRow> = rows
            .iter()
            .filter(|row| filter.includes(row.name.as_str()))
            .collect();

        if tools.is_empty() {
            human!("No outdated mise tools left after applying the include/exclude lists.");
            return;
        }

        if bump {
            human!("warning: mise --bump replaces lts/latest with a fixed version");
        }

        for row in tools {
            let name = row.name.as_str();
            if bump && is_lts_or_latest(row.requested.as_deref()) {
                let fixed = row.latest.as_str();
                human!("mise: install {name}@{fixed}");
                if let Err(error) = CommandSpec::new("mise")
                    .args(["install", &format!("{name}@{fixed}")])
                    .retry_transient_network(NETWORK_RETRIES)
                    .run()
                {
                    failures.push(StepError::run_from_process(MISE, &error));
                }
            } else {
                human!("mise: upgrade {name}");
                if let Err(error) = CommandSpec::new("mise")
                    .args(["upgrade", name])
                    .retry_transient_network(NETWORK_RETRIES)
                    .run()
                {
                    failures.push(StepError::run_from_process(MISE, &error));
                }
            }
        }
    }
}

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
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("mise")
            .arg("outdated")
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(MISE, &error))?;
        let mut items: Vec<OutdatedItem> = parse_mise_outdated(output.stdout());

        // Report when a newer Node LTS major exists than the active one,
        // without switching to it.
        if let Some(lts_item) = node_lts_outdated_item() {
            items.push(lts_item);
        }

        let report = CheckReport::from_items(items);
        Ok(report.filtered(|name| options.filters().packages().includes(name)))
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

        let before_node = active_node_version();
        let before_python = if python_defaults_configured() {
            active_python_version()
        } else {
            None
        };
        let before_ruby = if ruby_defaults_configured() {
            active_ruby_version()
        } else {
            None
        };

        self.upgrade_tools(options, &mut failures);

        let after_node = active_node_version();
        let after_python = if python_defaults_configured() {
            active_python_version()
        } else {
            None
        };
        let after_ruby = if ruby_defaults_configured() {
            active_ruby_version()
        } else {
            None
        };

        if let (Some(before), Some(after)) = (&before_node, &after_node) {
            if before != after {
                reinstall_default_npm_packages();
            }
        }

        if let (Some(before), Some(after)) = (&before_python, &after_python) {
            if before != after {
                reinstall_default_python_packages();
            }
        }

        if let (Some(before), Some(after)) = (&before_ruby, &after_ruby) {
            if before != after {
                reinstall_default_ruby_packages();
            }
        }

        let prunable = prunable_tool_versions();

        if let Err(error) = cleanup::run(
            MISE,
            "mise: prune (remove unused old versions)",
            CommandSpec::new("mise")
                .args(["prune", "-y"])
                .neutral_locale(),
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

/// Reinstalls the global npm packages listed in `~/.default-npm-packages`
/// into the Node version mise just installed.
///
/// The file follows the nvm convention: one package name per line, blank
/// lines and `#` comments ignored. Best effort: a missing file, a missing
/// `npm` or a failed install prints a warning and never fails the mise step.
fn reinstall_default_npm_packages() {
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok();
    let Some(home) = home else {
        return;
    };
    let path = Path::new(&home).join(".default-npm-packages");
    let Ok(contents) = fs::read_to_string(&path) else {
        return;
    };

    let packages: Vec<&str> = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

    if packages.is_empty() {
        return;
    }

    human!("mise: reinstall global npm packages for the new Node version");
    if let Err(error) = CommandSpec::new("npm")
        .args(["install", "-g"])
        .args(packages)
        .retry_transient_network(NETWORK_RETRIES)
        .run()
    {
        human!("warning: could not reinstall default npm packages: {error}");
    }
}

/// Whether `~/.default-python-packages` exists, meaning there are default
/// packages that might need reinstalling after a Python version change.
fn python_defaults_configured() -> bool {
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok();
    let Some(home) = home else {
        return false;
    };
    fs::metadata(Path::new(&home).join(".default-python-packages")).is_ok()
}

/// Reinstalls the Python packages listed in `~/.default-python-packages`
/// into the Python version mise just installed.
///
/// The file follows the nvm convention: one package name per line, blank
/// lines and `#` comments ignored. Best effort: a missing file, a missing
/// `pip` or a failed install prints a warning and never fails the mise step.
fn reinstall_default_python_packages() {
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok();
    let Some(home) = home else {
        return;
    };
    let path = Path::new(&home).join(".default-python-packages");
    let Ok(contents) = fs::read_to_string(&path) else {
        return;
    };

    let packages: Vec<&str> = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

    if packages.is_empty() {
        return;
    }

    human!("mise: reinstall default python packages for the new Python version");
    if let Err(error) = CommandSpec::new("pip")
        .args(["install"])
        .args(packages)
        .retry_transient_network(NETWORK_RETRIES)
        .run()
    {
        human!("warning: could not reinstall default python packages: {error}");
    }
}

/// Whether `~/.default-ruby-packages` exists, meaning there are default
/// packages that might need reinstalling after a Ruby version change.
fn ruby_defaults_configured() -> bool {
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok();
    let Some(home) = home else {
        return false;
    };
    fs::metadata(Path::new(&home).join(".default-ruby-packages")).is_ok()
}

/// Reinstalls the Ruby packages listed in `~/.default-ruby-packages`
/// into the Ruby version mise just installed.
///
/// The file follows the nvm convention: one package name per line, blank
/// lines and `#` comments ignored. Best effort: a missing file, a missing
/// `gem` or a failed install prints a warning and never fails the mise step.
fn reinstall_default_ruby_packages() {
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok();
    let Some(home) = home else {
        return;
    };
    let path = Path::new(&home).join(".default-ruby-packages");
    let Ok(contents) = fs::read_to_string(&path) else {
        return;
    };

    let packages: Vec<&str> = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

    if packages.is_empty() {
        return;
    }

    human!("mise: reinstall default ruby packages for the new Ruby version");
    if let Err(error) = CommandSpec::new("gem")
        .args(["install"])
        .args(packages)
        .retry_transient_network(NETWORK_RETRIES)
        .run()
    {
        human!("warning: could not reinstall default ruby packages: {error}");
    }
}

/// The active Ruby version from `mise current ruby`, when ruby is installed.
#[must_use]
fn active_ruby_version() -> Option<String> {
    let Ok(output) = CommandSpec::new("mise")
        .args(["current", "ruby"])
        .neutral_locale()
        .run()
    else {
        return None;
    };
    let version = output.stdout().trim().to_owned();
    (!version.is_empty()).then_some(version)
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

    // The label inside the brackets is translated, so only the shape of the
    // bracket is used: the version is what follows the last colon.
    let (_, bracket) = rest.split_once('[')?;
    let (bracket, _) = bracket.split_once(']')?;
    let (_, latest) = bracket.rsplit_once(':')?;
    let latest = latest.trim();

    if name.is_empty() || current.is_empty() || latest.is_empty() {
        return None;
    }

    Some(
        OutdatedItem::new(name)
            .with_current(current)
            .with_latest(latest),
    )
}

/// One row of `mise outdated` output, including the requested version.
#[derive(Debug, Clone)]
struct MiseOutdatedRow {
    name: String,
    requested: Option<String>,
    current: Option<String>,
    latest: String,
}

/// Parses the table printed by `mise outdated`.
///
/// The columns have been `name|tool|plugin`, `requested`, `current`, `latest`
/// and (in newer versions) `source`; only name, requested, current and latest
/// matter here. A header row is recognised by its `requested` column and
/// skipped. Older mise versions print `MISSING` instead of a version for a
/// tool that is not installed yet, so that column is treated as unknown.
fn parse_mise_outdated(output: &str) -> Vec<OutdatedItem> {
    parse_mise_outdated_rows(output)
        .into_iter()
        .map(|row| {
            let mut item = OutdatedItem::new(row.name).with_latest(row.latest);
            if let Some(current) = row.current {
                item = item.with_current(current);
            }
            item
        })
        .collect()
}

/// All rows of `mise outdated` output, keeping the requested version so the
/// `--bump` flag can tell `lts`/`latest` requests apart from pinned ones.
fn parse_mise_outdated_rows(output: &str) -> Vec<MiseOutdatedRow> {
    output.lines().filter_map(parse_mise_outdated_row).collect()
}

fn parse_mise_outdated_row(line: &str) -> Option<MiseOutdatedRow> {
    let mut fields = line.split_whitespace();
    let name = fields.next()?;
    let requested = fields.next().map(str::to_owned);
    let current = fields.next()?;
    let latest = fields.next()?;

    if requested
        .as_deref()
        .is_some_and(|r| r.eq_ignore_ascii_case("requested"))
        || current.eq_ignore_ascii_case("current")
    {
        return None;
    }

    // The labels of the table are translated, so the header row is not
    // recognised by its words but by the version it does not contain.
    if !looks_like_version(latest) {
        return None;
    }

    let current = if current.starts_with('[') || current.eq_ignore_ascii_case("missing") {
        None
    } else {
        Some(current.to_owned())
    };

    Some(MiseOutdatedRow {
        name: name.to_owned(),
        requested,
        current,
        latest: latest.to_owned(),
    })
}

/// Whether `field` can be the version column of `mise outdated`.
///
/// A version contains a digit, except for the special values mise prints for
/// a tool it does not manage itself.
fn looks_like_version(field: &str) -> bool {
    field.chars().any(|character| character.is_ascii_digit())
        || matches!(
            field.to_ascii_lowercase().as_str(),
            "latest" | "system" | "missing" | "unknown"
        )
}

/// Whether the tool was requested as `lts` or `latest`, meaning a plain
/// `mise upgrade` keeps it on a moving track instead of a fixed version.
fn is_lts_or_latest(requested: Option<&str>) -> bool {
    requested.is_some_and(|r| r.eq_ignore_ascii_case("lts") || r.eq_ignore_ascii_case("latest"))
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

/// Whether a newer Node LTS major exists than the active one, without
/// switching to it.
///
/// Runs `mise ls-remote node@lts` to list available LTS versions and
/// `mise current node` to learn the active version. When the newest LTS
/// major is higher than the active major, returns an outdated item naming
/// the latest LTS version. Best effort: a missing `mise`, an absent node
/// install or unparseable output yields `None` and never fails the step.
#[must_use]
fn node_lts_outdated_item() -> Option<OutdatedItem> {
    let active = active_node_version()?;
    let active_major = major_version(&active)?;

    let Ok(output) = CommandSpec::new("mise")
        .args(["ls-remote", "node@lts"])
        .neutral_locale()
        .retry_transient_network(NETWORK_RETRIES)
        .run()
    else {
        return None;
    };

    let latest = latest_lts_version(output.stdout())?;
    let latest_major = major_version(&latest)?;

    (latest_major > active_major).then_some(
        OutdatedItem::new("node")
            .with_current(active)
            .with_latest(latest)
            .with_reason("newer Node LTS major available"),
    )
}

/// The active Node version from `mise current node`, when node is installed.
#[must_use]
fn active_node_version() -> Option<String> {
    let Ok(output) = CommandSpec::new("mise")
        .args(["current", "node"])
        .neutral_locale()
        .run()
    else {
        return None;
    };
    let version = output.stdout().trim().to_owned();
    (!version.is_empty()).then_some(version)
}

/// The active Python version from `mise current python`, when python is installed.
#[must_use]
fn active_python_version() -> Option<String> {
    let Ok(output) = CommandSpec::new("mise")
        .args(["current", "python"])
        .neutral_locale()
        .run()
    else {
        return None;
    };
    let version = output.stdout().trim().to_owned();
    (!version.is_empty()).then_some(version)
}

/// The leading numeric component of a version string, e.g. `20` from `20.18.0`.
#[must_use]
fn major_version(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

/// The latest version from `mise ls-remote node@lts` output.
#[must_use]
fn latest_lts_version(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .max_by_key(|line| version_key(line))
        .map(str::to_owned)
}

/// A sortable key for a version string like `20.18.0`.
#[must_use]
fn version_key(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

#[cfg(test)]
mod tests {
    use super::{
        active_node_version, is_lts_or_latest, latest_lts_version, major_version,
        node_lts_outdated_item, parse_mise_outdated, parse_uv_outdated, register, version_key,
        MISE, UV,
    };
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
        assert_eq!(
            ordered_ids(Platform::Macos),
            [
                "macos-updates",
                "brew",
                "colima-stop",
                "brew-formulae",
                "colima-start",
                "brew-services",
                "brew-casks",
                "brew-autoremove",
                "brew-cleanup",
                "brew-doctor",
                "brew-bundle-cleanup",
                "brew-reopen",
                "docker",
                "uv",
                "mise",
            ]
        );
        assert_eq!(
            ordered_ids(Platform::Linux),
            [
                "brew",
                "colima-stop",
                "brew-formulae",
                "colima-start",
                "brew-services",
                "brew-casks",
                "brew-autoremove",
                "brew-cleanup",
                "brew-doctor",
                "brew-bundle-cleanup",
                "brew-reopen",
                "docker",
                "uv",
                "mise",
            ]
        );
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
    fn parses_uv_outdated_tools_with_a_translated_label() {
        let items = parse_uv_outdated(
            "black v23.1.0 [en son: 26.10.1]\n\
             - black\n",
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "black");
        assert_eq!(items[0].current(), Some("23.1.0"));
        assert_eq!(items[0].latest(), Some("26.10.1"));
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

    #[test]
    fn skips_a_translated_mise_header_row() {
        let items = parse_mise_outdated(
            "Araç  İstenen  Güncel  En Son\n\
             node  20         20.0.0   20.1.0\n",
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "node");
        assert_eq!(items[0].current(), Some("20.0.0"));
        assert_eq!(items[0].latest(), Some("20.1.0"));
    }

    #[test]
    fn version_key_parses_major_minor_patch() {
        assert_eq!(version_key("20.18.0"), (20, 18, 0));
        assert_eq!(version_key("22.12.0"), (22, 12, 0));
        assert_eq!(version_key("18.0.0"), (18, 0, 0));
        assert_eq!(version_key("20"), (20, 0, 0));
        assert_eq!(version_key("not.a.version"), (0, 0, 0));
    }

    #[test]
    fn major_version_returns_the_leading_component() {
        assert_eq!(major_version("20.18.0"), Some(20));
        assert_eq!(major_version("22.12.0"), Some(22));
        assert_eq!(major_version("18"), Some(18));
        assert_eq!(major_version("not-a-version"), None);
    }

    #[test]
    fn latest_lts_version_picks_the_highest_version() {
        let output = "18.20.0\n20.18.0\n22.12.0\n";
        assert_eq!(latest_lts_version(output), Some("22.12.0".to_owned()));

        let output = "22.12.0\n20.18.0\n18.20.0\n";
        assert_eq!(latest_lts_version(output), Some("22.12.0".to_owned()));

        let output = "";
        assert_eq!(latest_lts_version(output), None);
    }

    #[test]
    fn node_lts_outdated_item_reports_when_newer_lts_major_exists() {
        let _ = node_lts_outdated_item();
        // Best-effort: without a real `mise` on PATH this returns None.
        assert!(node_lts_outdated_item().is_none());
    }

    #[test]
    fn active_node_version_returns_none_without_mise() {
        assert!(active_node_version().is_none());
    }

    #[test]
    fn is_lts_or_latest_detects_moving_tracks() {
        assert!(is_lts_or_latest(Some("lts")));
        assert!(is_lts_or_latest(Some("latest")));
        assert!(is_lts_or_latest(Some("LTS")));
        assert!(is_lts_or_latest(Some("Latest")));
        assert!(!is_lts_or_latest(Some("20")));
        assert!(!is_lts_or_latest(Some("22.12.0")));
        assert!(!is_lts_or_latest(None));
    }
}
