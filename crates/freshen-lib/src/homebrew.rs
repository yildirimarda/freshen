//! Homebrew steps: update, formulae, casks, autoremove and cleanup.
//!
//! The zsh implementation runs one long Homebrew block. Splitting it into
//! steps keeps the behaviour (formulae before casks, so apps stay open during
//! the slow formula upgrade) while letting the runner record a failure per
//! command instead of aborting the rest of the block.
//!
//! Casks are inspected with `brew info --cask --json=v2` before the upgrade so
//! the app artifacts a cask will replace are known. Running apps are quit
//! before the upgrade and reopened after cleanup through [`crate::apps`].
//!
//! The doctor and the bundle cleanup are report-only: `brew doctor` and
//! `brew bundle cleanup` only list what they found, and the steps show that
//! list without changing anything.
//!
//! A Mac can have two Homebrew installations: the native one at
//! `/opt/homebrew` and the Intel one at `/usr/local` that runs under Rosetta.
//! The steps discover both (and the `brew` on `PATH`) and run each command for
//! every installation, so packages in the other architecture are not silently
//! left behind. A single installation keeps using the plain `brew` name, so
//! the command transcripts stay the same as before.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::apps::{
    find_running_cask_apps, quit_apps, quit_choice, reopen_apps, AppScan, AppSystem, MacAppSystem,
    QuitChoice, QuitPolicy, Reopen, RunningApp,
};
use crate::cleanup;
use crate::options::RunOptions;
use crate::output::{flush, human, human_print};
use crate::platform::Platform;
use crate::policy::AppLists;
use crate::process::{CommandSpec, ProcessError, ProcessOutput, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::snapshot::{self, SnapshotSection};
use crate::step::{
    CheckReport, Detection, Operation, OutdatedItem, SkipReason, Step, StepError, StepId,
};
use crate::system::{detect_terminal_app, Arch};
use crate::warnings::warn_about_breakable_sessions;
use crate::xcode;

pub(crate) const BREW: StepId = "brew";
pub(crate) const FORMULAE: StepId = "brew-formulae";
pub(crate) const CASKS: StepId = "brew-casks";
const AUTOREMOVE: StepId = "brew-autoremove";
const CLEANUP: StepId = "brew-cleanup";
const DOCTOR: StepId = "brew-doctor";
const BUNDLE_CLEANUP: StepId = "brew-bundle-cleanup";
pub(crate) const REOPEN: StepId = "brew-reopen";
const SERVICES: StepId = "brew-services";

const HOMEBREW_PLATFORMS: [Platform; 2] = [Platform::Macos, Platform::Linux];

/// Prefixes checked on macOS in addition to the `brew` on `PATH`.
const MACOS_BREW_PREFIXES: [&str; 2] = ["/opt/homebrew", "/usr/local"];

/// Environment variable that replaces the prefixes searched for Homebrew.
///
/// A PATH-style list of prefixes, used by tests to point at temporary
/// installations. The `brew` on `PATH` is still discovered.
const BREW_PREFIXES_ENV: &str = "FRESHEN_BREW_PREFIXES";

/// One Homebrew installation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrewInstall {
    /// The `bin/brew` executable.
    program: PathBuf,
    /// The installation prefix, for example `/opt/homebrew`.
    prefix: PathBuf,
    /// Whether the executable was found through `PATH`.
    from_path: bool,
    /// The architecture this installation serves.
    arch: Arch,
}

impl BrewInstall {
    /// The command that runs this installation.
    ///
    /// The installation on `PATH` keeps the plain `brew` name so the command
    /// transcript does not change for a normal single-install machine. The
    /// other installation is named by its absolute path, which is also what
    /// makes the two architectures unambiguous in the log.
    fn command(&self) -> CommandSpec {
        if self.from_path {
            CommandSpec::new("brew")
        } else {
            CommandSpec::new(self.program.as_os_str())
        }
    }

    /// The program name recorded in a pre-upgrade snapshot.
    fn snapshot_program(&self) -> String {
        if self.from_path {
            "brew".to_owned()
        } else {
            self.program.display().to_string()
        }
    }

    /// Whether this installation serves `arch`.
    fn serves(&self, arch: &Arch) -> bool {
        &self.arch == arch
    }
}

/// The Homebrew installations found on this machine.
#[derive(Debug, Clone, Default)]
struct BrewSet {
    installs: Vec<BrewInstall>,
}

impl BrewSet {
    /// Discovers every Homebrew installation.
    fn detect() -> Self {
        Self {
            installs: brew_installs(),
        }
    }

    fn installs(&self) -> &[BrewInstall] {
        &self.installs
    }

    fn is_empty(&self) -> bool {
        self.installs.is_empty()
    }

    fn is_multiple(&self) -> bool {
        self.installs.len() > 1
    }

    /// A label for a message, naming the prefix only when there is more than
    /// one installation.
    fn label(&self, install: &BrewInstall) -> String {
        if self.is_multiple() {
            format!("Homebrew at {}", install.prefix.display())
        } else {
            "Homebrew".to_owned()
        }
    }
}

/// Prefixes a message with the installation only when there is more than one.
fn install_message(brews: &BrewSet, install: &BrewInstall, message: impl Into<String>) -> String {
    let message = message.into();
    if brews.is_multiple() {
        format!("{}: {message}", brews.label(install))
    } else {
        message
    }
}

/// Discovers the `brew` on `PATH` and the standard prefixes.
fn brew_installs() -> Vec<BrewInstall> {
    let path = crate::executable::resolve("brew");
    brew_installs_from(path, &brew_prefixes(), &Arch::current())
}

/// The prefixes searched for a Homebrew installation.
fn brew_prefixes() -> Vec<PathBuf> {
    if let Some(value) = env::var_os(BREW_PREFIXES_ENV) {
        return env::split_paths(&value).collect();
    }
    if Platform::current() == Some(Platform::Macos) {
        return MACOS_BREW_PREFIXES.iter().map(PathBuf::from).collect();
    }
    Vec::new()
}

/// Discovers installations from an already resolved `PATH` executable and a
/// list of prefixes.
///
/// The result is deduplicated by executable and ordered with the installation
/// that matches the running architecture first, so a Rosetta run updates its
/// own architecture before the other one.
fn brew_installs_from(
    path: Option<PathBuf>,
    prefixes: &[PathBuf],
    current_arch: &Arch,
) -> Vec<BrewInstall> {
    let mut seen = HashSet::new();
    let mut installs = Vec::new();

    if let Some(program) = path {
        let prefix = prefix_of(&program);
        push_install(
            &mut installs,
            &mut seen,
            program,
            prefix,
            true,
            current_arch,
        );
    }
    for prefix in prefixes {
        push_install(
            &mut installs,
            &mut seen,
            prefix.join("bin").join("brew"),
            prefix.clone(),
            false,
            current_arch,
        );
    }

    installs.sort_by_key(|install| std::cmp::Reverse(install.serves(current_arch)));
    installs
}

fn push_install(
    installs: &mut Vec<BrewInstall>,
    seen: &mut HashSet<PathBuf>,
    program: PathBuf,
    prefix: PathBuf,
    from_path: bool,
    current_arch: &Arch,
) {
    if crate::executable::resolve(&program).is_none() {
        return;
    }

    let key = fs::canonicalize(&program).unwrap_or_else(|_| program.clone());
    if !seen.insert(key) {
        return;
    }

    installs.push(BrewInstall {
        arch: install_arch(&prefix, current_arch),
        program,
        prefix,
        from_path,
    });
}

/// The prefix of an executable named `<prefix>/bin/brew`.
fn prefix_of(program: &Path) -> PathBuf {
    program
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| PathBuf::from("/usr/local"), Path::to_path_buf)
}

/// The architecture a prefix serves.
fn install_arch(prefix: &Path, current_arch: &Arch) -> Arch {
    if prefix.ends_with("opt/homebrew") {
        Arch::Aarch64
    } else if prefix.ends_with("usr/local") {
        Arch::X86_64
    } else {
        current_arch.clone()
    }
}

/// The apps that were quit and still need to be reopened.
///
/// The session state of every app is kept, so an app comes back the way it
/// was before the upgrade.
#[derive(Clone, Default)]
struct ReopenQueue(Arc<Mutex<Vec<Reopen>>>);

impl ReopenQueue {
    fn extend(&self, apps: impl IntoIterator<Item = Reopen>) {
        lock(&self.0).extend(apps);
    }

    fn apps(&self) -> Vec<Reopen> {
        lock(&self.0).clone()
    }

    fn clear(&self) {
        lock(&self.0).clear();
    }
}

/// A Homebrew service that was running before its formula was upgraded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QueuedService {
    install: BrewInstall,
    name: String,
}

/// Homebrew services that were running before their formula was upgraded and
/// still need to be restarted.
#[derive(Clone, Default)]
struct ServiceQueue(Arc<Mutex<Vec<QueuedService>>>);

impl ServiceQueue {
    fn extend(&self, install: &BrewInstall, services: impl IntoIterator<Item = String>) {
        let mut queued = lock(&self.0);
        for name in services {
            let service = QueuedService {
                install: install.clone(),
                name,
            };
            if !queued.contains(&service) {
                queued.push(service);
            }
        }
    }

    fn services(&self) -> Vec<QueuedService> {
        lock(&self.0).clone()
    }

    fn remove(&self, service: &QueuedService) {
        lock(&self.0).retain(|queued| queued != service);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn reopen_queued(queue: &ReopenQueue, system: &dyn AppSystem, message: &str, interrupted: bool) {
    let apps = queue.apps();
    if apps.is_empty() {
        return;
    }

    human!("{message}");
    reopen_apps(system, &apps);
    if !interrupted {
        queue.clear();
    }
}

/// Registers every Homebrew step, in the order the zsh script runs them.
///
/// Prerequisites make the ordering explicit: formulae after the update, casks
/// after formulae, autoremove and cleanup last, and the quit apps are reopened
/// only after cleanup. The doctor and the bundle cleanup report on the state
/// the run leaves behind, so they run after the cleanup and before the apps
/// come back.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    let reopen = ReopenQueue::default();
    let services = ServiceQueue::default();
    registry
        .with(UpdateStep)
        .with(FormulaeStep::new(services.clone()))
        .with(ServicesStep::new(services))
        .with(CasksStep::new(reopen.clone()))
        .with(AutoremoveStep)
        .with(CleanupStep)
        .with(DoctorStep::new())
        .with(BundleCleanupStep::new())
        .with(ReopenStep::new(reopen))
}

/// A failure while reading Homebrew cask metadata.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaskError {
    /// `brew info --cask --json=v2` did not return valid JSON.
    #[error("invalid Homebrew cask JSON: {0}")]
    Json(String),
}

/// One cask from the metadata `brew info --cask --json=v2` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaskInfo {
    token: Option<String>,
    apps: Vec<String>,
    quit_bundle_ids: Vec<String>,
    administrator: bool,
    version: Option<String>,
    auto_updates: bool,
}

impl CaskInfo {
    /// The token of the cask, when the metadata names it.
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// The app bundles the cask installs.
    #[must_use]
    pub fn apps(&self) -> &[String] {
        &self.apps
    }

    /// The bundle ids the cask's `uninstall` stanza asks to quit.
    ///
    /// A cask whose artifacts are not an `.app` bundle, such as a pkg
    /// installer or a menu-bar helper, names the running app only here.
    #[must_use]
    pub fn quit_bundle_ids(&self) -> &[String] {
        &self.quit_bundle_ids
    }

    /// Returns `true` when the cask asks for an administrator password.
    #[must_use]
    pub fn needs_administrator(&self) -> bool {
        self.administrator
    }

    /// The version the cask declares, when the metadata names it.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Returns `true` when the cask updates itself.
    #[must_use]
    pub fn auto_updates(&self) -> bool {
        self.auto_updates
    }

    /// Returns `true` when the cask is only listed by `brew outdated` with
    /// `--greedy`.
    ///
    /// A cask with `version :latest` has no fixed version to compare, and an
    /// `auto_updates true` cask manages its own updates. Homebrew hides both
    /// unless `--greedy` is given.
    #[must_use]
    pub fn needs_greedy(&self) -> bool {
        self.version.as_deref() == Some("latest") || self.auto_updates
    }

    /// Why the cask needs `--greedy`, when it does.
    #[must_use]
    pub fn greedy_reason(&self) -> Option<String> {
        let latest = self.version.as_deref() == Some("latest");
        match (latest, self.auto_updates) {
            (true, true) => {
                Some("version :latest and auto_updates true; needs --greedy".to_owned())
            }
            (true, false) => Some("version :latest; needs --greedy".to_owned()),
            (false, true) => Some("auto_updates true; needs --greedy".to_owned()),
            (false, false) => None,
        }
    }
}

/// The casks from the metadata `brew info --cask --json=v2` returns.
///
/// The metadata says which apps a cask installs and whether its installer asks
/// for an administrator password, so a run without a terminal can skip the
/// casks it cannot install.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaskMetadata {
    casks: Vec<CaskInfo>,
}

impl CaskMetadata {
    /// The casks the metadata describes.
    #[must_use]
    pub fn casks(&self) -> &[CaskInfo] {
        &self.casks
    }

    /// The app bundles every cask installs.
    #[must_use]
    pub fn apps(&self) -> Vec<String> {
        self.casks
            .iter()
            .flat_map(|cask| cask.apps().iter().cloned())
            .collect()
    }

    /// Returns `true` when the cask `token` asks for an administrator
    /// password.
    ///
    /// A cask the metadata does not describe never asks for one.
    #[must_use]
    pub fn needs_administrator(&self, token: &str) -> bool {
        self.cask(token).is_some_and(CaskInfo::needs_administrator)
    }

    /// Returns `true` when upgrading the cask `token` needs `--greedy`.
    ///
    /// A cask the metadata does not describe never needs it.
    #[must_use]
    pub fn needs_greedy(&self, token: &str) -> bool {
        self.cask(token).is_some_and(CaskInfo::needs_greedy)
    }

    /// Why upgrading the cask `token` needs `--greedy`, when it does.
    #[must_use]
    pub fn greedy_reason(&self, token: &str) -> Option<String> {
        self.cask(token).and_then(CaskInfo::greedy_reason)
    }

    fn cask(&self, token: &str) -> Option<&CaskInfo> {
        self.casks.iter().find(|cask| cask.token() == Some(token))
    }
}

/// A read-only probe that verifies the recorded `brew info --cask --json=v2`
/// fixtures against the parser. Fails when the fixtures no longer parse,
/// which indicates parser drift.
pub fn cask_metadata_probe() -> Result<(), CaskError> {
    let fixtures = [
        include_str!("../tests/fixtures/brew-info-casks.json"),
        include_str!("../tests/fixtures/brew-info-casks-greedy.json"),
        include_str!("../tests/fixtures/brew-info-casks-administrator.json"),
    ];
    for json in fixtures {
        cask_metadata(json)?;
    }
    Ok(())
}

/// Parses the cask metadata from a recorded `brew info --cask --json=v2`
/// transcript.
///
/// # Errors
///
/// Returns [`CaskError::Json`] when the transcript is not valid JSON.
pub fn cask_metadata(json: &str) -> Result<CaskMetadata, CaskError> {
    let value: Value =
        serde_json::from_str(json).map_err(|error| CaskError::Json(error.to_string()))?;
    let Some(casks) = value.get("casks").and_then(Value::as_array) else {
        return Ok(CaskMetadata::default());
    };

    Ok(CaskMetadata {
        casks: casks.iter().map(cask_info).collect(),
    })
}

/// Reads one cask from the metadata.
fn cask_info(cask: &Value) -> CaskInfo {
    let artifacts = cask.get("artifacts").and_then(Value::as_array);

    CaskInfo {
        token: cask.get("token").and_then(Value::as_str).map(str::to_owned),
        apps: artifacts.map_or_else(Vec::new, |artifacts| app_artifacts(artifacts)),
        quit_bundle_ids: uninstall_quit_bundle_ids(
            cask,
            artifacts.map_or(&[], |artifacts| artifacts.as_slice()),
        ),
        administrator: artifacts
            .is_some_and(|artifacts| artifacts.iter().any(artifact_needs_administrator)),
        version: cask
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_owned),
        auto_updates: cask
            .get("auto_updates")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// App artifact names a set of casks will install, from a recorded
/// `brew info --cask --json=v2` transcript.
///
/// Both artifact forms Homebrew emits are supported: a plain string
/// (`"Firefox.app"`) and a mapping with a `target`
/// (`{"target": "/Applications/Google Chrome.app"}`). Non-app artifacts and
/// malformed entries are ignored, matching the zsh parser.
///
/// # Errors
///
/// Returns [`CaskError::Json`] when the transcript is not valid JSON.
pub fn cask_app_artifacts(json: &str) -> Result<Vec<String>, CaskError> {
    Ok(cask_metadata(json)?.apps())
}

/// The app bundles the artifacts install.
fn app_artifacts(artifacts: &[Value]) -> Vec<String> {
    let mut apps = Vec::new();
    for artifact in artifacts {
        let Some(entries) = artifact.get("app").and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            if let Some(name) = app_name(entry) {
                apps.push(name);
            }
        }
    }

    apps
}

fn app_name(entry: &Value) -> Option<String> {
    let path = match entry {
        Value::String(path) => path.as_str(),
        Value::Object(fields) => fields.get("target")?.as_str()?,
        _ => return None,
    };

    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

/// The bundle ids the cask's `uninstall` stanza asks to quit.
///
/// Homebrew emits the stanza as an artifact (`{"uninstall": [{"quit": ...}]}`),
/// and some metadata also carries it at the top level. Both forms are read so
/// a cask whose artifacts are not an `.app` bundle still has its running app
/// quit before the upgrade.
fn uninstall_quit_bundle_ids(cask: &Value, artifacts: &[Value]) -> Vec<String> {
    let mut bundle_ids = Vec::new();

    for artifact in artifacts {
        if let Some(uninstall) = artifact.get("uninstall") {
            collect_quit_bundle_ids(uninstall, &mut bundle_ids);
        }
    }
    if let Some(uninstall) = cask.get("uninstall") {
        collect_quit_bundle_ids(uninstall, &mut bundle_ids);
    }

    bundle_ids
}

fn collect_quit_bundle_ids(uninstall: &Value, bundle_ids: &mut Vec<String>) {
    match uninstall {
        Value::Array(items) => {
            for item in items {
                if let Some(quit) = item.get("quit") {
                    push_quit_bundle_ids(quit, bundle_ids);
                }
            }
        }
        Value::Object(_) => {
            if let Some(quit) = uninstall.get("quit") {
                push_quit_bundle_ids(quit, bundle_ids);
            }
        }
        _ => {}
    }
}

fn push_quit_bundle_ids(value: &Value, bundle_ids: &mut Vec<String>) {
    match value {
        Value::String(bundle_id) => push_unique_bundle_id(bundle_id, bundle_ids),
        Value::Array(items) => {
            for item in items {
                push_quit_bundle_ids(item, bundle_ids);
            }
        }
        Value::Object(fields) => {
            if let Some(bundle_id) = fields.get("bundle").and_then(Value::as_str) {
                push_unique_bundle_id(bundle_id, bundle_ids);
            }
        }
        _ => {}
    }
}

fn push_unique_bundle_id(bundle_id: &str, bundle_ids: &mut Vec<String>) {
    if !bundle_id.is_empty() && !bundle_ids.iter().any(|known| known == bundle_id) {
        bundle_ids.push(bundle_id.to_owned());
    }
}

/// Whether an artifact needs an administrator password.
///
/// Homebrew installs a macOS installer package and a keyboard layout with
/// `sudo`, and every other artifact that asks for `sudo` in its arguments, or
/// runs a `sudo` command itself, needs the password as well.
fn artifact_needs_administrator(artifact: &Value) -> bool {
    let Some(fields) = artifact.as_object() else {
        return false;
    };

    if fields.contains_key("pkg") || fields.contains_key("keyboard_layout") {
        return true;
    }

    fields.values().any(mentions_sudo)
}

/// Whether the value asks for `sudo` anywhere.
///
/// An install or uninstall step asks for `sudo` when it says so in its
/// arguments, and a script asks for it when the command it runs is `sudo`.
fn mentions_sudo(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            (key == "sudo" && !matches!(value, Value::Bool(false))) || mentions_sudo(value)
        }),
        Value::Array(items) => items.iter().any(mentions_sudo),
        Value::String(text) => mentions_sudo_command(text),
        _ => false,
    }
}

/// Whether the text runs `sudo`.
///
/// The command is recognized on its own (`sudo`, `/usr/bin/sudo`) and after a
/// separator (`;`, `&&`), so a word that merely contains it is not a match.
fn mentions_sudo_command(text: &str) -> bool {
    text.split(|character: char| character.is_whitespace() || ";&|()\"'`$<>".contains(character))
        .any(|token| token.rsplit('/').next() == Some("sudo"))
}

/// The message shown when a cask is skipped because its installer asks for an
/// administrator password.
#[must_use]
pub fn administrator_skip_message(cask: &str) -> String {
    format!(
        "skipping cask `{cask}`: its installer needs an administrator password, \
         but this run has no terminal to type it into. \
         Upgrade it yourself with `brew upgrade --cask {cask}`."
    )
}

/// Existing `.app` bundles for `apps`, searched in `roots` in order.
///
/// The first existing match per app wins, so callers can pass
/// `/Applications` before `~/Applications` and get the same app the zsh
/// script would have found.
#[must_use]
pub fn installed_app_paths(apps: &[String], roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for app in apps {
        for root in roots {
            let candidate = root.join(app);
            if candidate.is_dir() {
                paths.push(candidate);
                break;
            }
        }
    }
    paths
}

/// The commands that record every Homebrew installation for a snapshot.
///
/// A dual-architecture machine records both prefixes, so the rollback hints
/// cover the packages of each installation.
pub(crate) fn homebrew_snapshot_commands() -> Vec<snapshot::SnapshotCommand> {
    let brews = BrewSet::detect();
    let mut commands = Vec::new();
    for install in brews.installs() {
        let program = install.snapshot_program();
        commands.push(snapshot::SnapshotCommand::new(
            program.clone(),
            ["list", "--versions"],
        ));
        commands.push(snapshot::SnapshotCommand::new(
            program.clone(),
            ["list", "--cask", "--versions"],
        ));
        commands.push(
            snapshot::SnapshotCommand::new(program, ["bundle", "dump", "--force"])
                .writing_to_file("--file="),
        );
    }
    commands
}

fn detect_brew() -> Detection {
    if BrewSet::detect().is_empty() {
        Detection::NotInstalled
    } else {
        Detection::Installed
    }
}

fn run_brew(step: StepId, spec: CommandSpec) -> Result<ProcessOutput, StepError> {
    spec.retry_transient_network(NETWORK_RETRIES)
        .run()
        .map_err(|error| brew_failure(step, Operation::Run, &error))
}

/// The failure of a Homebrew command.
///
/// Homebrew refuses to work when the Xcode Command Line Tools are too old,
/// which is what happens right after a macOS update. That failure gets a
/// message that names the fix instead of the raw output of the command.
fn brew_failure(step: StepId, operation: Operation, error: &ProcessError) -> StepError {
    if let Some(output) = error.output() {
        let printed = format!("{}{}", output.stdout(), output.stderr());
        if let Some(failure) = command_line_tools_failure(step, operation, &printed) {
            return failure;
        }
    }
    StepError::from_process(step, operation, error)
}

/// The failure of a Homebrew command whose output says that the Xcode Command
/// Line Tools are too old.
fn command_line_tools_failure(
    step: StepId,
    operation: Operation,
    printed: &str,
) -> Option<StepError> {
    xcode::is_outdated(printed).then(|| StepError::new(step, operation, xcode::outdated_message()))
}

fn lines(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn outdated(step: StepId, install: &BrewInstall, args: &[&str]) -> Result<CheckReport, StepError> {
    let output = install
        .command()
        .args(["outdated"])
        .args(args)
        .neutral_locale()
        .retry_transient_network(NETWORK_RETRIES)
        .run()
        .map_err(|error| brew_failure(step, Operation::Check, &error))?;
    Ok(CheckReport::from_items(
        lines(output.stdout())
            .iter()
            .map(|line| parse_outdated(line)),
    ))
}

/// The reason shown for an outdated formula the user pinned with `brew pin`.
const PINNED_REASON: &str = "held back (pinned)";

/// Lists the formulae pinned with `brew pin`.
///
/// A failure is not fatal: Homebrew itself skips pinned formulae during an
/// upgrade, so the run can continue without the list. The warning keeps the
/// missing information visible.
fn pinned_formulae(install: &BrewInstall) -> Option<Vec<String>> {
    match install
        .command()
        .args(["list", "--pinned", "--formula"])
        .neutral_locale()
        .run()
    {
        Ok(output) => Some(lines(output.stdout())),
        Err(error) => {
            eprintln!("freshen: could not list pinned formulae: {error}");
            None
        }
    }
}

/// The last path segment of a formula name, without its tap.
fn short_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Returns `true` when `name` is in the pinned list.
///
/// `brew outdated` can print a tapped formula as `user/tap/tool`, while
/// `brew list --pinned` prints the rack name `tool`. Both forms name the same
/// pin, so the comparison uses the last path segment on both sides.
fn is_pinned(name: &str, pinned: &[String]) -> bool {
    let name = short_name(name);
    pinned.iter().any(|pinned| short_name(pinned) == name)
}

/// Adds the held-back reason to the outdated items the user pinned.
fn with_pinned_reasons(report: CheckReport, pinned: &[String]) -> CheckReport {
    if pinned.is_empty() {
        return report;
    }

    CheckReport::from_items(report.items().iter().map(|item| {
        if is_pinned(item.name(), pinned) {
            item.clone().with_reason(PINNED_REASON)
        } else {
            item.clone()
        }
    }))
}

/// Names the installation in a check item when more than one is present.
fn qualify_items(report: CheckReport, install: &BrewInstall, multiple: bool) -> CheckReport {
    if !multiple {
        return report;
    }

    CheckReport::from_items(report.items().iter().map(|item| {
        let from = format!("from {}", install.prefix.display());
        let reason = match item.reason() {
            Some(existing) => format!("{existing}; {from}"),
            None => from,
        };
        item.clone().with_reason(reason)
    }))
}

/// Parses the services `brew services list` reports as running.
///
/// The output is a table with a `Name Status ...` header. Only services that
/// are currently running are returned, so a stopped service is not started
/// behind the user's back.
fn running_services(output: &str) -> Vec<String> {
    lines(output)
        .into_iter()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let status = fields.next()?;
            (name != "Name" && matches!(status, "started" | "running")).then(|| name.to_owned())
        })
        .collect()
}

/// The running services whose formula is about to be upgraded.
fn services_for_formulae(output: &str, formulae: &[String]) -> Vec<String> {
    running_services(output)
        .into_iter()
        .filter(|service| {
            formulae
                .iter()
                .any(|formula| short_name(formula) == short_name(service))
        })
        .collect()
}

/// Records the services that were running before `formulae` are upgraded.
///
/// A failure is not fatal: the formula upgrade can still proceed, and the
/// warning keeps the missing service restart visible.
fn capture_running_services(formulae: &[String], install: &BrewInstall, queue: &ServiceQueue) {
    if formulae.is_empty() {
        return;
    }

    match install
        .command()
        .args(["services", "list"])
        .neutral_locale()
        .run()
    {
        Ok(output) => queue.extend(install, services_for_formulae(output.stdout(), formulae)),
        Err(error) => eprintln!("freshen: could not list Homebrew services: {error}"),
    }
}

/// Parses one line of `brew outdated` output.
///
/// Homebrew prints `name (current) < latest` for formulae and
/// `name (current) != latest` for casks, but older versions and some taps
/// print only the name. Both forms are accepted so `--check` still reports the
/// item when the versions are missing.
fn parse_outdated(line: &str) -> OutdatedItem {
    let line = line.trim();
    let Some((name, rest)) = line.split_once(" (") else {
        return OutdatedItem::new(line);
    };
    let Some((current, rest)) = rest.split_once(')') else {
        return OutdatedItem::new(line);
    };

    let item = OutdatedItem::new(name).with_current(current);
    match latest_version(rest.trim()) {
        Some(latest) => item.with_latest(latest),
        None => item,
    }
}

fn latest_version(rest: &str) -> Option<&str> {
    let rest = rest
        .strip_prefix("< ")
        .or_else(|| rest.strip_prefix("!= "))
        .or_else(|| rest.strip_prefix("== "))?;
    let latest = rest.split_whitespace().next()?;
    (!latest.is_empty()).then_some(latest)
}

/// Directories searched for installed cask apps. `FRESHEN_APP_ROOTS` (a
/// PATH-style list) replaces the defaults so tests never see the real machine.
fn app_roots() -> Vec<PathBuf> {
    if let Some(roots) = env::var_os("FRESHEN_APP_ROOTS") {
        return env::split_paths(&roots).collect();
    }
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots
}

fn report_cask_apps(apps: &[String]) {
    if apps.is_empty() {
        return;
    }

    human!("Cask apps: {}", apps.join(", "));

    let installed = installed_app_paths(apps, &app_roots());
    for path in installed {
        human!("Installed cask app: {}", path.display());
    }
}

/// The Brewfile `brew bundle cleanup` reads.
///
/// Homebrew reads `Brewfile` from the working directory, unless
/// `HOMEBREW_BUNDLE_FILE` points at another file. Returns `None` when there
/// is no such file, because `brew bundle cleanup` then has nothing to do.
fn brewfile() -> Option<PathBuf> {
    let file = match env::var_os("HOMEBREW_BUNDLE_FILE") {
        Some(file) if !file.is_empty() => Some(PathBuf::from(file)),
        _ => None,
    };

    brewfile_in(&env::current_dir().ok()?, file)
}

/// The Brewfile in `directory`, when there is one.
///
/// `file` is the file `HOMEBREW_BUNDLE_FILE` points at, if it is set.
fn brewfile_in(directory: &Path, file: Option<PathBuf>) -> Option<PathBuf> {
    let file = file.unwrap_or_else(|| PathBuf::from("Brewfile"));
    let path = if file.is_absolute() {
        file
    } else {
        directory.join(file)
    };

    path.is_file().then_some(path)
}

/// The notice `brew doctor` prints before its first warning.
///
/// The notice explains that the warnings are meant for the Homebrew
/// maintainers; it is not a finding of its own.
const DOCTOR_NOTICE: &str = "Please note that these warnings are just used to help";

/// Parses the findings of `brew doctor`.
///
/// Homebrew prints every finding as a `Warning: ` block, with the remedy on
/// the lines below it. The findings are printed on stderr; older Homebrew
/// versions printed them on stdout, so both are looked at.
fn parse_doctor_findings(stderr: &str, stdout: &str) -> Vec<DoctorFinding> {
    let findings = parse_doctor_warnings(stderr);
    if findings.is_empty() {
        return parse_doctor_warnings(stdout);
    }
    findings
}

/// Parses the `Warning: ` blocks of a `brew doctor` output.
fn parse_doctor_warnings(output: &str) -> Vec<DoctorFinding> {
    let mut findings: Vec<DoctorFinding> = Vec::new();
    let mut block: Vec<String> = Vec::new();
    let mut in_notice = false;

    for line in output.lines() {
        let line = line.trim();

        // The notice that precedes the first warning ends at the empty line
        // below it.
        if in_notice && !line.starts_with("Warning: ") {
            in_notice = !line.is_empty();
            continue;
        }
        in_notice = false;

        if line.starts_with(DOCTOR_NOTICE) {
            in_notice = true;
            continue;
        }

        if line.is_empty() {
            continue;
        }

        if let Some(finding) = line.strip_prefix("Warning: ") {
            if !block.is_empty() {
                findings.push(DoctorFinding::from_block(&block));
            }
            block.clear();
            block.push(finding.trim().to_owned());
            continue;
        }

        // A line that follows a warning belongs to it: it is the remedy
        // Homebrew suggests for the finding.
        if !block.is_empty() {
            block.push(line.to_owned());
        }
    }

    if !block.is_empty() {
        findings.push(DoctorFinding::from_block(&block));
    }

    findings
}

/// The hint `brew bundle cleanup` prints after it listed the packages.
const BUNDLE_HINT: &str = "Run `brew bundle cleanup --force`";

/// Parses the packages `brew bundle cleanup` would remove.
///
/// The command prints one group per package kind, with the packages of the
/// group below it.
fn parse_bundle_cleanup(stdout: &str) -> Vec<BundlePackage> {
    let mut packages = Vec::new();
    let mut kind = "package";

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(BUNDLE_HINT) {
            continue;
        }

        if let Some(header) = line.strip_suffix(':') {
            kind = bundle_kind(header);
            continue;
        }

        packages.push(BundlePackage {
            name: line.to_owned(),
            kind: kind.to_owned(),
        });
    }

    packages
}

/// The kind of package a `brew bundle cleanup` group names.
fn bundle_kind(header: &str) -> &'static str {
    let header = header.to_ascii_lowercase();
    if header.contains("cask") {
        "cask"
    } else if header.contains("formula") || header.contains("brew") {
        "formula"
    } else if header.contains("tap") {
        "tap"
    } else {
        "package"
    }
}

struct UpdateStep;

impl Step for UpdateStep {
    fn id(&self) -> StepId {
        BREW
    }

    fn display_name(&self) -> &str {
        "Homebrew"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_brew())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let brews = BrewSet::detect();
        for install in brews.installs() {
            human!("{}", install_message(&brews, install, "update"));
            run_brew(BREW, install.command().arg("update"))?;
        }
        Ok(())
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

struct FormulaeStep {
    services: ServiceQueue,
}

impl FormulaeStep {
    fn new(services: ServiceQueue) -> Self {
        Self { services }
    }

    fn run_install(
        &self,
        brews: &BrewSet,
        install: &BrewInstall,
        options: &RunOptions,
    ) -> Result<(), StepError> {
        let outdated = match install
            .command()
            .args(["outdated", "--formula", "--quiet"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            Ok(outdated) => Some(lines(outdated.stdout())),
            Err(error) => {
                eprintln!("freshen: could not list outdated formulae: {error}");
                None
            }
        };

        let Some(formulae) = outdated else {
            return run_brew(FORMULAE, install.command().args(["upgrade", "--formula"]))
                .map(|_| ());
        };

        let filter = options.filters().packages();
        let selected: Vec<String> = formulae
            .iter()
            .filter(|name| filter.includes(name))
            .cloned()
            .collect();

        if selected.is_empty() {
            if formulae.is_empty() {
                human!(
                    "{}",
                    install_message(brews, install, "No outdated formulae.")
                );
            } else {
                human!(
                    "{}",
                    install_message(
                        brews,
                        install,
                        "No outdated formulae left after applying the include/exclude lists."
                    )
                );
            }
            return Ok(());
        }

        let pinned = pinned_formulae(install).unwrap_or_default();
        let (held_back, upgradable): (Vec<String>, Vec<String>) = selected
            .into_iter()
            .partition(|name| is_pinned(name, &pinned));

        if !held_back.is_empty() {
            human!(
                "{}",
                install_message(
                    brews,
                    install,
                    format!("Held back (pinned): {}", held_back.join(" "))
                )
            );
        }

        warn_about_breakable_sessions(&upgradable.join("\n"));

        if upgradable.is_empty() {
            human!(
                "{}",
                install_message(
                    brews,
                    install,
                    "No formulae upgraded: every outdated formula is held back (pinned)."
                )
            );
            return Ok(());
        }

        let mut spec = install.command().args(["upgrade", "--formula"]);
        if !filter.is_empty() || !held_back.is_empty() {
            for name in &upgradable {
                spec = spec.arg(name);
            }
        }

        capture_running_services(&upgradable, install, &self.services);
        run_brew(FORMULAE, spec).map(|_| ())
    }
}

impl Step for FormulaeStep {
    fn id(&self) -> StepId {
        FORMULAE
    }

    fn display_name(&self) -> &str {
        "Homebrew formulae"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_brew())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        let brews = BrewSet::detect();
        let mut items = Vec::new();
        for install in brews.installs() {
            let report = outdated(FORMULAE, install, &["--formula"])?
                .filtered(|name| options.filters().packages().includes(name));
            if !report.is_up_to_date() {
                let pinned = pinned_formulae(install).unwrap_or_default();
                let report = with_pinned_reasons(report, &pinned);
                let report = qualify_items(report, install, brews.is_multiple());
                items.extend(report.items().iter().cloned());
            }
        }
        Ok(CheckReport::from_items(items))
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let brews = BrewSet::detect();
        for install in brews.installs() {
            self.run_install(&brews, install, options)?;
        }
        Ok(())
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[BREW]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

struct ServicesStep {
    services: ServiceQueue,
}

impl ServicesStep {
    fn new(services: ServiceQueue) -> Self {
        Self { services }
    }

    fn restart_queued(&self, message: &str) -> Result<(), StepError> {
        let services = self.services.services();
        if services.is_empty() {
            return Ok(());
        }

        let names: Vec<String> = services
            .iter()
            .map(|service| service.name.clone())
            .collect();
        human!("{message}: {}", names.join(" "));
        let mut restarted = Vec::new();
        let mut failures = Vec::new();

        for service in services {
            let result = run_brew(
                SERVICES,
                service
                    .install
                    .command()
                    .args(["services", "restart", &service.name]),
            );
            match result {
                Ok(_) => {
                    self.services.remove(&service);
                    restarted.push(service.name);
                }
                Err(error) => failures.push(error),
            }
        }

        if !restarted.is_empty() {
            human!("Restarted Homebrew services: {}", restarted.join(" "));
        }

        match failures.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Step for ServicesStep {
    fn id(&self) -> StepId {
        SERVICES
    }

    fn display_name(&self) -> &str {
        "Homebrew services"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_brew())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.restart_queued("Restarting Homebrew services")
    }

    fn restore(&self) -> Result<(), StepError> {
        self.restart_queued("Restoring Homebrew services")
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[FORMULAE]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

struct CasksStep {
    reopen: ReopenQueue,
    system: Arc<dyn AppSystem>,
    interactive: bool,
}

/// The bundle id of the terminal running freshen, used to protect it from being
/// quit.
///
/// `__CFBundleIdentifier` is authoritative when set. Otherwise the detected
/// terminal app supplies its bundle id, which covers hosts identified only
/// through `TERM_PROGRAM` or an environment marker.
fn own_bundle_id(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    get("__CFBundleIdentifier")
        .filter(|bundle_id| !bundle_id.is_empty())
        .or_else(|| detect_terminal_app(&get).and_then(|app| app.bundle_id().map(str::to_string)))
}

/// Which apps are quit before a cask upgrade and whether the user is asked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QuitPlan {
    /// The apps that are quit even when the user skips quitting the others.
    always: Vec<RunningApp>,

    /// Whether the user is asked before anything is quit.
    ask: bool,
}

/// Decides which of the running apps are quit and whether the user is asked.
///
/// The apps on the always-quit list are quit without asking, so a run whose
/// apps are all on that list never shows the prompt. Every other app is quit
/// only when the user says so.
fn quit_plan(scan: &AppScan, app_lists: &AppLists, assume_yes: bool) -> QuitPlan {
    let always: Vec<RunningApp> = scan
        .running()
        .iter()
        .filter(|app| app_lists.is_always_quit(app.bundle_id(), app.name()))
        .cloned()
        .collect();
    let everything_is_always = !always.is_empty() && always.len() == scan.running().len();

    QuitPlan {
        always,
        ask: !assume_yes && !everything_is_always,
    }
}

/// The app bundles the given casks install.
fn apps_of(metadata: &CaskMetadata, casks: &[String]) -> Vec<String> {
    metadata
        .casks()
        .iter()
        .filter(|cask| {
            cask.token()
                .is_some_and(|token| casks.iter().any(|name| name == token))
        })
        .flat_map(|cask| cask.apps().iter().cloned())
        .collect()
}

/// The bundle ids the given casks ask to quit in their `uninstall` stanza.
fn quit_bundle_ids_of(metadata: &CaskMetadata, casks: &[String]) -> Vec<String> {
    metadata
        .casks()
        .iter()
        .filter(|cask| {
            cask.token()
                .is_some_and(|token| casks.iter().any(|name| name == token))
        })
        .flat_map(|cask| cask.quit_bundle_ids().iter().cloned())
        .collect()
}

/// The command that reads the metadata of the given casks.
fn cask_info_command(install: &BrewInstall, casks: &[String]) -> CommandSpec {
    let mut info = install
        .command()
        .args(["info", "--cask", "--json=v2"])
        .neutral_locale()
        .retry_transient_network(NETWORK_RETRIES);
    for cask in casks {
        info = info.arg(cask);
    }
    info
}

impl CasksStep {
    fn new(reopen: ReopenQueue) -> Self {
        Self {
            reopen,
            system: Arc::new(MacAppSystem),
            interactive: crate::system::terminal_is_interactive(),
        }
    }

    fn run_install(
        &self,
        brews: &BrewSet,
        install: &BrewInstall,
        options: &RunOptions,
    ) -> Result<(), StepError> {
        let outdated = run_brew(
            CASKS,
            install
                .command()
                .args(["outdated", "--cask", "--greedy", "--quiet"])
                .neutral_locale(),
        )?;
        let filter = options.filters().casks();
        let casks: Vec<String> = lines(outdated.stdout())
            .into_iter()
            .filter(|cask| filter.includes(cask))
            .collect();

        if casks.is_empty() {
            human!("{}", install_message(brews, install, "No outdated casks."));
            return Ok(());
        }

        human!(
            "{}",
            install_message(
                brews,
                install,
                format!("Casks to upgrade: {}", casks.join(" "))
            )
        );

        let mut choice = QuitChoice::Quit;
        let mut selected = casks.clone();
        let mut needs_greedy = true;
        match cask_info_command(install, &casks).run() {
            Ok(output) => match cask_metadata(output.stdout()) {
                Ok(metadata) => {
                    let (upgraded, skipped) = self.select_casks(&casks, &metadata);
                    for cask in &skipped {
                        human!("warning: {}", administrator_skip_message(cask));
                    }
                    selected = upgraded;
                    needs_greedy = selected.iter().any(|cask| metadata.needs_greedy(cask));

                    let apps = apps_of(&metadata, &selected);
                    let bundle_ids = quit_bundle_ids_of(&metadata, &selected);
                    report_cask_apps(&apps);
                    choice = self.quit_running_apps(&apps, &bundle_ids, options);
                }
                Err(error) => eprintln!("freshen: could not read cask metadata: {error}"),
            },
            Err(error) => eprintln!("freshen: could not inspect casks: {error}"),
        }

        if choice == QuitChoice::SkipCasks {
            human!("warning: Cask upgrade skipped by user.");
            return Ok(());
        }

        if selected.is_empty() {
            human!(
                "{}",
                install_message(
                    brews,
                    install,
                    "No casks upgraded: every outdated cask needs an administrator password."
                )
            );
            return Ok(());
        }

        let mut spec = install.command().args(["upgrade", "--cask"]);
        if needs_greedy {
            spec = spec.arg("--greedy");
        }
        if !filter.is_empty() || selected.len() != casks.len() {
            for cask in &selected {
                spec = spec.arg(cask);
            }
        }

        run_brew(CASKS, spec).map(|_| ())
    }

    #[cfg(test)]
    fn with_system(reopen: ReopenQueue, system: Arc<dyn AppSystem>) -> Self {
        Self {
            reopen,
            system,
            interactive: false,
        }
    }

    #[cfg(test)]
    fn with_interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    /// The casks to upgrade and the casks to skip.
    ///
    /// Homebrew installs a cask that asks for an administrator password with
    /// `sudo`, which prompts on the terminal. A run without a terminal cannot
    /// answer that prompt, so the cask is skipped instead of waiting for a
    /// password that never comes.
    fn select_casks(
        &self,
        casks: &[String],
        metadata: &CaskMetadata,
    ) -> (Vec<String>, Vec<String>) {
        let skipped: Vec<String> = if self.interactive {
            Vec::new()
        } else {
            casks
                .iter()
                .filter(|cask| metadata.needs_administrator(cask))
                .cloned()
                .collect()
        };

        let selected = casks
            .iter()
            .filter(|cask| !skipped.contains(cask))
            .cloned()
            .collect();

        (selected, skipped)
    }

    /// Adds the reason each outdated cask needs `--greedy` to its report.
    ///
    /// The check output names why a cask only shows up with `--greedy`, so a
    /// self-updating cask is not mistaken for a normal one. When the metadata
    /// cannot be read the report is returned unchanged; the check itself still
    /// succeeds.
    fn with_greedy_reasons(&self, report: CheckReport, install: &BrewInstall) -> CheckReport {
        let casks: Vec<String> = report
            .items()
            .iter()
            .map(|item| item.name().to_owned())
            .collect();
        if casks.is_empty() {
            return report;
        }

        let metadata = match cask_info_command(install, &casks).run() {
            Ok(output) => match cask_metadata(output.stdout()) {
                Ok(metadata) => metadata,
                Err(error) => {
                    eprintln!("freshen: could not read cask metadata: {error}");
                    return report;
                }
            },
            Err(error) => {
                eprintln!("freshen: could not inspect casks: {error}");
                return report;
            }
        };

        CheckReport::from_items(report.items().iter().map(|item| {
            match metadata.greedy_reason(item.name()) {
                Some(reason) => item.clone().with_reason(reason),
                None => item.clone(),
            }
        }))
    }

    fn quit_running_apps(
        &self,
        apps: &[String],
        bundle_ids: &[String],
        options: &RunOptions,
    ) -> QuitChoice {
        let installed = installed_app_paths(apps, &app_roots());
        let self_bundle_id = own_bundle_id(|key| env::var(key).ok());
        let system = self.system.as_ref();
        let scan = find_running_cask_apps(
            system,
            &installed,
            bundle_ids,
            self_bundle_id.as_deref(),
            options.app_lists(),
        );

        for name in scan.protected() {
            human!(
                "warning: {name} is the terminal running this script, not quitting it (restart it manually after the upgrade)."
            );
        }

        for name in scan.never_quit() {
            human!("warning: {name} is on the never-quit list, not quitting it.");
        }

        for app in scan.updating() {
            human!(
                "{} is updating itself, so freshen waits for the update to finish before quitting it.",
                app.name()
            );
        }

        if scan.is_empty() {
            return QuitChoice::Quit;
        }

        human!("Running apps that will be quit and reopened:");
        for app in scan.running() {
            human!("   - {}", app.name());
        }

        let always_quit = quit_plan(&scan, options.app_lists(), options.assume_yes());

        let choice = if always_quit.ask {
            ask_quit_choice()
        } else {
            QuitChoice::Quit
        };

        if choice == QuitChoice::SkipCasks {
            return choice;
        }

        let to_quit: Vec<RunningApp> = match choice {
            QuitChoice::Quit => scan.running().to_vec(),
            QuitChoice::SkipQuit => always_quit.always,
            QuitChoice::SkipCasks => Vec::new(),
        };

        if !to_quit.is_empty() {
            let policy = QuitPolicy::from_env(options.force_kill());
            let reopen_ids = quit_apps(system, &to_quit, &policy);
            self.reopen.extend(reopen_ids);
        }

        choice
    }
}

fn ask_quit_choice() -> QuitChoice {
    human_print!("Quit these apps and upgrade casks? [Y]es / [s]kip quitting / [n]o: ");
    flush();

    let mut answer = String::new();
    let _ = io::stdin().read_line(&mut answer);
    quit_choice(&answer)
}

impl Step for CasksStep {
    fn id(&self) -> StepId {
        CASKS
    }

    fn display_name(&self) -> &str {
        "Homebrew casks"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_brew())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        cask_metadata_probe().map_err(|error| {
            StepError::new(
                CASKS,
                Operation::Check,
                format!("cask metadata probe failed: {error}"),
            )
        })?;
        let brews = BrewSet::detect();
        let mut items = Vec::new();
        for install in brews.installs() {
            let report = outdated(CASKS, install, &["--cask", "--greedy"])?
                .filtered(|name| options.filters().casks().includes(name));
            let report = self.with_greedy_reasons(report, install);
            let report = qualify_items(report, install, brews.is_multiple());
            items.extend(report.items().iter().cloned());
        }
        Ok(CheckReport::from_items(items))
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let brews = BrewSet::detect();
        for install in brews.installs() {
            self.run_install(&brews, install, options)?;
        }
        Ok(())
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn restore(&self) -> Result<(), StepError> {
        reopen_queued(
            &self.reopen,
            self.system.as_ref(),
            "Restoring apps that were quit",
            false,
        );
        Ok(())
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[FORMULAE]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

struct AutoremoveStep;

impl Step for AutoremoveStep {
    fn id(&self) -> StepId {
        AUTOREMOVE
    }

    fn display_name(&self) -> &str {
        "Homebrew autoremove"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_brew())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let brews = BrewSet::detect();
        for install in brews.installs() {
            cleanup::run_guarded(
                AUTOREMOVE,
                &install_message(&brews, install, "Homebrew autoremove"),
                install
                    .command()
                    .args(["autoremove", "--dry-run"])
                    .neutral_locale(),
                install
                    .command()
                    .arg("autoremove")
                    .retry_transient_network(NETWORK_RETRIES),
                options,
            )?;
        }
        Ok(())
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[CASKS]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

struct CleanupStep;

impl Step for CleanupStep {
    fn id(&self) -> StepId {
        CLEANUP
    }

    fn display_name(&self) -> &str {
        "Homebrew cleanup"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_brew())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let brews = BrewSet::detect();
        for install in brews.installs() {
            cleanup::run_guarded(
                CLEANUP,
                &install_message(&brews, install, "Homebrew cleanup"),
                install
                    .command()
                    .args(["cleanup", "--dry-run", "--prune=all"])
                    .neutral_locale(),
                install
                    .command()
                    .args(["cleanup", "--prune=all"])
                    .retry_transient_network(NETWORK_RETRIES),
                options,
            )?;
        }
        Ok(())
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[AUTOREMOVE]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

/// The `brew doctor` step: a report-only check of the Homebrew installation.
///
/// `brew doctor` never changes anything: it reports what it found and exits
/// with a non-zero status when it found something. freshen shows the findings
/// and never tries to fix them.
struct DoctorStep {
    /// The findings of the last `brew doctor`, so that a run does not ask the
    /// command twice.
    findings: Mutex<Option<Vec<DoctorFinding>>>,
}

/// One finding of `brew doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorFinding {
    /// What Homebrew found.
    message: String,
    /// How to fix it, when Homebrew says so.
    remedy: Option<String>,
}

impl DoctorFinding {
    /// The item the check report shows for this finding.
    fn item(&self) -> OutdatedItem {
        let item = OutdatedItem::new(self.message.clone());
        match &self.remedy {
            Some(remedy) => item.with_reason(remedy.clone()),
            None => item,
        }
    }

    /// Builds a finding from the lines of one `Warning: ` block.
    ///
    /// The first line is what Homebrew found, the lines below it are the
    /// remedy it suggests.
    fn from_block(lines: &[String]) -> Self {
        let Some((message, rest)) = lines.split_first() else {
            return Self {
                message: String::new(),
                remedy: None,
            };
        };

        let remedy = if rest.is_empty() {
            None
        } else {
            Some(rest.join(" "))
        };

        Self {
            message: message.clone(),
            remedy,
        }
    }
}

impl DoctorStep {
    fn new() -> Self {
        Self {
            findings: Mutex::new(None),
        }
    }

    /// The findings of `brew doctor`.
    ///
    /// The command only runs when the check did not run yet, so that a run
    /// asks Homebrew once and not twice.
    fn findings(&self, operation: Operation) -> Result<Vec<DoctorFinding>, StepError> {
        if let Some(findings) = lock(&self.findings).clone() {
            return Ok(findings);
        }

        let findings = self.doctor(operation)?;
        *lock(&self.findings) = Some(findings.clone());
        Ok(findings)
    }

    /// Runs `brew doctor` and parses what it printed.
    fn doctor(&self, operation: Operation) -> Result<Vec<DoctorFinding>, StepError> {
        let brews = BrewSet::detect();
        let mut all = Vec::new();
        for install in brews.installs() {
            let mut findings = self.doctor_install(install, operation)?;
            if brews.is_multiple() {
                for finding in &mut findings {
                    finding.message = format!("{}: {}", install.prefix.display(), finding.message);
                }
            }
            all.extend(findings);
        }
        Ok(all)
    }

    /// Runs `brew doctor` for one installation.
    fn doctor_install(
        &self,
        install: &BrewInstall,
        operation: Operation,
    ) -> Result<Vec<DoctorFinding>, StepError> {
        let output = install
            .command()
            .args(["doctor"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run();

        match output {
            Ok(output) => Ok(parse_doctor_findings(output.stderr(), output.stdout())),
            Err(error) => {
                // `brew doctor` exits with a non-zero status when it found
                // something, so its output is the report and not a failure.
                let Some(output) = error.output() else {
                    return Err(brew_failure(DOCTOR, operation, &error));
                };
                let findings = parse_doctor_findings(output.stderr(), output.stdout());
                if findings.is_empty() {
                    return Err(brew_failure(DOCTOR, operation, &error));
                }
                Ok(findings)
            }
        }
    }
}

impl Step for DoctorStep {
    fn id(&self) -> StepId {
        DOCTOR
    }

    fn display_name(&self) -> &str {
        "Homebrew doctor"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_brew())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let findings = self.findings(Operation::Check)?;
        Ok(CheckReport::from_items(
            findings.iter().map(DoctorFinding::item),
        ))
    }

    fn run(&self) -> Result<(), StepError> {
        let findings = self.findings(Operation::Run)?;
        if findings.is_empty() {
            human!("brew-doctor: `brew doctor` found no problems");
            return Ok(());
        }

        human!(
            "brew-doctor: {} warning(s) from `brew doctor` (nothing is changed)",
            findings.len()
        );
        for finding in &findings {
            human!("  - {}", finding.item());
        }
        Ok(())
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[BREW]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

/// The `brew bundle cleanup` step: a report-only check of the Brewfile.
///
/// `brew bundle cleanup` removes every package that is not in the Brewfile.
/// Without `--force` it only lists what it would remove, and that is the form
/// freshen runs: the step reports the packages, it never removes one.
struct BundleCleanupStep {
    /// The packages of the last `brew bundle cleanup`, so that a run does not
    /// ask the command twice.
    packages: Mutex<Option<Vec<BundlePackage>>>,
}

/// One package `brew bundle cleanup` would remove.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BundlePackage {
    /// Name of the package.
    name: String,
    /// What kind of package it is, for example `"formula"`.
    kind: String,
}

impl BundlePackage {
    /// The item the check report shows for this package.
    fn item(&self) -> OutdatedItem {
        OutdatedItem::new(self.name.clone())
            .with_reason(format!("{} not in the Brewfile", self.kind))
    }
}

impl BundleCleanupStep {
    fn new() -> Self {
        Self {
            packages: Mutex::new(None),
        }
    }

    /// The packages `brew bundle cleanup` would remove.
    ///
    /// The command only runs when the check did not run yet, so that a run
    /// asks Homebrew once and not twice.
    fn packages(&self, operation: Operation) -> Result<Vec<BundlePackage>, StepError> {
        if let Some(packages) = lock(&self.packages).clone() {
            return Ok(packages);
        }

        let packages = self.cleanup(operation)?;
        *lock(&self.packages) = Some(packages.clone());
        Ok(packages)
    }

    /// Runs `brew bundle cleanup` and parses what it printed.
    fn cleanup(&self, operation: Operation) -> Result<Vec<BundlePackage>, StepError> {
        let brews = BrewSet::detect();
        let mut all = Vec::new();
        for install in brews.installs() {
            let mut packages = self.cleanup_install(install, operation)?;
            if brews.is_multiple() {
                for package in &mut packages {
                    package.name = format!("{}: {}", install.prefix.display(), package.name);
                }
            }
            all.extend(packages);
        }
        Ok(all)
    }

    /// Runs `brew bundle cleanup` for one installation.
    fn cleanup_install(
        &self,
        install: &BrewInstall,
        operation: Operation,
    ) -> Result<Vec<BundlePackage>, StepError> {
        let output = install
            .command()
            .args(["bundle", "cleanup"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run();

        match output {
            Ok(output) => Ok(parse_bundle_cleanup(output.stdout())),
            Err(error) => {
                // `brew bundle cleanup` exits with a non-zero status when it
                // found a package to remove, so its output is the report and
                // not a failure.
                let Some(output) = error.output() else {
                    return Err(brew_failure(BUNDLE_CLEANUP, operation, &error));
                };
                let packages = parse_bundle_cleanup(output.stdout());
                if packages.is_empty() {
                    return Err(brew_failure(BUNDLE_CLEANUP, operation, &error));
                }
                Ok(packages)
            }
        }
    }

    /// The detection of the step, given what the machine offers.
    fn detection(brew_installed: bool, has_brewfile: bool) -> Detection {
        if !brew_installed {
            return Detection::NotInstalled;
        }
        if !has_brewfile {
            return Detection::Unavailable(SkipReason::MissingConfiguration);
        }
        Detection::Installed
    }
}

impl Step for BundleCleanupStep {
    fn id(&self) -> StepId {
        BUNDLE_CLEANUP
    }

    fn display_name(&self) -> &str {
        "Homebrew bundle cleanup"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(Self::detection(
            detect_brew().is_installed(),
            brewfile().is_some(),
        ))
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let packages = self.packages(Operation::Check)?;
        Ok(CheckReport::from_items(
            packages.iter().map(BundlePackage::item),
        ))
    }

    fn run(&self) -> Result<(), StepError> {
        let packages = self.packages(Operation::Run)?;
        if packages.is_empty() {
            human!("brew-bundle-cleanup: every package is in the Brewfile");
            return Ok(());
        }

        human!(
            "brew-bundle-cleanup: {} package(s) are not in the Brewfile (nothing is removed)",
            packages.len()
        );
        for package in &packages {
            human!("  - {}", package.item());
        }
        Ok(())
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[CLEANUP]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

struct ReopenStep {
    reopen: ReopenQueue,
    system: Arc<dyn AppSystem>,
}

impl ReopenStep {
    fn new(reopen: ReopenQueue) -> Self {
        Self {
            reopen,
            system: Arc::new(MacAppSystem),
        }
    }

    #[cfg(test)]
    fn with_system(reopen: ReopenQueue, system: Arc<dyn AppSystem>) -> Self {
        Self { reopen, system }
    }
}

impl Step for ReopenStep {
    fn id(&self) -> StepId {
        REOPEN
    }

    fn display_name(&self) -> &str {
        "Homebrew app reopen"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_brew())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        reopen_queued(
            &self.reopen,
            self.system.as_ref(),
            "Reopening apps that were quit",
            crate::signals::requested(),
        );
        Ok(())
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn restore(&self) -> Result<(), StepError> {
        reopen_queued(
            &self.reopen,
            self.system.as_ref(),
            "Restoring apps that were quit",
            false,
        );
        Ok(())
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[CLEANUP]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::brew_installs_from;
    use super::{
        administrator_skip_message, apps_of, brewfile_in, cask_app_artifacts, cask_metadata,
        command_line_tools_failure, install_arch, installed_app_paths, is_pinned, own_bundle_id,
        parse_bundle_cleanup, parse_doctor_findings, parse_outdated, quit_bundle_ids_of, quit_plan,
        register, running_services, services_for_formulae, BundleCleanupStep, BundlePackage,
        CaskError, CasksStep, DoctorFinding, ReopenQueue, ReopenStep, CASKS, FORMULAE, REOPEN,
        SERVICES,
    };
    use crate::apps::{AppScan, AppSystem, QuitOutcome, Reopen, RunningApp, SessionState};
    use crate::platform::Platform;
    use crate::policy::AppLists;
    use crate::registry::Registry;
    use crate::step::{CheckReport, Detection, Operation, SkipReason, Step, StepId};
    use crate::system::Arch;
    use std::path::Path;
    #[cfg(unix)]
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeAppSystem {
        opened: Mutex<Vec<String>>,
    }

    impl AppSystem for FakeAppSystem {
        fn bundle_id(&self, _app: &Path) -> Option<String> {
            None
        }

        fn is_running(&self, _app: &Path) -> bool {
            false
        }

        fn pids(&self, _app: &Path) -> Vec<u32> {
            Vec::new()
        }

        fn quit(&self, _bundle_id: &str) -> QuitOutcome {
            QuitOutcome::Quit
        }

        fn force_kill(&self, _pids: &[u32]) {}

        fn open(&self, app: &Reopen) -> bool {
            super::lock(&self.opened).push(app.bundle_id().to_owned());
            true
        }
    }

    /// A cask step that runs with or without a terminal.
    fn casks_step(interactive: bool) -> CasksStep {
        CasksStep::with_system(ReopenQueue::default(), Arc::new(FakeAppSystem::default()))
            .with_interactive(interactive)
    }

    fn ordered_ids(platform: Platform) -> Vec<StepId> {
        register(Registry::new())
            .ordered_for(platform)
            .unwrap_or_else(|error| panic!("registry order failed: {error}"))
            .iter()
            .map(|step| step.id())
            .collect()
    }

    #[test]
    fn the_error_about_the_outdated_command_line_tools_names_the_fix() {
        let failure = command_line_tools_failure(
            FORMULAE,
            Operation::Run,
            "Error: Your Command Line Tools are too outdated. \
             Update them from Software Update in the App Store.\n",
        )
        .unwrap_or_else(|| panic!("the failure should be reported"));

        assert_eq!(failure.step(), FORMULAE);
        assert_eq!(failure.operation(), Operation::Run);
        assert!(
            failure.message().contains("Command Line Tools"),
            "message: {}",
            failure.message()
        );
        assert!(
            failure
                .message()
                .contains("softwareupdate --install \"Command Line Tools for Xcode\""),
            "message: {}",
            failure.message()
        );
    }

    #[test]
    fn another_homebrew_failure_is_not_mistaken_for_the_outdated_command_line_tools() {
        assert!(command_line_tools_failure(
            FORMULAE,
            Operation::Run,
            "Error: No available formula with the name \"jq\".\n"
        )
        .is_none());
    }

    #[test]
    fn parses_formula_and_cask_outdated_lines_with_versions() {
        let formula = parse_outdated("jq (1.7.1) < 1.8.0");
        assert_eq!(formula.name(), "jq");
        assert_eq!(formula.current(), Some("1.7.1"));
        assert_eq!(formula.latest(), Some("1.8.0"));

        let cask = parse_outdated("firefox (121.0) != 122.0");
        assert_eq!(cask.name(), "firefox");
        assert_eq!(cask.current(), Some("121.0"));
        assert_eq!(cask.latest(), Some("122.0"));
    }

    #[test]
    fn parses_outdated_lines_without_versions() {
        let item = parse_outdated("node");

        assert_eq!(item.name(), "node");
        assert_eq!(item.current(), None);
        assert_eq!(item.latest(), None);
    }

    #[test]
    fn parses_tap_names_and_extra_suffixes() {
        let item = parse_outdated("user/tap/tool (0.9.0) < 1.0.0 (auto_updates)");

        assert_eq!(item.name(), "user/tap/tool");
        assert_eq!(item.current(), Some("0.9.0"));
        assert_eq!(item.latest(), Some("1.0.0"));
    }

    #[test]
    fn matches_pinned_formulae_with_and_without_the_tap() {
        let pinned = vec!["tool".to_owned(), "user/tap/other".to_owned()];

        assert!(is_pinned("tool", &pinned));
        assert!(is_pinned("user/tap/tool", &pinned));
        assert!(is_pinned("user/tap/other", &pinned));
        assert!(is_pinned("other", &pinned));
        assert!(!is_pinned("unpinned", &pinned));
    }

    #[test]
    fn registry_runs_formulae_before_services_before_casks_and_cleanup_last() {
        assert_eq!(
            ordered_ids(Platform::Macos),
            [
                "brew",
                "brew-formulae",
                "brew-services",
                "brew-casks",
                "brew-autoremove",
                "brew-cleanup",
                "brew-doctor",
                "brew-bundle-cleanup",
                "brew-reopen",
            ]
        );
        assert_eq!(
            ordered_ids(Platform::Linux),
            [
                "brew",
                "brew-formulae",
                "brew-services",
                "brew-casks",
                "brew-autoremove",
                "brew-cleanup",
                "brew-doctor",
                "brew-bundle-cleanup",
                "brew-reopen",
            ]
        );
    }

    #[test]
    fn parses_only_running_services_from_the_brew_services_table() {
        let output = "Name              Status  User File\n\
                      postgresql@16     started user ~/Library/LaunchAgents/homebrew.mxcl.postgresql@16.plist\n\
                      redis             stopped user ~/Library/LaunchAgents/homebrew.mxcl.redis.plist\n\
                      mysql             running user ~/Library/LaunchAgents/homebrew.mxcl.mysql.plist\n\
                      memcached         none\n\
                      broken            error   user ~/Library/LaunchAgents/homebrew.mxcl.broken.plist\n";

        assert_eq!(
            running_services(output),
            ["postgresql@16", "mysql"],
            "only started or running services need a restart"
        );
    }

    #[test]
    fn matches_running_services_to_upgraded_formulae_without_the_tap() {
        let output = "Name      Status\nuser/tap/tool  started\nother     started\n";
        let formulae = vec!["tool".to_owned(), "other".to_owned()];

        assert_eq!(
            services_for_formulae(output, &formulae),
            ["user/tap/tool", "other"]
        );
    }

    #[test]
    fn services_declare_formulae_as_a_prerequisite() {
        let registry = register(Registry::new());
        let services = registry
            .steps()
            .find(|step| step.id() == SERVICES)
            .unwrap_or_else(|| panic!("services step is not registered"));

        assert_eq!(services.prerequisites(), [FORMULAE]);
    }

    #[test]
    fn casks_declare_formulae_as_a_prerequisite() {
        let registry = register(Registry::new());
        let casks = registry
            .steps()
            .find(|step| step.id() == CASKS)
            .unwrap_or_else(|| panic!("casks step is not registered"));

        assert_eq!(casks.prerequisites(), [FORMULAE]);
    }

    #[test]
    fn reopen_runs_after_cleanup() {
        let registry = register(Registry::new());
        let reopen = registry
            .steps()
            .find(|step| step.id() == REOPEN)
            .unwrap_or_else(|| panic!("reopen step is not registered"));

        assert_eq!(reopen.prerequisites(), ["brew-cleanup"]);
    }

    #[test]
    fn parses_app_artifacts_from_a_recorded_cask_info_transcript() {
        let apps = cask_app_artifacts(include_str!("../tests/fixtures/brew-info-casks.json"))
            .unwrap_or_else(|error| panic!("fixture should parse: {error}"));

        assert_eq!(apps, ["Firefox.app", "Google Chrome.app"]);
    }

    #[test]
    fn accepts_string_and_target_artifact_forms() {
        let json = r#"{
            "casks": [
                {"artifacts": [
                    {"app": ["Plain.app"]},
                    {"app": [{"source": "Source.app", "target": "/Applications/Target.app"}]}
                ]}
            ]
        }"#;

        let apps = cask_app_artifacts(json)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert_eq!(apps, ["Plain.app", "Target.app"]);
    }

    #[test]
    fn ignores_non_app_and_malformed_artifacts() {
        let json = r#"{
            "casks": [
                {"artifacts": [
                    {"binary": [{"target": "tool"}]},
                    {"app": ["App.app", 7, {"target": 3}, {"source": "NoTarget.app"}]},
                    {"app": "not-an-array"}
                ]},
                {"no_artifacts": true}
            ]
        }"#;

        let apps = cask_app_artifacts(json)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert_eq!(apps, ["App.app"]);
    }

    #[test]
    fn a_transcript_without_casks_has_no_apps() {
        let apps = cask_app_artifacts(r#"{"formulae":[],"casks":[]}"#)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert!(apps.is_empty());
    }

    #[test]
    fn invalid_json_is_reported() {
        assert_eq!(
            cask_app_artifacts("not json"),
            Err(CaskError::Json(
                "expected ident at line 1 column 2".to_owned()
            ))
        );
    }

    /// A transcript with a cask that installs a macOS installer package, one
    /// whose uninstall script calls `sudo`, and one that needs neither.
    const CASKS_WITH_ADMINISTRATOR: &str = r#"{
        "casks": [
            {
                "token": "1password",
                "artifacts": [
                    {"pkg": ["1Password.pkg"]}
                ]
            },
            {
                "token": "some-tool",
                "artifacts": [
                    {"app": ["Some Tool.app"]},
                    {"uninstall": [{"script": {"executable": "uninstall.sh", "sudo": true}}]}
                ]
            },
            {
                "token": "firefox",
                "artifacts": [
                    {"app": ["Firefox.app"]}
                ]
            }
        ]
    }"#;

    #[test]
    fn a_pkg_installer_needs_an_administrator_password() {
        let metadata = cask_metadata(CASKS_WITH_ADMINISTRATOR)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert!(metadata.needs_administrator("1password"));
        assert!(!metadata.needs_administrator("firefox"));
    }

    #[test]
    fn a_sudo_script_needs_an_administrator_password() {
        let metadata = cask_metadata(CASKS_WITH_ADMINISTRATOR)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert!(metadata.needs_administrator("some-tool"));
    }

    #[test]
    fn a_sudo_command_in_a_script_needs_an_administrator_password() {
        let json = r#"{
            "casks": [
                {"token": "tool", "artifacts": [
                    {"uninstall": [{"script": "sudo rm -rf /usr/local/tool"}]}
                ]}
            ]
        }"#;

        let metadata =
            cask_metadata(json).unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert!(metadata.needs_administrator("tool"));
    }

    #[test]
    fn a_word_that_merely_contains_sudo_is_not_a_sudo_command() {
        let json = r#"{
            "casks": [
                {"token": "tool", "artifacts": [
                    {"binary": [{"target": "sudoku"}]},
                    {"uninstall": [{"script": {"executable": "clean.sh", "sudo": false}}]}
                ]}
            ]
        }"#;

        let metadata =
            cask_metadata(json).unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert!(!metadata.needs_administrator("tool"));
    }

    #[test]
    fn a_cask_the_metadata_does_not_describe_needs_no_administrator_password() {
        let metadata = cask_metadata(CASKS_WITH_ADMINISTRATOR)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert!(!metadata.needs_administrator("unknown-cask"));
    }

    const CASKS_WITH_GREEDY: &str = r#"{
        "casks": [
            {"token": "firefox", "version": "134.0", "artifacts": [{"app": ["Firefox.app"]}]},
            {"token": "spotify", "version": "1.2.60", "auto_updates": true,
             "artifacts": [{"app": ["Spotify.app"]}]},
            {"token": "nightly", "version": "latest",
             "artifacts": [{"app": ["Nightly.app"]}]},
            {"token": "both", "version": "latest", "auto_updates": true,
             "artifacts": [{"app": ["Both.app"]}]}
        ]
    }"#;

    #[test]
    fn parses_the_version_and_auto_updates_flag() {
        let metadata = cask_metadata(CASKS_WITH_GREEDY)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        let firefox = &metadata.casks()[0];
        assert_eq!(firefox.version(), Some("134.0"));
        assert!(!firefox.auto_updates());

        let spotify = &metadata.casks()[1];
        assert_eq!(spotify.version(), Some("1.2.60"));
        assert!(spotify.auto_updates());
    }

    #[test]
    fn only_latest_and_auto_updating_casks_need_greedy() {
        let metadata = cask_metadata(CASKS_WITH_GREEDY)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert!(!metadata.needs_greedy("firefox"));
        assert!(metadata.needs_greedy("spotify"));
        assert!(metadata.needs_greedy("nightly"));
        assert!(metadata.needs_greedy("both"));
        assert!(!metadata.needs_greedy("unknown-cask"));
    }

    #[test]
    fn the_greedy_reason_names_the_property_that_needs_it() {
        let metadata = cask_metadata(CASKS_WITH_GREEDY)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert_eq!(metadata.greedy_reason("firefox"), None);
        assert_eq!(
            metadata.greedy_reason("spotify").as_deref(),
            Some("auto_updates true; needs --greedy")
        );
        assert_eq!(
            metadata.greedy_reason("nightly").as_deref(),
            Some("version :latest; needs --greedy")
        );
        assert_eq!(
            metadata.greedy_reason("both").as_deref(),
            Some("version :latest and auto_updates true; needs --greedy")
        );
    }

    #[test]
    fn a_non_interactive_run_skips_the_casks_that_need_an_administrator_password() {
        let metadata = cask_metadata(CASKS_WITH_ADMINISTRATOR)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));
        let casks = vec!["1password".to_owned(), "firefox".to_owned()];
        let step = casks_step(false);

        let (selected, skipped) = step.select_casks(&casks, &metadata);

        assert_eq!(skipped, ["1password"]);
        assert_eq!(selected, ["firefox"]);
    }

    #[test]
    fn an_interactive_run_keeps_the_casks_that_need_an_administrator_password() {
        let metadata = cask_metadata(CASKS_WITH_ADMINISTRATOR)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));
        let casks = vec!["1password".to_owned(), "firefox".to_owned()];
        let step = casks_step(true);

        let (selected, skipped) = step.select_casks(&casks, &metadata);

        assert!(skipped.is_empty());
        assert_eq!(selected, ["1password", "firefox"]);
    }

    #[test]
    fn the_apps_of_a_skipped_cask_are_not_quit() {
        let metadata = cask_metadata(CASKS_WITH_ADMINISTRATOR)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        let apps = apps_of(&metadata, &["firefox".to_owned()]);

        assert_eq!(apps, ["Firefox.app"]);
    }

    /// A transcript with a menu-bar helper, a pkg installer and a cask whose
    /// `quit` list names several bundle ids.
    const CASKS_WITH_UNINSTALL_QUIT: &str = r#"{
        "casks": [
            {
                "token": "menu-bar",
                "artifacts": [
                    {"uninstall": [{"quit": "com.example.menubar", "launchctl": "com.example.helper"}]},
                    {"app": ["Menu Bar.app"]}
                ]
            },
            {
                "token": "pkg-tool",
                "artifacts": [
                    {"uninstall": [{"quit": {"bundle": "com.example.pkgtool"}}]},
                    {"pkg": ["Tool.pkg"]}
                ]
            },
            {
                "token": "multi",
                "artifacts": [
                    {"uninstall": [{"quit": ["com.example.one", "com.example.two"]}]}
                ]
            },
            {
                "token": "top-level",
                "uninstall": [{"quit": "com.example.toplevel"}],
                "artifacts": [{"pkg": ["Top.pkg"]}]
            },
            {
                "token": "no-quit",
                "artifacts": [
                    {"uninstall": [{"signal": ["TERM", "com.example.other"]}]},
                    {"pkg": ["Other.pkg"]}
                ]
            }
        ]
    }"#;

    #[test]
    fn reads_the_bundle_ids_from_the_uninstall_stanza() {
        let metadata = cask_metadata(CASKS_WITH_UNINSTALL_QUIT)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert_eq!(
            metadata.casks()[0].quit_bundle_ids(),
            ["com.example.menubar"]
        );
        assert_eq!(
            metadata.casks()[1].quit_bundle_ids(),
            ["com.example.pkgtool"]
        );
        assert_eq!(
            metadata.casks()[2].quit_bundle_ids(),
            ["com.example.one", "com.example.two"]
        );
        assert_eq!(
            metadata.casks()[3].quit_bundle_ids(),
            ["com.example.toplevel"]
        );
        assert!(metadata.casks()[4].quit_bundle_ids().is_empty());
    }

    #[test]
    fn only_the_quit_bundle_ids_of_the_selected_casks_are_collected() {
        let metadata = cask_metadata(CASKS_WITH_UNINSTALL_QUIT)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        let bundle_ids = quit_bundle_ids_of(
            &metadata,
            &[
                "menu-bar".to_owned(),
                "multi".to_owned(),
                "no-quit".to_owned(),
            ],
        );

        assert_eq!(
            bundle_ids,
            ["com.example.menubar", "com.example.one", "com.example.two"]
        );
    }

    #[test]
    fn a_cask_without_an_uninstall_stanza_has_no_quit_bundle_ids() {
        let metadata = cask_metadata(CASKS_WITH_ADMINISTRATOR)
            .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

        assert!(metadata.casks()[1].quit_bundle_ids().is_empty());
    }

    #[test]
    fn the_skip_message_names_the_cask_and_the_reason() {
        let message = administrator_skip_message("1password");

        assert!(message.contains("1password"), "message: {message}");
        assert!(
            message.contains("administrator password"),
            "message: {message}"
        );
    }

    #[test]
    fn an_interrupted_cask_step_reopens_the_apps_it_quit() {
        let queue = ReopenQueue::default();
        queue.extend([
            Reopen::new("com.firefox", SessionState::visible()),
            Reopen::new("com.editor", SessionState::visible()),
        ]);
        let system = Arc::new(FakeAppSystem::default());
        let step = CasksStep::with_system(queue.clone(), system.clone());

        step.restore()
            .unwrap_or_else(|error| panic!("restore failed: {error}"));

        assert_eq!(
            super::lock(&system.opened).as_slice(),
            ["com.firefox", "com.editor"]
        );
        assert!(queue.apps().is_empty());
    }

    #[test]
    fn the_reopen_step_restores_apps_after_an_interrupt() {
        let queue = ReopenQueue::default();
        queue.extend([Reopen::new("com.firefox", SessionState::visible())]);
        let system = Arc::new(FakeAppSystem::default());
        let step = ReopenStep::with_system(queue.clone(), system.clone());

        step.restore()
            .unwrap_or_else(|error| panic!("restore failed: {error}"));

        assert_eq!(super::lock(&system.opened).as_slice(), ["com.firefox"]);
        assert!(queue.apps().is_empty());
    }

    #[test]
    fn installed_apps_are_resolved_in_root_order() {
        let root =
            std::env::temp_dir().join(format!("freshen-homebrew-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Applications/Firefox.app"))
            .unwrap_or_else(|error| panic!("failed to create test app: {error}"));

        let apps = ["Firefox.app".to_owned(), "Missing.app".to_owned()];
        let paths = installed_app_paths(
            &apps,
            &[root.join("Applications"), root.join("OtherApplications")],
        );

        assert_eq!(paths, [root.join("Applications/Firefox.app")]);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn bundle_id_from(vars: &[(&str, &str)]) -> Option<String> {
        own_bundle_id(|key| {
            vars.iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        })
    }

    #[test]
    fn cf_bundle_identifier_takes_precedence_over_term_program() {
        let bundle_id = bundle_id_from(&[
            ("__CFBundleIdentifier", "com.example.host"),
            ("TERM_PROGRAM", "Apple_Terminal"),
        ]);

        assert_eq!(bundle_id.as_deref(), Some("com.example.host"));
    }

    #[test]
    fn an_empty_cf_bundle_identifier_falls_back_to_term_program() {
        let bundle_id =
            bundle_id_from(&[("__CFBundleIdentifier", ""), ("TERM_PROGRAM", "iTerm.app")]);

        assert_eq!(bundle_id.as_deref(), Some("com.googlecode.iterm2"));
    }

    #[test]
    fn a_terminal_identified_by_an_environment_marker_supplies_its_bundle_id() {
        let bundle_id = bundle_id_from(&[("KITTY_WINDOW_ID", "1")]);

        assert_eq!(bundle_id.as_deref(), Some("net.kovidgoyal.kitty"));
    }

    #[test]
    fn an_unknown_terminal_without_a_bundle_id_protects_nothing() {
        assert_eq!(
            bundle_id_from(&[("TERM_PROGRAM", "unknown-terminal")]),
            None
        );
    }

    fn scan_with(apps: &[(&str, &str)]) -> AppScan {
        let running = apps
            .iter()
            .map(|(bundle_id, name)| {
                RunningApp::new(
                    std::path::PathBuf::from(format!("/Applications/{name}")),
                    *bundle_id,
                )
            })
            .collect();
        AppScan::with_running(running)
    }

    #[test]
    fn the_user_is_asked_when_no_app_is_on_the_always_quit_list() {
        let scan = scan_with(&[("com.firefox", "Firefox.app")]);

        let plan = quit_plan(&scan, &AppLists::new(), false);

        assert!(plan.ask);
        assert!(plan.always.is_empty());
    }

    #[test]
    fn the_user_is_not_asked_when_every_app_is_on_the_always_quit_list() {
        let scan = scan_with(&[("com.firefox", "Firefox.app"), ("com.editor", "Editor.app")]);
        let lists = AppLists::new().with_always_quit(["com.firefox", "Editor"]);

        let plan = quit_plan(&scan, &lists, false);

        assert!(!plan.ask);
        assert_eq!(plan.always.len(), 2);
    }

    #[test]
    fn the_user_is_asked_when_only_some_apps_are_on_the_always_quit_list() {
        let scan = scan_with(&[("com.firefox", "Firefox.app"), ("com.editor", "Editor.app")]);
        let lists = AppLists::new().with_always_quit(["com.firefox"]);

        let plan = quit_plan(&scan, &lists, false);

        assert!(plan.ask);
        assert_eq!(plan.always.len(), 1);
        assert_eq!(plan.always[0].bundle_id(), "com.firefox");
    }

    #[test]
    fn assuming_yes_never_asks_the_user() {
        let scan = scan_with(&[("com.firefox", "Firefox.app")]);

        let plan = quit_plan(&scan, &AppLists::new(), true);

        assert!(!plan.ask);
    }

    /// The output of a `brew doctor` that found two problems.
    const DOCTOR_OUTPUT: &str = "Please note that these warnings are just used to help the Homebrew maintainers\n\
                                  with debugging if you file an issue. If everything you use Homebrew for\n\
                                  is working fine: please don't worry or file an issue; just ignore this. Thanks!\n\
                                  \n\
                                  Warning: Your Homebrew is outdated. Run `brew update` to update it.\n\
                                  You can solve this by running:\n\
                                  \x20 brew update\n\
                                  Warning: /opt/homebrew/bin is not in your PATH.\n";

    #[test]
    fn parses_the_warnings_of_brew_doctor() {
        let findings = parse_doctor_findings(DOCTOR_OUTPUT, "Your system is ready to brew.\n");

        assert_eq!(findings.len(), 2);
        assert_eq!(
            findings[0].message,
            "Your Homebrew is outdated. Run `brew update` to update it."
        );
        assert_eq!(
            findings[0].remedy.as_deref(),
            Some("You can solve this by running: brew update")
        );
        assert_eq!(
            findings[1].message,
            "/opt/homebrew/bin is not in your PATH."
        );
        assert_eq!(findings[1].remedy, None);
    }

    #[test]
    fn the_notice_before_the_first_warning_is_not_a_finding() {
        let findings = parse_doctor_findings(DOCTOR_OUTPUT, "");

        assert!(
            findings
                .iter()
                .all(|finding| !finding.message.contains("Please note")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn a_healthy_brew_doctor_has_no_findings() {
        let findings = parse_doctor_findings("", "Your system is ready to brew.\n");

        assert!(findings.is_empty());
    }

    #[test]
    fn the_notice_is_skipped_even_without_the_empty_line_below_it() {
        let output =
            "Please note that these warnings are just used to help the Homebrew maintainers\n\
                      Warning: /opt/homebrew/bin is not in your PATH.\n";

        let findings = parse_doctor_findings(output, "");

        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "/opt/homebrew/bin is not in your PATH."
        );
    }

    #[test]
    fn the_findings_of_brew_doctor_become_check_items() {
        let findings = parse_doctor_findings(DOCTOR_OUTPUT, "");
        let report = CheckReport::from_items(findings.iter().map(DoctorFinding::item));

        assert_eq!(report.pending(), 2);
        assert_eq!(
            report.items()[0].to_string(),
            "Your Homebrew is outdated. Run `brew update` to update it. \
             (You can solve this by running: brew update)"
        );
    }

    #[test]
    fn parses_the_packages_brew_bundle_cleanup_would_remove() {
        let output = "Would uninstall casks:\n\
                      google-chrome\n\
                      Would uninstall formulae:\n\
                      wget\n\
                      htop\n\
                      Run `brew bundle cleanup --force` to make these changes.\n";

        let packages = parse_bundle_cleanup(output);

        assert_eq!(packages.len(), 3);
        assert_eq!(packages[0].name, "google-chrome");
        assert_eq!(packages[0].kind, "cask");
        assert_eq!(packages[1].name, "wget");
        assert_eq!(packages[1].kind, "formula");
        assert_eq!(packages[2].name, "htop");
        assert_eq!(packages[2].kind, "formula");
    }

    #[test]
    fn the_packages_of_brew_bundle_cleanup_become_check_items() {
        let packages = parse_bundle_cleanup("Would uninstall formulae:\nwget\n");
        let report = CheckReport::from_items(packages.iter().map(BundlePackage::item));

        assert_eq!(report.pending(), 1);
        assert_eq!(
            report.items()[0].to_string(),
            "wget (formula not in the Brewfile)"
        );
    }

    #[test]
    fn an_empty_brew_bundle_cleanup_lists_no_package() {
        assert!(parse_bundle_cleanup("").is_empty());
        assert!(
            parse_bundle_cleanup("Run `brew bundle cleanup --force` to make these changes.\n")
                .is_empty()
        );
    }

    #[test]
    fn the_brewfile_is_read_from_the_working_directory() {
        let directory = std::env::temp_dir().join(format!(
            "freshen-brewfile-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", directory.display()));
        std::fs::write(directory.join("Brewfile"), "brew \"wget\"\n")
            .unwrap_or_else(|error| panic!("failed to write the Brewfile: {error}"));

        let path = brewfile_in(&directory, None);

        assert_eq!(path, Some(directory.join("Brewfile")));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_brewfile_is_not_a_brewfile() {
        let directory = std::env::temp_dir().join(format!(
            "freshen-no-brewfile-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", directory.display()));

        let path = brewfile_in(&directory, None);

        assert_eq!(path, None);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_brewfile_can_be_pointed_at_with_an_environment_variable() {
        let directory = std::env::temp_dir().join(format!(
            "freshen-brewfile-env-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", directory.display()));
        let file = directory.join("Brewfile.work");
        std::fs::write(&file, "brew \"wget\"\n")
            .unwrap_or_else(|error| panic!("failed to write the Brewfile: {error}"));

        let path = brewfile_in(&directory, Some(file.clone()));

        assert_eq!(path, Some(file));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_bundle_cleanup_step_is_skipped_without_a_brewfile() {
        assert_eq!(
            BundleCleanupStep::detection(true, false),
            Detection::Unavailable(SkipReason::MissingConfiguration)
        );
        assert_eq!(
            BundleCleanupStep::detection(true, true),
            Detection::Installed
        );
        assert_eq!(
            BundleCleanupStep::detection(false, true),
            Detection::NotInstalled
        );
    }
    #[cfg(unix)]
    fn executable_brew(prefix: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let program = prefix.join("bin").join("brew");
        std::fs::create_dir_all(program.parent().unwrap_or(prefix))
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", program.display()));
        std::fs::write(&program, "#!/bin/sh\nexit 0\n")
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", program.display()));
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap_or_else(
            |error| panic!("failed to make {} executable: {error}", program.display()),
        );
        program
    }

    #[cfg(unix)]
    fn discovery_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "freshen-brew-installs-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", root.display()));
        root
    }

    #[cfg(unix)]
    #[test]
    fn discovers_both_architectures_and_puts_the_native_one_first() {
        let root = discovery_root("dual");
        let arm = root.join("opt/homebrew");
        let intel = root.join("usr/local");
        let arm_program = executable_brew(&arm);
        let intel_program = executable_brew(&intel);

        let installs = brew_installs_from(
            Some(intel_program.clone()),
            &[arm.clone(), intel.clone()],
            &Arch::Aarch64,
        );

        assert_eq!(installs.len(), 2, "installs: {installs:?}");
        assert_eq!(installs[0].program, arm_program);
        assert_eq!(installs[0].arch, Arch::Aarch64);
        assert!(!installs[0].from_path);
        assert_eq!(installs[1].program, intel_program);
        assert_eq!(installs[1].arch, Arch::X86_64);
        assert!(installs[1].from_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn an_installation_on_path_is_not_added_twice() {
        let root = discovery_root("dedupe");
        let intel = root.join("usr/local");
        let program = executable_brew(&intel);

        let installs = brew_installs_from(
            Some(program.clone()),
            std::slice::from_ref(&intel),
            &Arch::X86_64,
        );

        assert_eq!(installs.len(), 1, "installs: {installs:?}");
        assert_eq!(installs[0].program, program);
        assert!(installs[0].from_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_standard_prefix_is_found_without_being_on_path() {
        let root = discovery_root("prefix-only");
        let arm = root.join("opt/homebrew");
        let program = executable_brew(&arm);

        let installs = brew_installs_from(None, std::slice::from_ref(&arm), &Arch::Aarch64);

        assert_eq!(installs.len(), 1, "installs: {installs:?}");
        assert_eq!(installs[0].program, program);
        assert!(!installs[0].from_path);
        assert_eq!(
            installs[0].snapshot_program(),
            program.display().to_string()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_standard_prefixes_map_to_their_architecture() {
        assert_eq!(
            install_arch(Path::new("/opt/homebrew"), &Arch::X86_64),
            Arch::Aarch64
        );
        assert_eq!(
            install_arch(Path::new("/usr/local"), &Arch::Aarch64),
            Arch::X86_64
        );
        assert_eq!(
            install_arch(Path::new("/custom/brew"), &Arch::Aarch64),
            Arch::Aarch64
        );
    }

    #[test]
    fn the_cask_metadata_probe_parses_all_recorded_fixtures() {
        assert!(
            super::cask_metadata_probe().is_ok(),
            "the fixtures should parse without parser drift"
        );
    }

    #[test]
    fn the_cask_metadata_probe_fails_on_parser_drift() {
        // A fixture that does not match the parser's expected format should
        // cause the probe to return an error.
        assert!(
            super::cask_metadata("not valid json").is_err(),
            "invalid JSON should trigger parser drift"
        );
    }
}
