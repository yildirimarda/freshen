//! Colima/lima and Docker orchestration.
//!
//! The zsh implementation stops the container runtime before Homebrew upgrades
//! `colima` or `lima`, then starts it again and brings the containers that were
//! running back up. Afterwards it pulls newer images for every running compose
//! project and prunes dangling images.
//!
//! The port keeps that behaviour in three steps so the runner can record a
//! failure per step: [`COLIMA_STOP`] runs before the formula upgrade,
//! [`COLIMA_START`] runs right after it, and [`DOCKER`] runs after the Homebrew
//! block. The snapshot of what was stopped is shared between the first two
//! steps, exactly like `colima_was_running`, `compose_projects` and
//! `standalone_containers` in `update-all.sh`.

use std::cell::RefCell;
use std::env;
use std::path::Path;
use std::rc::Rc;

use serde_json::Value;

use crate::cleanup;
use crate::homebrew::{BREW, FORMULAE, REOPEN};
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::snapshot::{self, SnapshotCommand, SnapshotSection};
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// Stops containers and Colima before the formula upgrade.
pub const COLIMA_STOP: StepId = "colima-stop";
/// Starts Colima and restores the containers that were stopped.
pub const COLIMA_START: StepId = "colima-start";
/// Pulls compose images and prunes dangling images.
pub const DOCKER: StepId = "docker";

const CONTAINER_PLATFORMS: [Platform; 2] = [Platform::Macos, Platform::Linux];

/// The `docker ps --format` argument, with the literal tabs the zsh script
/// produces with `$'...'`.
const PS_FORMAT: &str = "{{.ID}}\t{{.Names}}\t{{.Label \"com.docker.compose.project\"}}";

/// A compose project and the config files needed to address it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeProject {
    name: String,
    config_files: Vec<String>,
}

impl ComposeProject {
    /// Creates a project from its name and config files.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        config_files: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            config_files: config_files.into_iter().map(Into::into).collect(),
        }
    }

    /// Project name passed to `docker compose -p`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Config files passed to `docker compose -f`, in order.
    #[must_use]
    pub fn config_files(&self) -> &[String] {
        &self.config_files
    }
}

/// A running container that is not managed by a compose project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandaloneContainer {
    id: String,
    name: String,
}

impl StandaloneContainer {
    /// Creates a container from its id and name.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }

    /// Container id passed to `docker stop` and `docker start`.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Container name shown in the progress output.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Parses `docker compose ls --format json`.
///
/// Invalid JSON and malformed entries yield an empty list, matching the zsh
/// parser, which silently ignores a failed `docker compose ls` or bad JSON.
#[must_use]
pub fn parse_compose_projects(json: &str) -> Vec<ComposeProject> {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(projects) = value.as_array() else {
        return Vec::new();
    };

    projects.iter().filter_map(compose_project).collect()
}

fn compose_project(value: &Value) -> Option<ComposeProject> {
    let name = value.get("Name")?.as_str()?;
    let files = value.get("ConfigFiles")?.as_str()?;
    if name.is_empty() || files.is_empty() {
        return None;
    }

    let config_files: Vec<String> = files
        .split(',')
        .filter(|file| !file.is_empty())
        .map(str::to_owned)
        .collect();

    Some(ComposeProject {
        name: name.to_owned(),
        config_files,
    })
}

/// Parses the tab-separated output of `docker ps` used to find standalone
/// containers.
///
/// Rows whose compose-project label is non-empty are managed by compose and
/// are left out, exactly like the `awk -F'\t' '$3 == ""'` filter in the zsh
/// script.
#[must_use]
pub fn parse_standalone_containers(output: &str) -> Vec<StandaloneContainer> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let id = fields.next()?;
            let name = fields.next()?;
            let project = fields.next().unwrap_or_default();
            project.is_empty().then(|| StandaloneContainer {
                id: id.to_owned(),
                name: name.to_owned(),
            })
        })
        .collect()
}

/// Registers the container steps in the order they must be registered.
///
/// The steps declare their Homebrew prerequisites, so registering them before
/// the Homebrew steps makes the resulting run order:
/// `brew update`, stop Colima, upgrade formulae, start Colima, then casks,
/// cleanup, reopen and finally the Docker image step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    let snapshot = ContainerSnapshot::default();
    registry
        .with(ColimaStopStep::new(snapshot.clone()))
        .with(ColimaStartStep::new(snapshot))
        .with(DockerStep)
}

#[derive(Debug, Default, Clone)]
struct Snapshot {
    compose: Vec<ComposeProject>,
    standalone: Vec<StandaloneContainer>,
}

#[derive(Clone, Default)]
struct ContainerSnapshot(Rc<RefCell<Option<Snapshot>>>);

impl ContainerSnapshot {
    fn store(&self, snapshot: Snapshot) {
        *self.0.borrow_mut() = Some(snapshot);
    }

    fn get(&self) -> Option<Snapshot> {
        self.0.borrow().clone()
    }

    fn clear(&self) {
        *self.0.borrow_mut() = None;
    }
}

struct ColimaStopStep {
    snapshot: ContainerSnapshot,
}

impl ColimaStopStep {
    fn new(snapshot: ContainerSnapshot) -> Self {
        Self { snapshot }
    }
}

impl Step for ColimaStopStep {
    fn id(&self) -> StepId {
        COLIMA_STOP
    }

    fn display_name(&self) -> &str {
        "Colima/lima stop"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_with_brew("colima"))
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let outdated = CommandSpec::new("brew")
            .args(["outdated", "--formula", "--quiet"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::run_from_process(COLIMA_STOP, &error))?;
        let upgrade_stops_colima = lines(outdated.stdout())
            .iter()
            .any(|name| name == "colima" || name == "lima");
        if !upgrade_stops_colima || !colima_is_running() {
            return Ok(());
        }

        human!("colima/lima is outdated and colima is running: stopping containers and colima");

        let mut snapshot = Snapshot::default();
        let mut failures = Vec::new();

        if command_exists("docker") {
            snapshot = Snapshot {
                compose: list_compose_projects(),
                standalone: list_standalone_containers(),
            };
        }

        self.snapshot.store(snapshot.clone());

        for project in &snapshot.compose {
            human!("docker compose down: {}", project.name());
            if let Err(error) = compose_command(project, &["down"]).run() {
                failures.push(StepError::run_from_process(COLIMA_STOP, &error));
            }
        }
        for container in &snapshot.standalone {
            human!("docker stop: {}", container.name());
            if let Err(error) = docker_command(&["stop"], container.id()).run() {
                failures.push(StepError::run_from_process(COLIMA_STOP, &error));
            }
        }

        if let Err(error) = CommandSpec::new("colima").arg("stop").run() {
            failures.push(StepError::run_from_process(COLIMA_STOP, &error));
        }

        finish(failures)
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn restore(&self) -> Result<(), StepError> {
        restore_colima(&self.snapshot, COLIMA_STOP)
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[BREW]
    }

    fn platforms(&self) -> &'static [Platform] {
        &CONTAINER_PLATFORMS
    }
}

struct ColimaStartStep {
    snapshot: ContainerSnapshot,
}

impl ColimaStartStep {
    fn new(snapshot: ContainerSnapshot) -> Self {
        Self { snapshot }
    }

    fn start_and_restore(&self, message: &str) -> Result<(), StepError> {
        let Some(snapshot) = self.snapshot.get() else {
            return Ok(());
        };

        human!("{message}");
        let mut failures = Vec::new();

        if let Err(error) = CommandSpec::new("colima").arg("start").run() {
            human!("warning: colima did not start; containers were NOT restarted");
            return Err(StepError::run_from_process(COLIMA_START, &error));
        }

        for container in &snapshot.standalone {
            human!("docker start: {}", container.name());
            if let Err(error) = docker_command(&["start"], container.id()).run() {
                failures.push(StepError::run_from_process(COLIMA_START, &error));
            }
        }
        for project in &snapshot.compose {
            human!("docker compose up -d: {}", project.name());
            if let Err(error) = compose_command(project, &["up", "-d"]).run() {
                failures.push(StepError::run_from_process(COLIMA_START, &error));
            }
        }

        let result = finish(failures);
        if result.is_ok() {
            self.snapshot.clear();
        }
        result
    }
}

fn restore_colima(snapshot: &ContainerSnapshot, step: StepId) -> Result<(), StepError> {
    let Some(stopped) = snapshot.get() else {
        return Ok(());
    };

    human!("Restoring colima and containers");
    let mut failures = Vec::new();

    if let Err(error) = CommandSpec::new("colima").arg("start").run() {
        human!("warning: colima did not start; containers were NOT restarted");
        return Err(StepError::run_from_process(step, &error));
    }

    for container in &stopped.standalone {
        human!("docker start: {}", container.name());
        if let Err(error) = docker_command(&["start"], container.id()).run() {
            failures.push(StepError::run_from_process(step, &error));
        }
    }
    for project in &stopped.compose {
        human!("docker compose up -d: {}", project.name());
        if let Err(error) = compose_command(project, &["up", "-d"]).run() {
            failures.push(StepError::run_from_process(step, &error));
        }
    }

    let result = finish(failures);
    if result.is_ok() {
        snapshot.clear();
    }
    result
}

impl Step for ColimaStartStep {
    fn id(&self) -> StepId {
        COLIMA_START
    }

    fn display_name(&self) -> &str {
        "Colima/lima start"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_with_brew("colima"))
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.start_and_restore("Starting colima and containers")
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn restore(&self) -> Result<(), StepError> {
        self.start_and_restore("Restoring colima and containers")
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[FORMULAE, COLIMA_STOP]
    }

    fn platforms(&self) -> &'static [Platform] {
        &CONTAINER_PLATFORMS
    }
}

struct DockerStep;

impl Step for DockerStep {
    fn id(&self) -> StepId {
        DOCKER
    }

    fn display_name(&self) -> &str {
        "Docker images"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        let detected = if command_exists("docker") && command_exists("colima") {
            Detection::Installed
        } else {
            Detection::NotInstalled
        };
        Ok(detected)
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if !colima_is_running() {
            human!("warning: colima is not running, skipping Docker image updates.");
            return Ok(());
        }

        human!("Docker: pull newer images for running compose projects");
        let projects = list_compose_projects();
        let mut failures = Vec::new();

        if projects.is_empty() {
            human!("No running compose projects.");
        } else {
            for project in &projects {
                human!("docker compose pull + up -d: {}", project.name());
                if let Err(error) = compose_command(project, &["pull", "--quiet"]).run() {
                    failures.push(StepError::run_from_process(DOCKER, &error));
                }
                if let Err(error) = compose_command(project, &["up", "-d"]).run() {
                    failures.push(StepError::run_from_process(DOCKER, &error));
                }
            }
        }

        if let Err(error) = cleanup::run(
            DOCKER,
            "Docker: prune dangling images",
            CommandSpec::new("docker").args(["image", "prune", "-f"]),
            options,
        ) {
            failures.push(error);
        }

        finish(failures)
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(SnapshotSection::new(
            DOCKER,
            vec![SnapshotCommand::new("docker", ["image", "ls", "--digests"])],
        ))
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[REOPEN]
    }

    fn platforms(&self) -> &'static [Platform] {
        &CONTAINER_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn lines(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn colima_is_running() -> bool {
    CommandSpec::new("colima").arg("status").run().is_ok()
}

fn list_compose_projects() -> Vec<ComposeProject> {
    let Ok(output) = CommandSpec::new("docker")
        .args(["compose", "ls", "--format", "json"])
        .run()
    else {
        return Vec::new();
    };
    parse_compose_projects(output.stdout())
}

fn list_standalone_containers() -> Vec<StandaloneContainer> {
    let Ok(output) = CommandSpec::new("docker")
        .arg("ps")
        .args(["--format", PS_FORMAT])
        .run()
    else {
        return Vec::new();
    };
    parse_standalone_containers(output.stdout())
}

fn compose_command(project: &ComposeProject, arguments: &[&str]) -> CommandSpec {
    let mut spec = CommandSpec::new("docker")
        .args(["compose", "-p"])
        .arg(project.name());
    for file in project.config_files() {
        spec = spec.arg("-f").arg(file.as_str());
    }
    spec.args(arguments)
        .retry_transient_network(NETWORK_RETRIES)
}

fn docker_command(arguments: &[&str], id: &str) -> CommandSpec {
    CommandSpec::new("docker").args(arguments).arg(id)
}

fn detect_with_brew(tool: &str) -> Detection {
    if command_exists("brew") && command_exists(tool) {
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
    use super::{
        compose_command, parse_compose_projects, parse_standalone_containers, register,
        ComposeProject, StandaloneContainer, COLIMA_START, COLIMA_STOP, DOCKER, PS_FORMAT,
    };
    use crate::homebrew;
    use crate::platform::Platform;
    use crate::registry::Registry;
    use crate::step::StepId;
    use std::ffi::OsString;

    fn ordered_ids(platform: Platform) -> Vec<StepId> {
        homebrew::register(register(Registry::new()))
            .ordered_for(platform)
            .unwrap_or_else(|error| panic!("registry order failed: {error}"))
            .iter()
            .map(|step| step.id())
            .collect()
    }

    fn os_strings(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn registry_stops_colima_before_formulae_and_starts_it_afterwards() {
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
                ],
                "{platform}"
            );
        }
    }

    #[test]
    fn container_steps_declare_their_homebrew_prerequisites() {
        let registry = register(Registry::new());
        let prerequisite_ids = |id: StepId| {
            registry
                .steps()
                .find(|step| step.id() == id)
                .unwrap_or_else(|| panic!("{id} is not registered"))
                .prerequisites()
                .to_vec()
        };

        assert_eq!(prerequisite_ids(COLIMA_STOP), ["brew"]);
        assert_eq!(
            prerequisite_ids(COLIMA_START),
            ["brew-formulae", "colima-stop"]
        );
        assert_eq!(prerequisite_ids(DOCKER), ["brew-reopen"]);
    }

    #[test]
    fn parses_compose_projects_and_splits_config_files() {
        let json = r#"[
            {"Name": "web", "ConfigFiles": "/srv/web/compose.yml,/srv/web/override.yml"},
            {"Name": "db", "ConfigFiles": "/srv/db/compose.yml"}
        ]"#;

        assert_eq!(
            parse_compose_projects(json),
            [
                ComposeProject::new("web", ["/srv/web/compose.yml", "/srv/web/override.yml"]),
                ComposeProject::new("db", ["/srv/db/compose.yml"]),
            ]
        );
    }

    #[test]
    fn malformed_compose_output_yields_no_projects() {
        for json in ["", "not json", "{}", "[]", "null"] {
            assert!(parse_compose_projects(json).is_empty(), "input: {json:?}");
        }
    }

    #[test]
    fn compose_projects_without_a_name_or_files_are_ignored() {
        let json = r#"[
            {"Name": "", "ConfigFiles": "/srv/a.yml"},
            {"Name": "no-files", "ConfigFiles": ""},
            {"Name": "missing-files"},
            {"ConfigFiles": "/srv/b.yml"},
            {"Name": "kept", "ConfigFiles": "/srv/kept.yml"}
        ]"#;

        assert_eq!(
            parse_compose_projects(json),
            [ComposeProject::new("kept", ["/srv/kept.yml"])]
        );
    }

    #[test]
    fn standalone_containers_exclude_compose_managed_rows() {
        let output = "abc\tweb-1\tweb\ndef\tlonely\t\nghi\tlonely2\t\njkl\tonly-two-fields\n";

        assert_eq!(
            parse_standalone_containers(output),
            [
                StandaloneContainer::new("def", "lonely"),
                StandaloneContainer::new("ghi", "lonely2"),
                StandaloneContainer::new("jkl", "only-two-fields"),
            ]
        );
    }

    #[test]
    fn compose_commands_match_the_zsh_transcript() {
        let project = ComposeProject::new("web", ["/srv/web/compose.yml", "/srv/web/override.yml"]);

        assert_eq!(
            compose_command(&project, &["down"]).arguments(),
            os_strings(&[
                "compose",
                "-p",
                "web",
                "-f",
                "/srv/web/compose.yml",
                "-f",
                "/srv/web/override.yml",
                "down",
            ])
        );
        assert_eq!(
            compose_command(&project, &["pull", "--quiet"]).arguments(),
            os_strings(&[
                "compose",
                "-p",
                "web",
                "-f",
                "/srv/web/compose.yml",
                "-f",
                "/srv/web/override.yml",
                "pull",
                "--quiet",
            ])
        );
    }

    #[test]
    fn ps_format_contains_the_tabs_the_zsh_script_uses() {
        assert_eq!(
            PS_FORMAT,
            "{{.ID}}\t{{.Names}}\t{{.Label \"com.docker.compose.project\"}}"
        );
        assert_eq!(PS_FORMAT.matches('\t').count(), 2);
    }
}
