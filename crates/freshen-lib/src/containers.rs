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

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::apps::quit_command;
use crate::cleanup;
use crate::homebrew;
use crate::homebrew::{BrewInstall, BREW, CASKS, FORMULAE, REOPEN};
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, ProcessError, NETWORK_RETRIES};
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

/// A container runtime that freshen can identify from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerRuntimeKind {
    /// Colima (including the Lima VM it manages).
    Colima,
    /// OrbStack.
    OrbStack,
    /// Docker Desktop.
    DockerDesktop,
    /// Rancher Desktop.
    RancherDesktop,
    /// Podman, including its machine on macOS and Windows.
    Podman,
    /// A native Docker Engine socket, or another Docker-compatible endpoint.
    NativeDocker,
}

impl ContainerRuntimeKind {
    /// Every known runtime.
    pub const ALL: [Self; 6] = [
        Self::Colima,
        Self::OrbStack,
        Self::DockerDesktop,
        Self::RancherDesktop,
        Self::Podman,
        Self::NativeDocker,
    ];

    /// Canonical name accepted by `--container-runtime` and used in reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Colima => "colima",
            Self::OrbStack => "orbstack",
            Self::DockerDesktop => "docker-desktop",
            Self::RancherDesktop => "rancher-desktop",
            Self::Podman => "podman",
            Self::NativeDocker => "docker",
        }
    }

    /// Human-readable name used in the listing and the run summary.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Colima => "Colima",
            Self::OrbStack => "OrbStack",
            Self::DockerDesktop => "Docker Desktop",
            Self::RancherDesktop => "Rancher Desktop",
            Self::Podman => "Podman",
            Self::NativeDocker => "Docker Engine",
        }
    }
}

impl fmt::Display for ContainerRuntimeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContainerRuntimeKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "colima" | "lima" => Ok(Self::Colima),
            "orbstack" | "orb-stack" => Ok(Self::OrbStack),
            "docker-desktop" | "dockerdesktop" | "desktop" => Ok(Self::DockerDesktop),
            "rancher-desktop" | "rancher" | "rd" => Ok(Self::RancherDesktop),
            "podman" | "podman-machine" => Ok(Self::Podman),
            "docker" | "native" | "native-docker" | "docker-engine" => Ok(Self::NativeDocker),
            _ => Err(format!(
                "unknown container runtime `{value}` (known: {})",
                Self::ALL
                    .iter()
                    .map(|kind| kind.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

/// Where the active container runtime was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerRuntimeSource {
    /// The `DOCKER_HOST` environment variable.
    Environment,
    /// `docker context show`.
    DockerContext,
    /// A well-known socket path.
    Socket(PathBuf),
    /// The `--container-runtime` flag.
    Override,
}

impl fmt::Display for ContainerRuntimeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment => f.write_str("DOCKER_HOST"),
            Self::DockerContext => f.write_str("docker context"),
            Self::Socket(path) => write!(f, "{}", path.display()),
            Self::Override => f.write_str("--container-runtime"),
        }
    }
}

/// The container runtime freshen will talk to, and where that choice came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveContainerRuntime {
    kind: ContainerRuntimeKind,
    source: ContainerRuntimeSource,
}

impl ActiveContainerRuntime {
    /// Creates a runtime detected from `source`.
    #[must_use]
    pub fn new(kind: ContainerRuntimeKind, source: ContainerRuntimeSource) -> Self {
        Self { kind, source }
    }

    /// Creates a runtime forced by `--container-runtime`.
    #[must_use]
    pub fn overridden(kind: ContainerRuntimeKind) -> Self {
        Self {
            kind,
            source: ContainerRuntimeSource::Override,
        }
    }

    /// The detected runtime.
    #[must_use]
    pub fn kind(&self) -> ContainerRuntimeKind {
        self.kind
    }

    /// Where the runtime was detected.
    #[must_use]
    pub fn source(&self) -> &ContainerRuntimeSource {
        &self.source
    }
}

impl fmt::Display for ActiveContainerRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.kind.display_name(), self.source)
    }
}

/// The environment and filesystem facts used to detect the active runtime.
///
/// Reading them through a trait keeps the detection rules testable without a
/// real Docker installation: a test injects `DOCKER_HOST`, a context name and
/// a set of existing socket paths.
pub trait ContainerRuntimeProbe {
    /// Reads an environment variable, returning `None` when it is unset or
    /// empty.
    fn variable(&self, name: &str) -> Option<String>;

    /// Returns the name printed by `docker context show`.
    fn docker_context(&self) -> Option<String>;

    /// Returns `true` when `path` exists.
    fn socket_exists(&self, path: &Path) -> bool;
}

/// Reads the container runtime facts from the running host.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostContainerRuntimeProbe;

impl ContainerRuntimeProbe for HostContainerRuntimeProbe {
    fn variable(&self, name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn docker_context(&self) -> Option<String> {
        if !command_exists("docker") {
            return None;
        }

        CommandSpec::new("docker")
            .args(["context", "show"])
            .neutral_locale()
            .run()
            .ok()
            .map(|output| output.stdout().trim().to_owned())
            .filter(|name| !name.is_empty())
    }

    fn socket_exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

/// Detects the active container runtime from the injected facts.
///
/// `DOCKER_HOST` wins over `docker context show`, which wins over the
/// well-known socket paths. Returns `None` when none of them identifies a
/// runtime, so the caller can leave the runtime line out instead of guessing.
#[must_use]
pub fn detect_active_runtime(probe: &dyn ContainerRuntimeProbe) -> Option<ActiveContainerRuntime> {
    if let Some(host) = probe.variable("DOCKER_HOST") {
        return Some(ActiveContainerRuntime::new(
            kind_from_docker_host(&host),
            ContainerRuntimeSource::Environment,
        ));
    }

    if let Some(context) = probe.docker_context() {
        if let Some(kind) = kind_from_context(&context) {
            return Some(ActiveContainerRuntime::new(
                kind,
                ContainerRuntimeSource::DockerContext,
            ));
        }
    }

    for (path, kind) in socket_candidates(probe) {
        if probe.socket_exists(&path) {
            return Some(ActiveContainerRuntime::new(
                kind,
                ContainerRuntimeSource::Socket(path),
            ));
        }
    }

    None
}

fn kind_from_docker_host(host: &str) -> ContainerRuntimeKind {
    let lower = host.to_ascii_lowercase();
    if lower.contains("podman") {
        return ContainerRuntimeKind::Podman;
    }
    if lower.contains("orbstack") {
        return ContainerRuntimeKind::OrbStack;
    }
    if lower.contains("colima") {
        return ContainerRuntimeKind::Colima;
    }
    if lower.contains("rancher") || lower.contains("/.rd/") {
        return ContainerRuntimeKind::RancherDesktop;
    }
    if lower.contains("com.docker.docker")
        || lower.contains("/.docker/")
        || lower.contains("docker-desktop")
        || (lower.contains("npipe") && lower.contains("docker_engine"))
    {
        return ContainerRuntimeKind::DockerDesktop;
    }

    ContainerRuntimeKind::NativeDocker
}

fn kind_from_context(context: &str) -> Option<ContainerRuntimeKind> {
    let name = context.trim().to_ascii_lowercase();
    if name.is_empty() || name == "default" {
        return None;
    }

    if name == "colima" || name.starts_with("colima-") {
        Some(ContainerRuntimeKind::Colima)
    } else if name == "orbstack" || name.starts_with("orbstack-") {
        Some(ContainerRuntimeKind::OrbStack)
    } else if name == "desktop-linux"
        || name == "desktop-windows"
        || name == "docker-desktop"
        || name.starts_with("desktop-")
    {
        Some(ContainerRuntimeKind::DockerDesktop)
    } else if name == "rancher-desktop" || name.starts_with("rancher-") {
        Some(ContainerRuntimeKind::RancherDesktop)
    } else if name == "podman" || name.starts_with("podman-") || name == "rootless" {
        Some(ContainerRuntimeKind::Podman)
    } else {
        None
    }
}

fn socket_candidates(probe: &dyn ContainerRuntimeProbe) -> Vec<(PathBuf, ContainerRuntimeKind)> {
    let home = probe
        .variable("HOME")
        .or_else(|| probe.variable("USERPROFILE"))
        .map(PathBuf::from);
    let runtime_dir = probe.variable("XDG_RUNTIME_DIR").map(PathBuf::from);
    let mut candidates = Vec::new();

    if let Some(home) = &home {
        candidates.extend([
            (
                home.join(".colima/default/docker.sock"),
                ContainerRuntimeKind::Colima,
            ),
            (
                home.join(".colima/docker.sock"),
                ContainerRuntimeKind::Colima,
            ),
            (
                home.join(".orbstack/run/docker.sock"),
                ContainerRuntimeKind::OrbStack,
            ),
            (
                home.join(".docker/run/docker.sock"),
                ContainerRuntimeKind::DockerDesktop,
            ),
            (
                home.join("Library/Containers/com.docker.docker/Data/docker.raw.sock"),
                ContainerRuntimeKind::DockerDesktop,
            ),
            (
                home.join(".rd/docker.sock"),
                ContainerRuntimeKind::RancherDesktop,
            ),
            (
                home.join(".local/share/containers/podman/machine/podman.sock"),
                ContainerRuntimeKind::Podman,
            ),
            (
                home.join(".local/share/containers/podman/machine/qemu/podman.sock"),
                ContainerRuntimeKind::Podman,
            ),
            (
                home.join(".local/share/containers/podman/machine/applehv/podman.sock"),
                ContainerRuntimeKind::Podman,
            ),
        ]);
    }

    if let Some(runtime_dir) = &runtime_dir {
        candidates.push((
            runtime_dir.join("podman/podman.sock"),
            ContainerRuntimeKind::Podman,
        ));
    }

    if let Some(uid) = probe.variable("UID") {
        candidates.push((
            PathBuf::from(format!("/run/user/{uid}/podman/podman.sock")),
            ContainerRuntimeKind::Podman,
        ));
    }

    candidates.extend([
        (
            PathBuf::from("/run/podman/podman.sock"),
            ContainerRuntimeKind::Podman,
        ),
        (
            PathBuf::from("/var/run/docker.sock"),
            ContainerRuntimeKind::NativeDocker,
        ),
        (
            PathBuf::from("/run/docker.sock"),
            ContainerRuntimeKind::NativeDocker,
        ),
    ]);

    candidates
}

/// A container runtime that has to be stopped around an upgrade of its own
/// packages and started again afterwards.
///
/// The Homebrew steps do not know which runtime they are dealing with: the
/// stop and start steps ask the runtime whether it is installed and running,
/// stop and start it, and use the runtime's trigger lists to decide whether an
/// outdated formula or cask needs the restart cycle.
pub trait ContainerRuntime: Send + Sync {
    /// Human-readable name used in progress messages, e.g. `colima/lima`.
    fn name(&self) -> &'static str;

    /// Name shown in the step list, e.g. `Colima/lima`.
    fn display_name(&self) -> &'static str;

    /// Executable that controls the runtime, e.g. `colima`.
    fn command(&self) -> &'static str;

    /// The executables whose presence next to a discovered Homebrew `brew`
    /// marks this runtime as installed.
    ///
    /// A tool installed through Homebrew lives in `{prefix}/bin` next to
    /// `brew`, so `detect_with_brew` also finds a runtime installed only in a
    /// non-`PATH` prefix such as `/opt/homebrew` or `/usr/local`.
    fn detection_commands(&self) -> &'static [&'static str];

    /// Whether the runtime's tool is installed.
    fn detect(&self) -> Detection;

    /// Whether the runtime is currently running.
    fn is_running(&self) -> bool;

    /// Stops the runtime.
    fn stop(&self) -> Result<(), ProcessError>;

    /// Starts the runtime.
    fn start(&self) -> Result<(), ProcessError>;

    /// Formulae whose upgrade needs the runtime to be stopped and started.
    fn restart_formulae(&self) -> &'static [&'static str];

    /// Casks whose upgrade needs the runtime to be stopped and started.
    fn restart_casks(&self) -> &'static [&'static str];

    /// Platforms on which this runtime can be controlled.
    fn platforms(&self) -> &'static [Platform] {
        &CONTAINER_PLATFORMS
    }

    /// Whether upgrading `package` needs a stop/start cycle.
    fn triggers_restart(&self, package: &str) -> bool {
        self.restart_formulae().contains(&package) || self.restart_casks().contains(&package)
    }

    /// Records whether the `lima` formula itself is being upgraded, as opposed
    /// to only `colima`. When `false`, standalone Lima instances are left
    /// running while Colima is stopped and started. Runtimes that do not
    /// manage standalone Lima instances can ignore this.
    fn set_lima_upgrading(&self, _upgrading: bool) {}
}

/// The Colima/lima container runtime.
///
/// Colima manages a Lima VM that runs Docker. Upgrading either the `colima`
/// or the `lima` formula can restart the VM, so both trigger a stop/start
/// cycle. Standalone Lima instances that Colima does not manage are stopped
/// and started through `limactl` only when the `lima` formula itself is being
/// upgraded; on a Colima-only upgrade they keep running.
#[derive(Debug, Default, Clone)]
pub struct ColimaRuntime {
    state: Arc<Mutex<ColimaState>>,
}

#[derive(Debug, Default)]
struct ColimaState {
    observed: bool,
    colima_running: bool,
    standalone_lima: Vec<String>,
    /// Whether the `lima` formula itself is being upgraded, as opposed to only
    /// `colima`. Standalone Lima instances are stopped and started only when
    /// this is `true`; on a Colima-only upgrade they keep running.
    lima_upgrading: bool,
}

impl ColimaRuntime {
    /// Creates the Colima/lima runtime.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ContainerRuntime for ColimaRuntime {
    fn name(&self) -> &'static str {
        "colima/lima"
    }

    fn display_name(&self) -> &'static str {
        "Colima/lima"
    }

    fn command(&self) -> &'static str {
        "colima"
    }

    fn detection_commands(&self) -> &'static [&'static str] {
        &["colima", "limactl"]
    }

    fn detect(&self) -> Detection {
        if command_exists("colima") || command_exists("limactl") {
            Detection::Installed
        } else {
            Detection::NotInstalled
        }
    }

    fn is_running(&self) -> bool {
        let colima_running = colima_is_running();
        let standalone_lima = running_standalone_lima_instances();
        {
            let mut state = lock(&self.state);
            state.observed = true;
            state.colima_running = colima_running;
            state.standalone_lima = standalone_lima.clone();
        }
        colima_running || !standalone_lima.is_empty()
    }

    fn stop(&self) -> Result<(), ProcessError> {
        let (colima_running, standalone_lima, lima_upgrading) = self.observed_state();
        let mut failures = Vec::new();

        if colima_running {
            if let Err(error) = CommandSpec::new("colima").arg("stop").run() {
                failures.push(error);
            }
        }

        if lima_upgrading {
            for instance in &standalone_lima {
                human!("limactl stop: {instance}");
                if let Err(error) = CommandSpec::new("limactl").args(["stop", instance]).run() {
                    failures.push(error);
                }
            }
        }

        first_process_failure(failures)
    }

    fn start(&self) -> Result<(), ProcessError> {
        let (colima_running, standalone_lima, lima_upgrading) = self.observed_state();
        let mut failures = Vec::new();

        if colima_running {
            if let Err(error) = CommandSpec::new("colima").arg("start").run() {
                failures.push(error);
            }
        }

        if lima_upgrading {
            for instance in &standalone_lima {
                human!("limactl start: {instance}");
                if let Err(error) = CommandSpec::new("limactl").args(["start", instance]).run() {
                    failures.push(error);
                }
            }
        }

        first_process_failure(failures)
    }

    fn restart_formulae(&self) -> &'static [&'static str] {
        &["colima", "lima"]
    }

    fn restart_casks(&self) -> &'static [&'static str] {
        &[]
    }

    fn set_lima_upgrading(&self, upgrading: bool) {
        let mut state = lock(&self.state);
        state.lima_upgrading = upgrading;
    }
}

impl ColimaRuntime {
    fn observed_state(&self) -> (bool, Vec<String>, bool) {
        let state = lock(&self.state);
        if state.observed {
            return (
                state.colima_running,
                state.standalone_lima.clone(),
                state.lima_upgrading,
            );
        }
        drop(state);

        (
            colima_is_running(),
            running_standalone_lima_instances(),
            false,
        )
    }
}

fn colima_is_running() -> bool {
    command_exists("colima") && CommandSpec::new("colima").arg("status").run().is_ok()
}

fn running_standalone_lima_instances() -> Vec<String> {
    if !command_exists("limactl") {
        return Vec::new();
    }

    let Ok(output) = CommandSpec::new("limactl")
        .args(["list", "--json"])
        .neutral_locale()
        .run()
    else {
        return Vec::new();
    };

    parse_running_standalone_lima_instances(output.stdout())
}

fn parse_running_standalone_lima_instances(output: &str) -> Vec<String> {
    let mut instances = Vec::new();
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return instances;
    }

    if let Ok(Value::Array(values)) = serde_json::from_str::<Value>(trimmed) {
        for value in &values {
            push_running_standalone_lima_instance(&mut instances, value);
        }
        return instances;
    }

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            push_running_standalone_lima_instance(&mut instances, &value);
        }
    }

    instances
}

fn push_running_standalone_lima_instance(instances: &mut Vec<String>, value: &Value) {
    let Some(name) = value.get("name").and_then(Value::as_str) else {
        return;
    };
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !status.eq_ignore_ascii_case("running") || is_colima_managed_instance(name) {
        return;
    }
    if !instances.iter().any(|existing| existing == name) {
        instances.push(name.to_owned());
    }
}

fn is_colima_managed_instance(name: &str) -> bool {
    name == "colima" || name.starts_with("colima-")
}

fn first_process_failure(failures: Vec<ProcessError>) -> Result<(), ProcessError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The bundle id of the Docker Desktop application.
const DOCKER_DESKTOP_BUNDLE_ID: &str = "com.docker.docker";

/// How long to wait for Docker Desktop to exit after the AppleScript quit.
const DOCKER_DESKTOP_QUIT_TIMEOUT: Duration = Duration::from_secs(5);

/// How often to poll whether Docker Desktop has exited.
const DOCKER_DESKTOP_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Docker Desktop on macOS.
///
/// Docker Desktop ships as the `docker-desktop` cask, so its cask upgrade
/// triggers the stop/start cycle. The `docker desktop` CLI is preferred;
/// older releases do not have it, so stopping falls back to the AppleScript
/// quit that freshen uses for every other app.
#[derive(Debug, Clone, Copy)]
pub struct DockerDesktopRuntime {
    /// How long to wait for Docker Desktop to exit after the AppleScript quit.
    quit_timeout: Duration,
}

impl Default for DockerDesktopRuntime {
    fn default() -> Self {
        Self {
            quit_timeout: DOCKER_DESKTOP_QUIT_TIMEOUT,
        }
    }
}

impl DockerDesktopRuntime {
    /// Creates the Docker Desktop runtime.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Waits for Docker Desktop to exit after it was asked to quit.
    ///
    /// Returns an error with a clear message when it does not stop before the
    /// timeout, so an older Docker Desktop without `docker desktop stop` is
    /// known to have stopped before its cask is upgraded.
    fn wait_until_stopped(&self) -> Result<(), ProcessError> {
        let mut waited = Duration::ZERO;
        while self.is_running() && waited < self.quit_timeout {
            std::thread::sleep(DOCKER_DESKTOP_POLL_INTERVAL);
            waited += DOCKER_DESKTOP_POLL_INTERVAL;
        }

        if self.is_running() {
            return Err(ProcessError::StopTimedOut {
                program: "docker desktop".to_owned(),
                timeout: self.quit_timeout,
            });
        }

        Ok(())
    }
}

impl ContainerRuntime for DockerDesktopRuntime {
    fn name(&self) -> &'static str {
        "docker-desktop"
    }

    fn display_name(&self) -> &'static str {
        "Docker Desktop"
    }

    fn command(&self) -> &'static str {
        "docker desktop"
    }

    fn detection_commands(&self) -> &'static [&'static str] {
        &["docker"]
    }

    fn detect(&self) -> Detection {
        if command_exists("docker") {
            Detection::Installed
        } else {
            Detection::NotInstalled
        }
    }

    fn is_running(&self) -> bool {
        match CommandSpec::new("docker").args(["desktop", "status"]).run() {
            Ok(output) => {
                let status = output.stdout().trim().to_ascii_lowercase();
                status.is_empty()
                    || (!status.contains("stopped") && !status.contains("not running"))
            }
            Err(_) => false,
        }
    }

    fn stop(&self) -> Result<(), ProcessError> {
        match CommandSpec::new("docker").args(["desktop", "stop"]).run() {
            Ok(_) => Ok(()),
            Err(error) => {
                // Older releases lack `docker desktop stop`, so ask Docker
                // Desktop to quit through AppleScript and wait for it to
                // actually exit before the upgrade proceeds.
                if quit_command(DOCKER_DESKTOP_BUNDLE_ID).run().is_err() {
                    return Err(error);
                }
                self.wait_until_stopped()?;
                Ok(())
            }
        }
    }

    fn start(&self) -> Result<(), ProcessError> {
        match CommandSpec::new("docker").args(["desktop", "start"]).run() {
            Ok(_) => Ok(()),
            Err(error) => {
                // Older releases lack `docker desktop start`, so start Docker
                // Desktop by its app bundle instead.
                CommandSpec::new("open")
                    .arg("-a")
                    .arg(DOCKER_DESKTOP_BUNDLE_ID)
                    .run()
                    .map(|_| ())
                    .map_err(|_| error)
            }
        }
    }

    fn restart_formulae(&self) -> &'static [&'static str] {
        &[]
    }

    fn restart_casks(&self) -> &'static [&'static str] {
        &["docker-desktop"]
    }
}

/// The facts and commands used to control a native Docker Engine through
/// systemd.
///
/// Reading them through a trait keeps the runtime testable without a real
/// Docker installation: a test injects whether the tools exist, whether the
/// `docker` unit is active and whether live-restore is enabled, and records the
/// systemctl actions.
pub trait DockerEngineProbe: Send + Sync {
    /// Returns `true` when `program` is an executable on `PATH`.
    fn command_exists(&self, program: &str) -> bool;

    /// Returns `true` when the systemd unit is active.
    fn unit_active(&self, unit: &str) -> bool;

    /// Returns the Docker daemon's live-restore setting, or `None` when it
    /// cannot be read.
    fn live_restore_enabled(&self) -> Option<bool>;

    /// Runs a systemctl action such as `stop` or `restart` for `unit`.
    fn run_systemctl(&self, action: &str, unit: &str) -> Result<(), ProcessError>;
}

/// Reads the native Docker Engine facts from the running host.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostDockerEngineProbe;

impl DockerEngineProbe for HostDockerEngineProbe {
    fn command_exists(&self, program: &str) -> bool {
        crate::executable::command_exists(program)
    }

    fn unit_active(&self, unit: &str) -> bool {
        if !self.command_exists("systemctl") {
            return false;
        }

        CommandSpec::new("systemctl")
            .args(["is-active", "--quiet", unit])
            .run()
            .is_ok()
    }

    fn live_restore_enabled(&self) -> Option<bool> {
        if !self.command_exists("docker") {
            return None;
        }

        let output = CommandSpec::new("docker")
            .args(["info", "--format", "{{.LiveRestoreEnabled}}"])
            .neutral_locale()
            .run()
            .ok()?;
        parse_live_restore_enabled(output.stdout())
    }

    fn run_systemctl(&self, action: &str, unit: &str) -> Result<(), ProcessError> {
        CommandSpec::new("systemctl")
            .args([action, unit])
            .run()
            .map(|_| ())
    }
}

fn parse_live_restore_enabled(output: &str) -> Option<bool> {
    match output.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Warns that a Docker restart interrupts running containers when the daemon
/// does not keep them alive during the restart.
const LIVE_RESTORE_WARNING: &str =
    "warning: Docker live-restore is off; restarting Docker will stop running containers.";

fn live_restore_warning(enabled: Option<bool>) -> Option<&'static str> {
    (enabled == Some(false)).then_some(LIVE_RESTORE_WARNING)
}

/// A native Docker Engine managed by systemd on Linux.
///
/// Upgrading the `docker` formula needs the daemon to be restarted. The unit is
/// stopped before the upgrade and restarted afterwards. When live-restore is
/// off, restarting the daemon stops every running container, so the runtime
/// warns about that first.
pub struct NativeDockerRuntime {
    probe: Arc<dyn DockerEngineProbe>,
}

impl NativeDockerRuntime {
    /// Creates the native Docker Engine runtime.
    #[must_use]
    pub fn new() -> Self {
        Self {
            probe: Arc::new(HostDockerEngineProbe),
        }
    }

    /// Creates the runtime with injected facts and commands.
    #[must_use]
    pub fn with_probe(probe: impl DockerEngineProbe + 'static) -> Self {
        Self {
            probe: Arc::new(probe),
        }
    }
}

impl Default for NativeDockerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ContainerRuntime for NativeDockerRuntime {
    fn name(&self) -> &'static str {
        "docker-engine"
    }

    fn display_name(&self) -> &'static str {
        "Docker Engine"
    }

    fn command(&self) -> &'static str {
        "docker"
    }

    fn detection_commands(&self) -> &'static [&'static str] {
        &["docker"]
    }

    fn detect(&self) -> Detection {
        if self.probe.command_exists("docker") && self.probe.command_exists("systemctl") {
            Detection::Installed
        } else {
            Detection::NotInstalled
        }
    }

    fn is_running(&self) -> bool {
        self.probe.unit_active("docker")
    }

    fn stop(&self) -> Result<(), ProcessError> {
        if let Some(warning) = live_restore_warning(self.probe.live_restore_enabled()) {
            human!("{warning}");
        }
        self.probe.run_systemctl("stop", "docker")
    }

    fn start(&self) -> Result<(), ProcessError> {
        self.probe.run_systemctl("restart", "docker")
    }

    fn restart_formulae(&self) -> &'static [&'static str] {
        &["docker"]
    }

    fn restart_casks(&self) -> &'static [&'static str] {
        &[]
    }

    fn platforms(&self) -> &'static [Platform] {
        &[Platform::Linux]
    }
}

/// The OrbStack application on macOS.
///
/// OrbStack ships as the `orbstack` cask, so its cask upgrade triggers the
/// stop/start cycle. `orbctl status` reports whether the VM is running, and
/// `orbctl stop` / `orbctl start` control it.
#[derive(Debug, Default, Clone, Copy)]
pub struct OrbStackRuntime;

impl OrbStackRuntime {
    /// Creates the OrbStack runtime.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ContainerRuntime for OrbStackRuntime {
    fn name(&self) -> &'static str {
        "orbstack"
    }

    fn display_name(&self) -> &'static str {
        "OrbStack"
    }

    fn command(&self) -> &'static str {
        "orbctl"
    }

    fn detection_commands(&self) -> &'static [&'static str] {
        &["orbctl"]
    }

    fn detect(&self) -> Detection {
        if command_exists(self.command()) {
            Detection::Installed
        } else {
            Detection::NotInstalled
        }
    }

    fn is_running(&self) -> bool {
        match CommandSpec::new(self.command()).arg("status").run() {
            Ok(output) => {
                let status = output.stdout().trim().to_ascii_lowercase();
                status.is_empty()
                    || (!status.contains("stopped") && !status.contains("not running"))
            }
            Err(_) => false,
        }
    }

    fn stop(&self) -> Result<(), ProcessError> {
        CommandSpec::new(self.command())
            .arg("stop")
            .run()
            .map(|_| ())
    }

    fn start(&self) -> Result<(), ProcessError> {
        CommandSpec::new(self.command())
            .arg("start")
            .run()
            .map(|_| ())
    }

    fn restart_formulae(&self) -> &'static [&'static str] {
        &[]
    }

    fn restart_casks(&self) -> &'static [&'static str] {
        &["orbstack"]
    }
}

/// The Homebrew cask that installs Rancher Desktop.
const RANCHER_DESKTOP_CASK: &str = "rancher";

/// Rancher Desktop on macOS.
///
/// Rancher Desktop ships as the `rancher` cask, so its cask upgrade triggers
/// the stop/start cycle. `rdctl shutdown` stops the application and
/// `rdctl start` brings it back; `rdctl list-settings` only succeeds while the
/// application is running, which is what `is_running` relies on.
#[derive(Debug, Default, Clone, Copy)]
pub struct RancherDesktopRuntime;

impl RancherDesktopRuntime {
    /// Creates the Rancher Desktop runtime.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ContainerRuntime for RancherDesktopRuntime {
    fn name(&self) -> &'static str {
        "rancher-desktop"
    }

    fn display_name(&self) -> &'static str {
        "Rancher Desktop"
    }

    fn command(&self) -> &'static str {
        "rdctl"
    }

    fn detection_commands(&self) -> &'static [&'static str] {
        &["rdctl"]
    }

    fn detect(&self) -> Detection {
        if command_exists(self.command()) {
            Detection::Installed
        } else {
            Detection::NotInstalled
        }
    }

    fn is_running(&self) -> bool {
        CommandSpec::new(self.command())
            .arg("list-settings")
            .run()
            .is_ok()
    }

    fn stop(&self) -> Result<(), ProcessError> {
        CommandSpec::new(self.command())
            .arg("shutdown")
            .run()
            .map(|_| ())
    }

    fn start(&self) -> Result<(), ProcessError> {
        CommandSpec::new(self.command())
            .arg("start")
            .run()
            .map(|_| ())
    }

    fn restart_formulae(&self) -> &'static [&'static str] {
        &[]
    }

    fn restart_casks(&self) -> &'static [&'static str] {
        &[RANCHER_DESKTOP_CASK]
    }
}

/// The Podman machine on macOS and Windows.
///
/// Podman runs containers inside a virtual machine. Upgrading the `podman`
/// formula can restart that machine, so the formula triggers the stop/start
/// cycle. `podman machine list --format json` reports whether a machine is
/// running, and `podman machine stop` / `podman machine start` control it.
#[derive(Debug, Default, Clone, Copy)]
pub struct PodmanRuntime;

impl PodmanRuntime {
    /// Creates the Podman machine runtime.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ContainerRuntime for PodmanRuntime {
    fn name(&self) -> &'static str {
        "podman-machine"
    }

    fn display_name(&self) -> &'static str {
        "Podman machine"
    }

    fn command(&self) -> &'static str {
        "podman machine"
    }

    fn detection_commands(&self) -> &'static [&'static str] {
        &["podman"]
    }

    fn detect(&self) -> Detection {
        if command_exists("podman") {
            Detection::Installed
        } else {
            Detection::NotInstalled
        }
    }

    fn is_running(&self) -> bool {
        let Ok(output) = CommandSpec::new("podman")
            .args(["machine", "list", "--format", "json"])
            .neutral_locale()
            .run()
        else {
            return false;
        };
        podman_machine_running(output.stdout())
    }

    fn stop(&self) -> Result<(), ProcessError> {
        CommandSpec::new("podman")
            .args(["machine", "stop"])
            .run()
            .map(|_| ())
    }

    fn start(&self) -> Result<(), ProcessError> {
        CommandSpec::new("podman")
            .args(["machine", "start"])
            .run()
            .map(|_| ())
    }

    fn restart_formulae(&self) -> &'static [&'static str] {
        &["podman"]
    }

    fn restart_casks(&self) -> &'static [&'static str] {
        &[]
    }
}

fn podman_machine_running(json: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return false;
    };
    value
        .as_array()
        .map(|machines| {
            machines.iter().any(|machine| {
                machine
                    .get("Running")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// The compose implementation freshen drives.
///
/// Docker ships Compose v2 as the `docker compose` plugin, older installations
/// have the standalone `docker-compose` binary, and Podman provides
/// `podman compose`. The detected command is used for every compose action so
/// a run never mixes implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeCommand {
    /// The `docker compose` CLI plugin.
    DockerCompose,
    /// The standalone `docker-compose` binary.
    DockerComposeStandalone,
    /// `podman compose`.
    PodmanCompose,
}

impl ComposeCommand {
    /// Every known compose command, in detection order.
    pub const ALL: [Self; 3] = [
        Self::DockerCompose,
        Self::DockerComposeStandalone,
        Self::PodmanCompose,
    ];

    /// Name used in progress messages and reports, e.g. `docker compose`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DockerCompose => "docker compose",
            Self::DockerComposeStandalone => "docker-compose",
            Self::PodmanCompose => "podman compose",
        }
    }

    /// Executable that runs the compose implementation.
    #[must_use]
    pub fn program(self) -> &'static str {
        match self {
            Self::DockerCompose => "docker",
            Self::DockerComposeStandalone => "docker-compose",
            Self::PodmanCompose => "podman",
        }
    }

    fn prefix(self) -> &'static [&'static str] {
        match self {
            Self::DockerCompose | Self::PodmanCompose => &["compose"],
            Self::DockerComposeStandalone => &[],
        }
    }

    /// Builds a compose command for `project` with `arguments`.
    #[must_use]
    pub fn command(self, project: &ComposeProject, arguments: &[&str]) -> CommandSpec {
        let mut spec = CommandSpec::new(self.program()).args(self.prefix());
        spec = spec.args(["-p"]).arg(project.name());
        for file in project.config_files() {
            spec = spec.arg("-f").arg(file.as_str());
        }
        spec.args(arguments)
            .retry_transient_network(NETWORK_RETRIES)
    }

    /// Builds the command that lists running compose projects as JSON.
    #[must_use]
    pub fn list_command(self) -> CommandSpec {
        CommandSpec::new(self.program())
            .args(self.prefix())
            .args(["ls", "--format", "json"])
            .neutral_locale()
    }
}

impl fmt::Display for ComposeCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The command lookups used to detect the compose implementation.
///
/// Reading them through a trait keeps detection testable without a real
/// container toolchain: a test injects which programs exist and which ones
/// provide a working `compose` subcommand.
pub trait ComposeCommandProbe {
    /// Returns `true` when `program` is an executable on `PATH`.
    fn command_exists(&self, program: &str) -> bool;

    /// Returns `true` when `program compose version` succeeds.
    fn supports_compose(&self, program: &str) -> bool;
}

/// Reads the compose command facts from the running host.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostComposeCommandProbe;

impl ComposeCommandProbe for HostComposeCommandProbe {
    fn command_exists(&self, program: &str) -> bool {
        crate::executable::command_exists(program)
    }

    fn supports_compose(&self, program: &str) -> bool {
        if !self.command_exists(program) {
            return false;
        }

        CommandSpec::new(program)
            .args(["compose", "version"])
            .neutral_locale()
            .run()
            .is_ok()
    }
}

/// Detects the compose command freshen should use.
///
/// When Podman is the active runtime, `podman compose` is preferred even if a
/// Docker compose implementation is also installed, so a forced
/// `--container-runtime podman` run never falls back to the Docker CLI.
/// Otherwise `docker compose` wins when the Docker CLI has a working compose
/// plugin, then the standalone `docker-compose` binary, then `podman compose`.
/// Returns `None` when no implementation is available.
#[must_use]
pub fn detect_compose_command(
    probe: &dyn ComposeCommandProbe,
    runtime: Option<ContainerRuntimeKind>,
) -> Option<ComposeCommand> {
    if runtime == Some(ContainerRuntimeKind::Podman) && probe.supports_compose("podman") {
        return Some(ComposeCommand::PodmanCompose);
    }
    if probe.supports_compose("docker") {
        Some(ComposeCommand::DockerCompose)
    } else if probe.command_exists("docker-compose") {
        Some(ComposeCommand::DockerComposeStandalone)
    } else if probe.supports_compose("podman") {
        Some(ComposeCommand::PodmanCompose)
    } else {
        None
    }
}

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
    register_with_runtime(registry, ContainerRuntimeKind::Colima, ColimaRuntime::new())
}

/// Registers the container steps for a specific container runtime.
#[must_use]
pub fn register_with_runtime(
    registry: Registry,
    kind: ContainerRuntimeKind,
    runtime: impl ContainerRuntime + 'static,
) -> Registry {
    register_with_compose(
        registry,
        runtime,
        detect_compose_command(&HostComposeCommandProbe, Some(kind)),
        container_cli(kind),
    )
}

/// The CLI used to list and control standalone containers for a runtime.
///
/// Colima, OrbStack, Docker Desktop, Rancher Desktop and native Docker Engine
/// all expose a Docker-compatible CLI, so standalone containers are managed
/// with `docker`. Podman exposes its own CLI, so a Podman-only host without
/// the Docker CLI uses `podman` instead.
#[must_use]
fn container_cli(kind: ContainerRuntimeKind) -> &'static str {
    match kind {
        ContainerRuntimeKind::Podman => "podman",
        _ => "docker",
    }
}

/// Registers the container steps for a specific runtime and compose command.
///
/// The compose command is resolved once and shared by the stop, start and
/// image steps, so `pull`, `up` and `down` always use the same implementation.
#[must_use]
pub fn register_with_compose(
    registry: Registry,
    runtime: impl ContainerRuntime + 'static,
    compose: Option<ComposeCommand>,
    container_cli: &'static str,
) -> Registry {
    let runtime: Arc<dyn ContainerRuntime> = Arc::new(runtime);
    let snapshot = ContainerSnapshot::default();
    registry
        .with(ContainerStopStep::new(
            Arc::clone(&runtime),
            snapshot.clone(),
            compose,
            container_cli,
        ))
        .with(ContainerStartStep::new(
            Arc::clone(&runtime),
            snapshot,
            compose,
            container_cli,
        ))
        .with(DockerStep::new(runtime, compose, container_cli))
}

/// Registers the container steps for the detected runtime.
///
/// A runtime that has no implementation yet keeps the Colima/lima steps, so
/// detection never removes container handling from a run. Colima/lima,
/// OrbStack, Docker Desktop, Rancher Desktop, Podman and native Docker Engine
/// have their own implementations.
#[must_use]
pub fn register_for(registry: Registry, kind: Option<ContainerRuntimeKind>) -> Registry {
    match kind {
        Some(ContainerRuntimeKind::OrbStack) => {
            register_with_runtime(registry, ContainerRuntimeKind::OrbStack, OrbStackRuntime)
        }
        Some(ContainerRuntimeKind::DockerDesktop) => register_with_runtime(
            registry,
            ContainerRuntimeKind::DockerDesktop,
            DockerDesktopRuntime::default(),
        ),
        Some(ContainerRuntimeKind::RancherDesktop) => register_with_runtime(
            registry,
            ContainerRuntimeKind::RancherDesktop,
            RancherDesktopRuntime,
        ),
        Some(ContainerRuntimeKind::Podman) => {
            register_with_runtime(registry, ContainerRuntimeKind::Podman, PodmanRuntime)
        }
        Some(ContainerRuntimeKind::NativeDocker) => register_with_runtime(
            registry,
            ContainerRuntimeKind::NativeDocker,
            NativeDockerRuntime::new(),
        ),
        _ => register(registry),
    }
}

#[derive(Debug, Default, Clone)]
struct Snapshot {
    compose: Vec<ComposeProject>,
    standalone: Vec<StandaloneContainer>,
}

#[derive(Clone, Default)]
struct ContainerSnapshot(Arc<Mutex<Option<Snapshot>>>);

impl ContainerSnapshot {
    fn store(&self, snapshot: Snapshot) {
        *lock(&self.0) = Some(snapshot);
    }

    fn get(&self) -> Option<Snapshot> {
        lock(&self.0).clone()
    }

    fn clear(&self) {
        *lock(&self.0) = None;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct ContainerStopStep {
    runtime: Arc<dyn ContainerRuntime>,
    snapshot: ContainerSnapshot,
    compose: Option<ComposeCommand>,
    container_cli: &'static str,
    display_name: String,
}

impl ContainerStopStep {
    fn new(
        runtime: Arc<dyn ContainerRuntime>,
        snapshot: ContainerSnapshot,
        compose: Option<ComposeCommand>,
        container_cli: &'static str,
    ) -> Self {
        let display_name = format!("{} stop", runtime.display_name());
        Self {
            runtime,
            snapshot,
            compose,
            container_cli,
            display_name,
        }
    }

    fn upgrade_triggers_restart(&self, installs: &[BrewInstall]) -> Result<bool, StepError> {
        let outdated = self.outdated_restart_packages(installs)?;
        let triggers_restart = outdated
            .iter()
            .any(|name| self.runtime.triggers_restart(name));
        // Standalone Lima instances are stopped and started only when the
        // `lima` formula itself is being upgraded; on a Colima-only upgrade
        // they keep running.
        self.runtime
            .set_lima_upgrading(outdated.iter().any(|name| name == "lima"));
        Ok(triggers_restart)
    }

    /// Every outdated formula or cask whose upgrade would restart the runtime,
    /// across all discovered Homebrew installations.
    fn outdated_restart_packages(
        &self,
        installs: &[BrewInstall],
    ) -> Result<Vec<String>, StepError> {
        let mut outdated = Vec::new();
        for install in installs {
            if !self.runtime.restart_formulae().is_empty() {
                outdated
                    .extend(self.outdated_through(install, &["outdated", "--formula", "--quiet"])?);
            }
            if !self.runtime.restart_casks().is_empty() {
                outdated.extend(
                    self.outdated_through(install, &["outdated", "--cask", "--greedy", "--quiet"])?,
                );
            }
        }
        Ok(outdated)
    }

    fn outdated_through(
        &self,
        install: &BrewInstall,
        arguments: &[&str],
    ) -> Result<Vec<String>, StepError> {
        let output = install
            .command()
            .args(arguments)
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::run_from_process(COLIMA_STOP, &error))?;
        Ok(lines(output.stdout()))
    }
}

impl Step for ContainerStopStep {
    fn id(&self) -> StepId {
        COLIMA_STOP
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_with_brew(self.runtime.as_ref()))
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if !self.upgrade_triggers_restart(&homebrew::brew_installs())? || !self.runtime.is_running()
        {
            return Ok(());
        }

        human!(
            "{} is outdated and {} is running: stopping containers and {}",
            self.runtime.name(),
            self.runtime.command(),
            self.runtime.command()
        );

        let mut snapshot = Snapshot::default();
        let mut failures = Vec::new();

        if let Some(compose) = self.compose {
            snapshot.compose = selected_projects(list_compose_projects(compose), options);
        }
        if command_exists(self.container_cli) {
            snapshot.standalone =
                selected_containers(list_standalone_containers(self.container_cli), options);
        }

        self.snapshot.store(snapshot.clone());

        if let Some(compose) = self.compose {
            for project in &snapshot.compose {
                human!("{} down: {}", compose.as_str(), project.name());
                if let Err(error) = compose.command(project, &["down"]).run() {
                    failures.push(StepError::run_from_process(COLIMA_STOP, &error));
                }
            }
        }
        for container in &snapshot.standalone {
            human!("{} stop: {}", self.container_cli, container.name());
            if let Err(error) =
                container_command(self.container_cli, &["stop"], container.id()).run()
            {
                failures.push(StepError::run_from_process(COLIMA_STOP, &error));
            }
        }

        if let Err(error) = self.runtime.stop() {
            failures.push(StepError::run_from_process(COLIMA_STOP, &error));
        }

        finish(failures)
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn restore(&self) -> Result<(), StepError> {
        restore_runtime(
            self.runtime.as_ref(),
            &self.snapshot,
            COLIMA_STOP,
            self.compose,
            self.container_cli,
        )
    }

    fn prerequisites(&self) -> &'static [StepId] {
        if self.runtime.restart_formulae().is_empty() && !self.runtime.restart_casks().is_empty() {
            &[FORMULAE]
        } else {
            &[BREW]
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        self.runtime.platforms()
    }
}

struct ContainerStartStep {
    runtime: Arc<dyn ContainerRuntime>,
    snapshot: ContainerSnapshot,
    compose: Option<ComposeCommand>,
    container_cli: &'static str,
    display_name: String,
}

impl ContainerStartStep {
    fn new(
        runtime: Arc<dyn ContainerRuntime>,
        snapshot: ContainerSnapshot,
        compose: Option<ComposeCommand>,
        container_cli: &'static str,
    ) -> Self {
        let display_name = format!("{} start", runtime.display_name());
        Self {
            runtime,
            snapshot,
            compose,
            container_cli,
            display_name,
        }
    }

    fn start_and_restore(&self, action: &str) -> Result<(), StepError> {
        let Some(snapshot) = self.snapshot.get() else {
            return Ok(());
        };

        human!("{action} {} and containers", self.runtime.command());
        let mut failures = Vec::new();

        if let Err(error) = self.runtime.start() {
            human!(
                "warning: {} did not start; containers were NOT restarted",
                self.runtime.command()
            );
            return Err(StepError::run_from_process(COLIMA_START, &error));
        }

        for container in &snapshot.standalone {
            human!("{} start: {}", self.container_cli, container.name());
            if let Err(error) =
                container_command(self.container_cli, &["start"], container.id()).run()
            {
                failures.push(StepError::run_from_process(COLIMA_START, &error));
            }
        }
        if let Some(compose) = self.compose {
            for project in &snapshot.compose {
                human!("{} up -d: {}", compose.as_str(), project.name());
                if let Err(error) = compose.command(project, &["up", "-d"]).run() {
                    failures.push(StepError::run_from_process(COLIMA_START, &error));
                }
            }
        }

        let result = finish(failures);
        if result.is_ok() {
            self.snapshot.clear();
        }
        result
    }
}

fn restore_runtime(
    runtime: &dyn ContainerRuntime,
    snapshot: &ContainerSnapshot,
    step: StepId,
    compose: Option<ComposeCommand>,
    container_cli: &'static str,
) -> Result<(), StepError> {
    let Some(stopped) = snapshot.get() else {
        return Ok(());
    };

    human!("Restoring {} and containers", runtime.command());
    let mut failures = Vec::new();

    if let Err(error) = runtime.start() {
        human!(
            "warning: {} did not start; containers were NOT restarted",
            runtime.command()
        );
        return Err(StepError::run_from_process(step, &error));
    }

    for container in &stopped.standalone {
        human!("{} start: {}", container_cli, container.name());
        if let Err(error) = container_command(container_cli, &["start"], container.id()).run() {
            failures.push(StepError::run_from_process(step, &error));
        }
    }
    if let Some(compose) = compose {
        for project in &stopped.compose {
            human!("{} up -d: {}", compose.as_str(), project.name());
            if let Err(error) = compose.command(project, &["up", "-d"]).run() {
                failures.push(StepError::run_from_process(step, &error));
            }
        }
    }

    let result = finish(failures);
    if result.is_ok() {
        snapshot.clear();
    }
    result
}

impl Step for ContainerStartStep {
    fn id(&self) -> StepId {
        COLIMA_START
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_with_brew(self.runtime.as_ref()))
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.start_and_restore("Starting")
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn restore(&self) -> Result<(), StepError> {
        self.start_and_restore("Restoring")
    }

    fn prerequisites(&self) -> &'static [StepId] {
        if self.runtime.restart_casks().is_empty() {
            &[FORMULAE, COLIMA_STOP]
        } else {
            &[CASKS, COLIMA_STOP]
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        self.runtime.platforms()
    }
}

struct DockerStep {
    runtime: Arc<dyn ContainerRuntime>,
    compose: Option<ComposeCommand>,
    container_cli: &'static str,
}

impl DockerStep {
    fn new(
        runtime: Arc<dyn ContainerRuntime>,
        compose: Option<ComposeCommand>,
        container_cli: &'static str,
    ) -> Self {
        Self {
            runtime,
            compose,
            container_cli,
        }
    }
}

impl Step for DockerStep {
    fn id(&self) -> StepId {
        DOCKER
    }

    fn display_name(&self) -> &str {
        "Docker images"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        let detected = if (command_exists("docker") || self.compose.is_some())
            && self.runtime.detect().is_installed()
        {
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
        if !self.runtime.is_running() {
            human!(
                "warning: {} is not running, skipping Docker image updates.",
                self.runtime.command()
            );
            return Ok(());
        }

        human!("Docker: pull newer images for running compose projects");
        let projects = self
            .compose
            .map(|compose| selected_projects(list_compose_projects(compose), options))
            .unwrap_or_default();
        let mut failures = Vec::new();

        if projects.is_empty() {
            human!("No running compose projects.");
        } else if let Some(compose) = self.compose {
            for project in &projects {
                human!("{} pull + up -d: {}", compose.as_str(), project.name());
                if let Err(error) = compose.command(project, &["pull", "--quiet"]).run() {
                    failures.push(StepError::run_from_process(DOCKER, &error));
                }
                if let Err(error) = compose.command(project, &["up", "-d"]).run() {
                    failures.push(StepError::run_from_process(DOCKER, &error));
                }
            }
        }

        if command_exists(self.container_cli) {
            if let Err(error) = cleanup::run(
                DOCKER,
                "Docker: prune dangling images",
                CommandSpec::new(self.container_cli)
                    .args(["image", "prune", "-f"])
                    .neutral_locale(),
                options,
            ) {
                failures.push(error);
            }
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
        self.runtime.platforms()
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

fn list_compose_projects(compose: ComposeCommand) -> Vec<ComposeProject> {
    let Ok(output) = compose.list_command().run() else {
        return Vec::new();
    };
    parse_compose_projects(output.stdout())
}

fn list_standalone_containers(container_cli: &'static str) -> Vec<StandaloneContainer> {
    let Ok(output) = CommandSpec::new(container_cli)
        .arg("ps")
        .args(["--format", PS_FORMAT])
        .neutral_locale()
        .run()
    else {
        return Vec::new();
    };
    parse_standalone_containers(output.stdout())
}

/// Keeps the compose projects the project filter allows.
///
/// A project that is filtered out is never stopped, pulled or started, so it
/// keeps running while the container runtime is upgraded.
fn selected_projects(projects: Vec<ComposeProject>, options: &RunOptions) -> Vec<ComposeProject> {
    let filter = options.filters().projects();
    if filter.is_empty() {
        return projects;
    }

    projects
        .into_iter()
        .filter(|project| filter.includes(project.name()))
        .collect()
}

/// Keeps the standalone containers the container filter allows.
///
/// A container is named by its name or by its id, so either one can be
/// excluded or included.
fn selected_containers(
    containers: Vec<StandaloneContainer>,
    options: &RunOptions,
) -> Vec<StandaloneContainer> {
    let filter = options.filters().containers();
    if filter.is_empty() {
        return containers;
    }

    containers
        .into_iter()
        .filter(|container| {
            let names = [container.name(), container.id()];
            !names.iter().any(|name| filter.is_excluded(name))
                && (filter.include().is_empty()
                    || names.iter().any(|name| filter.is_included(name)))
        })
        .collect()
}

fn container_command(container_cli: &'static str, arguments: &[&str], id: &str) -> CommandSpec {
    CommandSpec::new(container_cli).args(arguments).arg(id)
}

fn detect_with_brew(runtime: &dyn ContainerRuntime) -> Detection {
    let installs = homebrew::brew_installs();
    if installs.is_empty() {
        return Detection::NotInstalled;
    }

    let installed = installs.iter().any(|install| {
        runtime
            .detection_commands()
            .iter()
            .any(|command| command_next_to_brew(install, command))
    });

    if installed {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Whether `command` is executable next to a discovered Homebrew `brew`.
///
/// A tool installed through Homebrew lives in `{prefix}/bin`, the same
/// directory as the installation's `brew`. A `PATH` installation keeps its
/// tools in the directory that also contains `brew`, so this covers both a
/// non-`PATH` prefix such as `/opt/homebrew` or `/usr/local` and the
/// `brew` found through `PATH`.
fn command_next_to_brew(install: &BrewInstall, command: &str) -> bool {
    let Some(bin) = install.program.parent() else {
        return false;
    };
    crate::executable::resolve(bin.join(command)).is_some()
}

fn command_exists(program: &str) -> bool {
    crate::executable::command_exists(program)
}

#[cfg(test)]
mod tests {
    use super::{
        detect_active_runtime, detect_compose_command, live_restore_warning,
        parse_compose_projects, parse_live_restore_enabled,
        parse_running_standalone_lima_instances, parse_standalone_containers,
        podman_machine_running, register, register_for, register_with_runtime,
        ActiveContainerRuntime, ColimaRuntime, ComposeCommand, ComposeCommandProbe, ComposeProject,
        ContainerRuntime, ContainerRuntimeKind, ContainerRuntimeProbe, ContainerRuntimeSource,
        DockerDesktopRuntime, DockerEngineProbe, NativeDockerRuntime, OrbStackRuntime,
        PodmanRuntime, RancherDesktopRuntime, StandaloneContainer, COLIMA_START, COLIMA_STOP,
        DOCKER, PS_FORMAT,
    };
    use crate::homebrew;
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::process::ProcessError;
    use crate::registry::Registry;
    use crate::step::{Detection, StepId};
    use std::collections::{HashMap, HashSet};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

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

    fn options_with(projects: &[&str], containers: &[&str]) -> RunOptions {
        use crate::policy::{ItemFilters, NameFilter};

        RunOptions::new().with_filters(
            ItemFilters::new()
                .with_projects(NameFilter::new().with_exclude(projects.to_vec()))
                .with_containers(NameFilter::new().with_exclude(containers.to_vec())),
        )
    }

    fn names(projects: &[ComposeProject]) -> Vec<&str> {
        projects.iter().map(ComposeProject::name).collect()
    }

    fn container_names(containers: &[StandaloneContainer]) -> Vec<&str> {
        containers.iter().map(StandaloneContainer::name).collect()
    }

    #[test]
    fn an_excluded_project_is_not_stopped_or_started() {
        let projects = vec![
            ComposeProject::new("web", Vec::<String>::new()),
            ComposeProject::new("db", Vec::<String>::new()),
        ];

        let selected = super::selected_projects(projects, &options_with(&["db"], &[]));

        assert_eq!(names(&selected), ["web"]);
    }

    #[test]
    fn a_project_that_is_not_excluded_is_kept() {
        let projects = vec![ComposeProject::new("web", Vec::<String>::new())];

        let selected = super::selected_projects(projects, &options_with(&["db"], &[]));

        assert_eq!(names(&selected), ["web"]);
    }

    #[test]
    fn an_excluded_container_is_not_stopped_or_started() {
        let containers = vec![
            StandaloneContainer::new("def", "lonely"),
            StandaloneContainer::new("ghi", "lonely2"),
        ];

        let selected = super::selected_containers(containers, &options_with(&[], &["lonely2"]));

        assert_eq!(container_names(&selected), ["lonely"]);
    }

    #[test]
    fn a_container_can_be_excluded_by_its_id() {
        let containers = vec![
            StandaloneContainer::new("def", "lonely"),
            StandaloneContainer::new("ghi", "lonely2"),
        ];

        let selected = super::selected_containers(containers, &options_with(&[], &["ghi"]));

        assert_eq!(container_names(&selected), ["lonely"]);
    }

    #[test]
    fn without_a_filter_every_container_is_kept() {
        let containers = vec![
            StandaloneContainer::new("def", "lonely"),
            StandaloneContainer::new("ghi", "lonely2"),
        ];

        let selected = super::selected_containers(containers, &options_with(&[], &[]));

        assert_eq!(container_names(&selected), ["lonely", "lonely2"]);
    }

    #[test]
    fn registry_stops_colima_before_formulae_and_starts_it_afterwards() {
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
            ]
        );
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
    fn docker_compose_commands_match_the_zsh_transcript() {
        let project = ComposeProject::new("web", ["/srv/web/compose.yml", "/srv/web/override.yml"]);

        assert_eq!(
            ComposeCommand::DockerCompose
                .command(&project, &["down"])
                .arguments(),
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
            ComposeCommand::DockerCompose
                .command(&project, &["pull", "--quiet"])
                .arguments(),
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
    fn every_compose_variant_builds_the_same_pull_up_and_down_shape() {
        let project = ComposeProject::new("web", ["/srv/web/compose.yml"]);

        for (command, program, prefix) in [
            (ComposeCommand::DockerCompose, "docker", vec!["compose"]),
            (
                ComposeCommand::DockerComposeStandalone,
                "docker-compose",
                vec![],
            ),
            (ComposeCommand::PodmanCompose, "podman", vec!["compose"]),
        ] {
            for action in [vec!["down"], vec!["up", "-d"], vec!["pull", "--quiet"]] {
                let spec = command.command(&project, &action);
                let mut expected = prefix.clone();
                expected.extend(["-p", "web", "-f", "/srv/web/compose.yml"]);
                expected.extend(action);

                assert_eq!(spec.program(), program, "{command}");
                assert_eq!(spec.arguments(), os_strings(&expected), "{command}");
            }
        }
    }

    struct FakeComposeProbe {
        existing: HashSet<&'static str>,
        composing: HashSet<&'static str>,
    }

    impl FakeComposeProbe {
        fn new(existing: &[&'static str], composing: &[&'static str]) -> Self {
            Self {
                existing: existing.iter().copied().collect(),
                composing: composing.iter().copied().collect(),
            }
        }
    }

    impl ComposeCommandProbe for FakeComposeProbe {
        fn command_exists(&self, program: &str) -> bool {
            self.existing.contains(program)
        }

        fn supports_compose(&self, program: &str) -> bool {
            self.composing.contains(program)
        }
    }

    #[test]
    fn detects_the_docker_compose_plugin_first() {
        let probe = FakeComposeProbe::new(
            &["docker", "docker-compose", "podman"],
            &["docker", "podman"],
        );

        assert_eq!(
            detect_compose_command(&probe, None),
            Some(ComposeCommand::DockerCompose)
        );
    }

    #[test]
    fn falls_back_to_the_standalone_binary_without_the_docker_plugin() {
        let probe = FakeComposeProbe::new(&["docker", "docker-compose"], &[]);

        assert_eq!(
            detect_compose_command(&probe, None),
            Some(ComposeCommand::DockerComposeStandalone)
        );
    }

    #[test]
    fn falls_back_to_podman_compose_when_docker_has_no_compose() {
        let probe = FakeComposeProbe::new(&["docker", "podman"], &["podman"]);

        assert_eq!(
            detect_compose_command(&probe, None),
            Some(ComposeCommand::PodmanCompose)
        );
    }

    #[test]
    fn no_compose_command_is_detected_without_a_working_implementation() {
        let probe = FakeComposeProbe::new(&["docker", "podman"], &[]);

        assert_eq!(detect_compose_command(&probe, None), None);
    }

    #[test]
    fn prefers_podman_compose_when_podman_is_the_active_runtime() {
        let probe = FakeComposeProbe::new(
            &["docker", "docker-compose", "podman"],
            &["docker", "podman"],
        );

        assert_eq!(
            detect_compose_command(&probe, Some(ContainerRuntimeKind::Podman)),
            Some(ComposeCommand::PodmanCompose)
        );
    }

    #[test]
    fn does_not_prefer_podman_compose_for_other_active_runtimes() {
        let probe = FakeComposeProbe::new(
            &["docker", "docker-compose", "podman"],
            &["docker", "podman"],
        );

        assert_eq!(
            detect_compose_command(&probe, Some(ContainerRuntimeKind::NativeDocker)),
            Some(ComposeCommand::DockerCompose)
        );
    }

    #[test]
    fn podman_compose_only_when_podman_supports_it() {
        let probe = FakeComposeProbe::new(&["docker", "docker-compose", "podman"], &["docker"]);

        assert_eq!(
            detect_compose_command(&probe, Some(ContainerRuntimeKind::Podman)),
            Some(ComposeCommand::DockerCompose)
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

    #[test]
    fn colima_runtime_declares_the_packages_that_trigger_a_restart() {
        let runtime = ColimaRuntime::new();

        assert_eq!(runtime.name(), "colima/lima");
        assert_eq!(runtime.display_name(), "Colima/lima");
        assert_eq!(runtime.command(), "colima");
        assert_eq!(runtime.restart_formulae(), ["colima", "lima"]);
        assert!(runtime.restart_casks().is_empty());
        assert!(runtime.triggers_restart("colima"));
        assert!(runtime.triggers_restart("lima"));
        assert!(!runtime.triggers_restart("docker"));
    }

    #[test]
    fn docker_desktop_runtime_declares_its_cask_as_the_restart_trigger() {
        let runtime = DockerDesktopRuntime::new();

        assert_eq!(runtime.name(), "docker-desktop");
        assert_eq!(runtime.display_name(), "Docker Desktop");
        assert_eq!(runtime.command(), "docker desktop");
        assert!(runtime.restart_formulae().is_empty());
        assert_eq!(runtime.restart_casks(), ["docker-desktop"]);
        assert!(runtime.triggers_restart("docker-desktop"));
        assert!(!runtime.triggers_restart("docker"));
        assert!(!runtime.triggers_restart("colima"));
    }

    #[test]
    fn docker_desktop_stops_before_casks_and_starts_after_them() {
        let registry = homebrew::register(register_for(
            Registry::new(),
            Some(ContainerRuntimeKind::DockerDesktop),
        ));
        let ordered: Vec<StepId> = registry
            .ordered_for(Platform::Macos)
            .unwrap_or_else(|error| panic!("registry order failed: {error}"))
            .iter()
            .map(|step| step.id())
            .collect();

        assert_eq!(
            ordered,
            [
                "macos-updates",
                "brew",
                "brew-formulae",
                "colima-stop",
                "brew-services",
                "brew-casks",
                "colima-start",
                "brew-autoremove",
                "brew-cleanup",
                "brew-doctor",
                "brew-bundle-cleanup",
                "brew-reopen",
                "docker",
            ]
        );
    }

    #[test]
    fn orbstack_runtime_declares_its_cask_as_the_restart_trigger() {
        let runtime = OrbStackRuntime::new();

        assert_eq!(runtime.name(), "orbstack");
        assert_eq!(runtime.display_name(), "OrbStack");
        assert_eq!(runtime.command(), "orbctl");
        assert!(runtime.restart_formulae().is_empty());
        assert_eq!(runtime.restart_casks(), ["orbstack"]);
        assert!(runtime.triggers_restart("orbstack"));
        assert!(!runtime.triggers_restart("orbctl"));
        assert!(!runtime.triggers_restart("docker"));
        assert!(!runtime.triggers_restart("colima"));
    }

    #[test]
    fn orbstack_stops_before_casks_and_starts_after_them() {
        let registry = homebrew::register(register_for(
            Registry::new(),
            Some(ContainerRuntimeKind::OrbStack),
        ));
        let ordered: Vec<StepId> = registry
            .ordered_for(Platform::Macos)
            .unwrap_or_else(|error| panic!("registry order failed: {error}"))
            .iter()
            .map(|step| step.id())
            .collect();

        assert_eq!(
            ordered,
            [
                "macos-updates",
                "brew",
                "brew-formulae",
                "colima-stop",
                "brew-services",
                "brew-casks",
                "colima-start",
                "brew-autoremove",
                "brew-cleanup",
                "brew-doctor",
                "brew-bundle-cleanup",
                "brew-reopen",
                "docker",
            ]
        );
    }

    #[test]
    fn rancher_desktop_runtime_declares_its_cask_as_the_restart_trigger() {
        let runtime = RancherDesktopRuntime::new();

        assert_eq!(runtime.name(), "rancher-desktop");
        assert_eq!(runtime.display_name(), "Rancher Desktop");
        assert_eq!(runtime.command(), "rdctl");
        assert!(runtime.restart_formulae().is_empty());
        assert_eq!(runtime.restart_casks(), ["rancher"]);
        assert!(runtime.triggers_restart("rancher"));
        assert!(!runtime.triggers_restart("rancher-desktop"));
        assert!(!runtime.triggers_restart("docker"));
        assert!(!runtime.triggers_restart("colima"));
    }

    #[test]
    fn rancher_desktop_stops_before_casks_and_starts_after_them() {
        let registry = homebrew::register(register_for(
            Registry::new(),
            Some(ContainerRuntimeKind::RancherDesktop),
        ));
        let ordered: Vec<StepId> = registry
            .ordered_for(Platform::Macos)
            .unwrap_or_else(|error| panic!("registry order failed: {error}"))
            .iter()
            .map(|step| step.id())
            .collect();

        assert_eq!(
            ordered,
            [
                "macos-updates",
                "brew",
                "brew-formulae",
                "colima-stop",
                "brew-services",
                "brew-casks",
                "colima-start",
                "brew-autoremove",
                "brew-cleanup",
                "brew-doctor",
                "brew-bundle-cleanup",
                "brew-reopen",
                "docker",
            ]
        );
    }

    #[test]
    fn podman_runtime_declares_its_formula_as_the_restart_trigger() {
        let runtime = PodmanRuntime::new();

        assert_eq!(runtime.name(), "podman-machine");
        assert_eq!(runtime.display_name(), "Podman machine");
        assert_eq!(runtime.command(), "podman machine");
        assert_eq!(runtime.restart_formulae(), ["podman"]);
        assert!(runtime.restart_casks().is_empty());
        assert!(runtime.triggers_restart("podman"));
        assert!(!runtime.triggers_restart("podman-desktop"));
        assert!(!runtime.triggers_restart("docker"));
        assert!(!runtime.triggers_restart("colima"));
    }

    #[test]
    fn podman_stops_before_formulae_and_starts_after_them() {
        let registry = homebrew::register(register_for(
            Registry::new(),
            Some(ContainerRuntimeKind::Podman),
        ));
        let ordered: Vec<StepId> = registry
            .ordered_for(Platform::Macos)
            .unwrap_or_else(|error| panic!("registry order failed: {error}"))
            .iter()
            .map(|step| step.id())
            .collect();

        assert_eq!(
            ordered,
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
            ]
        );
    }

    #[test]
    fn podman_machine_running_reads_the_running_flag() {
        assert!(podman_machine_running(
            r#"[{"Name":"podman-machine-default","Running":true}]"#
        ));
        assert!(podman_machine_running(
            r#"[{"Name":"stopped","Running":false},{"Name":"default","Running":true}]"#
        ));
        assert!(!podman_machine_running(
            r#"[{"Name":"podman-machine-default","Running":false}]"#
        ));
        assert!(!podman_machine_running("[]"));
        assert!(!podman_machine_running(""));
        assert!(!podman_machine_running("not json"));
    }

    #[test]
    fn running_standalone_lima_instances_exclude_colima_and_stopped_instances() {
        let output = concat!(
            r#"{"name":"colima","status":"Running"}"#,
            "\n",
            r#"{"name":"colima-dev","status":"Running"}"#,
            "\n",
            r#"{"name":"default","status":"Running"}"#,
            "\n",
            r#"{"name":"stopped","status":"Stopped"}"#,
            "\n",
        );

        assert_eq!(parse_running_standalone_lima_instances(output), ["default"]);
    }

    #[test]
    fn running_standalone_lima_instances_accept_an_array_and_skip_duplicates() {
        let output = r#"[
            {"name":"default","status":"running"},
            {"name":"default","status":"Running"},
            {"name":"colima","status":"Running"},
            {"name":"work","status":"Running"}
        ]"#;

        assert_eq!(
            parse_running_standalone_lima_instances(output),
            ["default", "work"]
        );
    }

    #[test]
    fn malformed_lima_output_yields_no_instances() {
        for output in ["", "not json", "{}", r#"{"name":"default"}"#, "[]"] {
            assert!(
                parse_running_standalone_lima_instances(output).is_empty(),
                "input: {output:?}"
            );
        }
    }

    #[test]
    fn an_unknown_runtime_keeps_the_colima_steps() {
        for kind in [None, Some(ContainerRuntimeKind::Colima)] {
            let registry = register_for(Registry::new(), kind);
            let names: Vec<_> = registry
                .steps()
                .map(|step| step.display_name().to_owned())
                .collect();

            assert_eq!(
                names,
                ["Colima/lima stop", "Colima/lima start", "Docker images"]
            );
        }
    }

    struct FakeDockerEngineProbe {
        commands: HashSet<&'static str>,
        active: bool,
        live_restore: Option<bool>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeDockerEngineProbe {
        fn installed() -> (Self, Arc<Mutex<Vec<String>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let probe = Self {
                commands: ["docker", "systemctl"].into_iter().collect(),
                active: true,
                live_restore: Some(true),
                calls: Arc::clone(&calls),
            };

            (probe, calls)
        }

        fn without(command: &'static str) -> Self {
            let (mut probe, _) = Self::installed();
            probe.commands.remove(command);
            probe
        }
    }

    impl DockerEngineProbe for FakeDockerEngineProbe {
        fn command_exists(&self, program: &str) -> bool {
            self.commands.contains(program)
        }

        fn unit_active(&self, unit: &str) -> bool {
            self.active && unit == "docker"
        }

        fn live_restore_enabled(&self) -> Option<bool> {
            self.live_restore
        }

        fn run_systemctl(&self, action: &str, unit: &str) -> Result<(), ProcessError> {
            let mut calls = self
                .calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            calls.push(format!("{action} {unit}"));
            Ok(())
        }
    }

    #[test]
    fn native_docker_runtime_declares_the_docker_formula_on_linux() {
        let (probe, _) = FakeDockerEngineProbe::installed();
        let runtime = NativeDockerRuntime::with_probe(probe);

        assert_eq!(runtime.name(), "docker-engine");
        assert_eq!(runtime.display_name(), "Docker Engine");
        assert_eq!(runtime.command(), "docker");
        assert_eq!(runtime.restart_formulae(), ["docker"]);
        assert!(runtime.restart_casks().is_empty());
        assert!(runtime.triggers_restart("docker"));
        assert!(!runtime.triggers_restart("docker-desktop"));
        assert_eq!(runtime.platforms(), [Platform::Linux]);
        assert_eq!(runtime.detect(), Detection::Installed);
        assert!(runtime.is_running());
    }

    #[test]
    fn native_docker_requires_the_docker_and_systemctl_commands() {
        let (probe, _) = FakeDockerEngineProbe::installed();
        assert_eq!(
            NativeDockerRuntime::with_probe(probe).detect(),
            Detection::Installed
        );

        for missing in ["docker", "systemctl"] {
            let runtime = NativeDockerRuntime::with_probe(FakeDockerEngineProbe::without(missing));

            assert_eq!(
                runtime.detect(),
                Detection::NotInstalled,
                "missing {missing}"
            );
        }
    }

    #[test]
    fn native_docker_is_running_only_when_the_systemd_unit_is_active() {
        let (mut probe, _) = FakeDockerEngineProbe::installed();
        probe.active = false;

        assert!(!NativeDockerRuntime::with_probe(probe).is_running());
    }

    #[test]
    fn native_docker_stops_then_restarts_the_daemon() {
        let (probe, calls) = FakeDockerEngineProbe::installed();
        let runtime = NativeDockerRuntime::with_probe(probe);

        runtime
            .stop()
            .unwrap_or_else(|error| panic!("stop failed: {error}"));
        runtime
            .start()
            .unwrap_or_else(|error| panic!("start failed: {error}"));

        let calls = calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(*calls, ["stop docker", "restart docker"]);
    }

    #[test]
    fn live_restore_warning_only_when_disabled() {
        assert_eq!(
            live_restore_warning(Some(false)),
            Some(super::LIVE_RESTORE_WARNING)
        );
        assert_eq!(live_restore_warning(Some(true)), None);
        assert_eq!(live_restore_warning(None), None);
    }

    #[test]
    fn live_restore_parsing_accepts_true_and_false() {
        assert_eq!(parse_live_restore_enabled("true\n"), Some(true));
        assert_eq!(parse_live_restore_enabled(" FALSE "), Some(false));
        assert_eq!(parse_live_restore_enabled(""), None);
        assert_eq!(parse_live_restore_enabled("maybe"), None);
    }

    #[test]
    fn native_docker_steps_are_linux_only() {
        let registry = homebrew::register(register_for(
            Registry::new(),
            Some(ContainerRuntimeKind::NativeDocker),
        ));
        let ordered = |platform| -> Vec<StepId> {
            registry
                .ordered_for(platform)
                .unwrap_or_else(|error| panic!("registry order failed: {error}"))
                .iter()
                .map(|step| step.id())
                .collect()
        };

        assert_eq!(
            ordered(Platform::Linux),
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
            ]
        );

        let macos = ordered(Platform::Macos);
        assert!(!macos.contains(&COLIMA_STOP), "steps: {macos:?}");
        assert!(!macos.contains(&COLIMA_START), "steps: {macos:?}");
        assert!(!macos.contains(&DOCKER), "steps: {macos:?}");
    }

    struct FakeRuntime;

    impl ContainerRuntime for FakeRuntime {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn display_name(&self) -> &'static str {
            "Fake"
        }

        fn command(&self) -> &'static str {
            "fake"
        }

        fn detection_commands(&self) -> &'static [&'static str] {
            &["fake"]
        }

        fn detect(&self) -> Detection {
            Detection::Installed
        }

        fn is_running(&self) -> bool {
            true
        }

        fn stop(&self) -> Result<(), ProcessError> {
            Ok(())
        }

        fn start(&self) -> Result<(), ProcessError> {
            Ok(())
        }

        fn restart_formulae(&self) -> &'static [&'static str] {
            &["fake-formula"]
        }

        fn restart_casks(&self) -> &'static [&'static str] {
            &["fake-cask"]
        }
    }

    #[test]
    fn a_runtime_triggers_a_restart_for_formulae_and_casks() {
        let runtime = FakeRuntime;

        assert!(runtime.triggers_restart("fake-formula"));
        assert!(runtime.triggers_restart("fake-cask"));
        assert!(!runtime.triggers_restart("other"));
    }

    #[test]
    fn the_steps_take_their_names_from_the_registered_runtime() {
        let registry = register_with_runtime(
            Registry::new(),
            ContainerRuntimeKind::NativeDocker,
            FakeRuntime,
        );

        let names: Vec<_> = registry
            .steps()
            .map(|step| step.display_name().to_owned())
            .collect();

        assert_eq!(names, ["Fake stop", "Fake start", "Docker images"]);
    }

    #[derive(Default)]
    struct FakeProbe {
        variables: HashMap<String, String>,
        context: Option<String>,
        sockets: HashSet<PathBuf>,
    }

    impl FakeProbe {
        fn with_var(mut self, name: &str, value: &str) -> Self {
            self.variables.insert(name.to_owned(), value.to_owned());
            self
        }

        fn with_context(mut self, context: &str) -> Self {
            self.context = Some(context.to_owned());
            self
        }

        fn with_socket(mut self, path: impl Into<PathBuf>) -> Self {
            self.sockets.insert(path.into());
            self
        }
    }

    impl ContainerRuntimeProbe for FakeProbe {
        fn variable(&self, name: &str) -> Option<String> {
            self.variables.get(name).cloned()
        }

        fn docker_context(&self) -> Option<String> {
            self.context.clone()
        }

        fn socket_exists(&self, path: &Path) -> bool {
            self.sockets.contains(path)
        }
    }

    fn detected(probe: &FakeProbe) -> Option<ActiveContainerRuntime> {
        detect_active_runtime(probe)
    }

    #[test]
    fn docker_host_identifies_each_supported_runtime() {
        for (host, expected) in [
            (
                "unix:///Users/me/.colima/default/docker.sock",
                ContainerRuntimeKind::Colima,
            ),
            (
                "unix:///Users/me/.orbstack/run/docker.sock",
                ContainerRuntimeKind::OrbStack,
            ),
            (
                "unix:///Users/me/.docker/run/docker.sock",
                ContainerRuntimeKind::DockerDesktop,
            ),
            (
                "unix:///Users/me/.rd/docker.sock",
                ContainerRuntimeKind::RancherDesktop,
            ),
            (
                "unix:///run/user/501/podman/podman.sock",
                ContainerRuntimeKind::Podman,
            ),
            (
                "unix:///var/run/docker.sock",
                ContainerRuntimeKind::NativeDocker,
            ),
        ] {
            let probe = FakeProbe::default().with_var("DOCKER_HOST", host);
            let runtime = detected(&probe).unwrap_or_else(|| panic!("{host} was not detected"));

            assert_eq!(runtime.kind(), expected, "{host}");
            assert_eq!(runtime.source(), &ContainerRuntimeSource::Environment);
        }
    }

    #[test]
    fn docker_context_identifies_the_runtime_when_docker_host_is_unset() {
        for (context, expected) in [
            ("colima", ContainerRuntimeKind::Colima),
            ("colima-dev", ContainerRuntimeKind::Colima),
            ("orbstack", ContainerRuntimeKind::OrbStack),
            ("desktop-linux", ContainerRuntimeKind::DockerDesktop),
            ("rancher-desktop", ContainerRuntimeKind::RancherDesktop),
            ("podman-machine-default", ContainerRuntimeKind::Podman),
        ] {
            let probe = FakeProbe::default().with_context(context);
            let runtime = detected(&probe).unwrap_or_else(|| panic!("{context} was not detected"));

            assert_eq!(runtime.kind(), expected, "{context}");
            assert_eq!(runtime.source(), &ContainerRuntimeSource::DockerContext);
        }
    }

    #[test]
    fn the_default_context_falls_through_to_the_socket_paths() {
        let probe = FakeProbe::default()
            .with_context("default")
            .with_socket("/var/run/docker.sock");

        let runtime =
            detected(&probe).unwrap_or_else(|| panic!("default context was not detected"));

        assert_eq!(runtime.kind(), ContainerRuntimeKind::NativeDocker);
        assert_eq!(
            runtime.source(),
            &ContainerRuntimeSource::Socket(PathBuf::from("/var/run/docker.sock"))
        );
    }

    #[test]
    fn well_known_socket_paths_identify_the_runtime() {
        let home = PathBuf::from("/Users/me");
        for (relative, expected) in [
            (".colima/default/docker.sock", ContainerRuntimeKind::Colima),
            (".orbstack/run/docker.sock", ContainerRuntimeKind::OrbStack),
            (
                ".docker/run/docker.sock",
                ContainerRuntimeKind::DockerDesktop,
            ),
            (".rd/docker.sock", ContainerRuntimeKind::RancherDesktop),
            (
                ".local/share/containers/podman/machine/podman.sock",
                ContainerRuntimeKind::Podman,
            ),
        ] {
            let path = home.join(relative);
            let probe = FakeProbe::default()
                .with_var("HOME", "/Users/me")
                .with_socket(&path);
            let runtime = detected(&probe).unwrap_or_else(|| panic!("{relative} was not detected"));

            assert_eq!(runtime.kind(), expected, "{relative}");
            assert_eq!(
                runtime.source(),
                &ContainerRuntimeSource::Socket(path.clone()),
                "{relative}"
            );
        }
    }

    #[test]
    fn the_xdg_runtime_directory_identifies_podman() {
        let path = PathBuf::from("/run/user/1000/podman/podman.sock");
        let probe = FakeProbe::default()
            .with_var("XDG_RUNTIME_DIR", "/run/user/1000")
            .with_socket(&path);

        let runtime = detected(&probe).unwrap_or_else(|| panic!("podman was not detected"));

        assert_eq!(runtime.kind(), ContainerRuntimeKind::Podman);
        assert_eq!(runtime.source(), &ContainerRuntimeSource::Socket(path));
    }

    #[test]
    fn docker_host_wins_over_the_context_and_the_sockets() {
        let probe = FakeProbe::default()
            .with_var("DOCKER_HOST", "unix:///Users/me/.orbstack/run/docker.sock")
            .with_context("desktop-linux")
            .with_socket("/var/run/docker.sock");

        let runtime = detected(&probe).unwrap_or_else(|| panic!("runtime was not detected"));

        assert_eq!(runtime.kind(), ContainerRuntimeKind::OrbStack);
        assert_eq!(runtime.source(), &ContainerRuntimeSource::Environment);
    }

    #[test]
    fn the_context_wins_over_the_socket_paths() {
        let probe = FakeProbe::default()
            .with_context("rancher-desktop")
            .with_socket("/var/run/docker.sock");

        let runtime = detected(&probe).unwrap_or_else(|| panic!("runtime was not detected"));

        assert_eq!(runtime.kind(), ContainerRuntimeKind::RancherDesktop);
        assert_eq!(runtime.source(), &ContainerRuntimeSource::DockerContext);
    }

    #[test]
    fn no_facts_means_no_detected_runtime() {
        assert_eq!(detected(&FakeProbe::default()), None);
    }

    #[test]
    fn an_override_replaces_the_detected_runtime() {
        let runtime = ActiveContainerRuntime::overridden(ContainerRuntimeKind::Podman);

        assert_eq!(runtime.kind(), ContainerRuntimeKind::Podman);
        assert_eq!(runtime.source(), &ContainerRuntimeSource::Override);
        assert_eq!(runtime.to_string(), "Podman (--container-runtime)");
    }

    #[test]
    fn runtime_names_parse_with_aliases() {
        for (name, expected) in [
            ("colima", ContainerRuntimeKind::Colima),
            ("lima", ContainerRuntimeKind::Colima),
            ("orbstack", ContainerRuntimeKind::OrbStack),
            ("docker_desktop", ContainerRuntimeKind::DockerDesktop),
            ("rancher", ContainerRuntimeKind::RancherDesktop),
            ("podman-machine", ContainerRuntimeKind::Podman),
            ("docker-engine", ContainerRuntimeKind::NativeDocker),
        ] {
            assert_eq!(name.parse::<ContainerRuntimeKind>(), Ok(expected), "{name}");
        }

        assert!("nope".parse::<ContainerRuntimeKind>().is_err());
    }

    #[test]
    fn the_runtime_display_names_the_kind_and_the_source() {
        let runtime = ActiveContainerRuntime::new(
            ContainerRuntimeKind::DockerDesktop,
            ContainerRuntimeSource::DockerContext,
        );

        assert_eq!(runtime.to_string(), "Docker Desktop (docker context)");
    }

    #[cfg(unix)]
    mod brew_install_injection {
        use std::env;
        use std::fs;
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::sync::Arc;
        use std::time::SystemTime;

        use super::super::{ColimaRuntime, ContainerSnapshot, ContainerStopStep};
        use crate::homebrew::BrewInstall;
        use crate::system::Arch;

        /// Writes a stub `brew` that prints the given outdated packages and
        /// exits successfully, mirroring `brew outdated --quiet`.
        fn stub_brew(prefix: &Path, packages: &[&str]) -> PathBuf {
            let program = prefix.join("bin").join("brew");
            fs::create_dir_all(program.parent().unwrap_or(prefix))
                .unwrap_or_else(|error| panic!("failed to create {}: {error}", program.display()));
            let mut script = String::from("#!/bin/sh\n");
            for package in packages {
                script.push_str(&format!("echo {package}\n"));
            }
            script.push_str("exit 0\n");
            fs::write(&program, script)
                .unwrap_or_else(|error| panic!("failed to write {}: {error}", program.display()));
            fs::set_permissions(&program, Permissions::from_mode(0o755)).unwrap_or_else(|error| {
                panic!("failed to make {} executable: {error}", program.display())
            });
            program
        }

        fn temp_root(name: &str) -> PathBuf {
            let root = env::temp_dir().join(format!(
                "freshen-container-stop-{}-{}-{name}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|since| since.as_nanos())
                    .unwrap_or_default()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root)
                .unwrap_or_else(|error| panic!("failed to create {}: {error}", root.display()));
            root
        }

        fn stop_step() -> ContainerStopStep {
            ContainerStopStep::new(
                Arc::new(ColimaRuntime::new()),
                ContainerSnapshot::default(),
                None,
                "docker",
            )
        }

        fn install(prefix: PathBuf, arch: Arch, from_path: bool) -> BrewInstall {
            BrewInstall {
                program: prefix.join("bin").join("brew"),
                prefix,
                from_path,
                arch,
            }
        }

        #[test]
        fn stops_containers_when_colima_is_outdated_in_either_architecture() {
            let root = temp_root("either-architecture");
            let arm = root.join("opt/homebrew");
            let intel = root.join("usr/local");
            stub_brew(&arm, &["colima"]);
            stub_brew(&intel, &[]);

            let installs = [
                install(arm, Arch::Aarch64, false),
                install(intel, Arch::X86_64, false),
            ];

            assert!(
                stop_step()
                    .upgrade_triggers_restart(&installs)
                    .unwrap_or_else(|error| panic!("upgrade_triggers_restart failed: {error}")),
                "an outdated colima in any architecture should stop the containers"
            );
        }

        #[test]
        fn stops_containers_when_lima_is_outdated_in_either_architecture() {
            let root = temp_root("lima-either-architecture");
            let arm = root.join("opt/homebrew");
            let intel = root.join("usr/local");
            stub_brew(&arm, &[]);
            stub_brew(&intel, &["lima"]);

            let installs = [
                install(arm, Arch::Aarch64, false),
                install(intel, Arch::X86_64, false),
            ];

            assert!(
                stop_step()
                    .upgrade_triggers_restart(&installs)
                    .unwrap_or_else(|error| panic!("upgrade_triggers_restart failed: {error}")),
                "an outdated lima in any architecture should stop the containers"
            );
        }

        #[test]
        fn does_not_stop_containers_when_nothing_is_outdated() {
            let root = temp_root("nothing-outdated");
            let arm = root.join("opt/homebrew");
            let intel = root.join("usr/local");
            stub_brew(&arm, &[]);
            stub_brew(&intel, &[]);

            let installs = [
                install(arm, Arch::Aarch64, false),
                install(intel, Arch::X86_64, false),
            ];

            assert!(
                !stop_step()
                    .upgrade_triggers_restart(&installs)
                    .unwrap_or_else(|error| panic!("upgrade_triggers_restart failed: {error}")),
                "no outdated colima or lima means no restart"
            );
        }

        #[test]
        fn checks_every_installation_rather_than_only_the_first() {
            let root = temp_root("check-every-installation");
            let arm = root.join("opt/homebrew");
            let intel = root.join("usr/local");
            stub_brew(&arm, &[]); // first installation reports nothing
            stub_brew(&intel, &["lima"]); // second installation reports lima

            let installs = [
                install(arm, Arch::Aarch64, false),
                install(intel, Arch::X86_64, false),
            ];

            assert!(
                stop_step()
                    .upgrade_triggers_restart(&installs)
                    .unwrap_or_else(|error| panic!("upgrade_triggers_restart failed: {error}")),
                "the second installation must be consulted even when the first is current"
            );
        }
    }
}
