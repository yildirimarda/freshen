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

use std::cell::RefCell;
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::Value;

use crate::apps::{
    find_running_apps, quit_apps, quit_choice, reopen_apps, AppSystem, MacAppSystem, QuitChoice,
    QuitPolicy,
};
use crate::cleanup;
use crate::options::RunOptions;
use crate::platform::Platform;
use crate::process::{CommandSpec, ProcessOutput, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::snapshot::{self, SnapshotSection};
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};
use crate::warnings::warn_about_breakable_sessions;

pub(crate) const BREW: StepId = "brew";
pub(crate) const FORMULAE: StepId = "brew-formulae";
const CASKS: StepId = "brew-casks";
const AUTOREMOVE: StepId = "brew-autoremove";
const CLEANUP: StepId = "brew-cleanup";
pub(crate) const REOPEN: StepId = "brew-reopen";

const HOMEBREW_PLATFORMS: [Platform; 2] = [Platform::Macos, Platform::Linux];

/// Bundle ids of the apps that were quit and still need to be reopened.
#[derive(Clone, Default)]
struct ReopenQueue(Rc<RefCell<Vec<String>>>);

impl ReopenQueue {
    fn extend(&self, bundle_ids: impl IntoIterator<Item = String>) {
        self.0.borrow_mut().extend(bundle_ids);
    }

    fn bundle_ids(&self) -> Vec<String> {
        self.0.borrow().clone()
    }

    fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

fn reopen_queued(queue: &ReopenQueue, system: &dyn AppSystem, message: &str, interrupted: bool) {
    let bundle_ids = queue.bundle_ids();
    if bundle_ids.is_empty() {
        return;
    }

    println!("{message}");
    reopen_apps(system, &bundle_ids);
    if !interrupted {
        queue.clear();
    }
}

/// Registers every Homebrew step, in the order the zsh script runs them.
///
/// Prerequisites make the ordering explicit: formulae after the update, casks
/// after formulae, autoremove and cleanup last, and the quit apps are reopened
/// only after cleanup.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    let reopen = ReopenQueue::default();
    registry
        .with(UpdateStep)
        .with(FormulaeStep)
        .with(CasksStep::new(reopen.clone()))
        .with(AutoremoveStep)
        .with(CleanupStep)
        .with(ReopenStep::new(reopen))
}

/// A failure while reading Homebrew cask metadata.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaskError {
    /// `brew info --cask --json=v2` did not return valid JSON.
    #[error("invalid Homebrew cask JSON: {0}")]
    Json(String),
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
    let value: Value =
        serde_json::from_str(json).map_err(|error| CaskError::Json(error.to_string()))?;
    let Some(casks) = value.get("casks").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut apps = Vec::new();
    for cask in casks {
        let Some(artifacts) = cask.get("artifacts").and_then(Value::as_array) else {
            continue;
        };
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
    }

    Ok(apps)
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

fn detect_brew() -> Detection {
    let Some(path) = env::var_os("PATH") else {
        return Detection::NotInstalled;
    };
    if env::split_paths(&path).any(|directory| is_executable(&directory.join("brew"))) {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
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

fn run_brew(step: StepId, spec: CommandSpec) -> Result<ProcessOutput, StepError> {
    spec.retry_transient_network(NETWORK_RETRIES)
        .run()
        .map_err(|error| StepError::run_from_process(step, &error))
}

fn lines(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn outdated(step: StepId, args: &[&str]) -> Result<CheckReport, StepError> {
    let output = CommandSpec::new("brew")
        .args(["outdated"])
        .args(args)
        .retry_transient_network(NETWORK_RETRIES)
        .run()
        .map_err(|error| StepError::check_from_process(step, &error))?;
    Ok(CheckReport::from_items(
        lines(output.stdout())
            .iter()
            .map(|line| parse_outdated(line)),
    ))
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

fn report_cask_apps(apps: &[String]) {
    if apps.is_empty() {
        return;
    }

    println!("Cask apps: {}", apps.join(", "));

    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    let installed = installed_app_paths(apps, &roots);
    for path in installed {
        println!("Installed cask app: {}", path.display());
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
        run_brew(BREW, CommandSpec::new("brew").arg("update")).map(|_| ())
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(snapshot::homebrew_section())
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

struct FormulaeStep;

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
        outdated(FORMULAE, &["--formula"])
    }

    fn run(&self) -> Result<(), StepError> {
        let outdated = match CommandSpec::new("brew")
            .args(["outdated", "--formula", "--quiet"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            Ok(outdated) => {
                warn_about_breakable_sessions(outdated.stdout());
                Some(lines(outdated.stdout()))
            }
            Err(error) => {
                eprintln!("freshen: could not list outdated formulae: {error}");
                None
            }
        };

        if outdated.is_some_and(|formulae| formulae.is_empty()) {
            println!("No outdated formulae.");
            return Ok(());
        }

        run_brew(
            FORMULAE,
            CommandSpec::new("brew").args(["upgrade", "--formula"]),
        )
        .map(|_| ())
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

struct CasksStep {
    reopen: ReopenQueue,
    system: Rc<dyn AppSystem>,
}

impl CasksStep {
    fn new(reopen: ReopenQueue) -> Self {
        Self {
            reopen,
            system: Rc::new(MacAppSystem),
        }
    }

    #[cfg(test)]
    fn with_system(reopen: ReopenQueue, system: Rc<dyn AppSystem>) -> Self {
        Self { reopen, system }
    }

    fn quit_running_apps(&self, apps: &[String], options: &RunOptions) -> QuitChoice {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        let installed = installed_app_paths(apps, &roots);
        let self_bundle_id = env::var("__CFBundleIdentifier")
            .ok()
            .filter(|bundle_id| !bundle_id.is_empty());
        let system = self.system.as_ref();
        let scan = find_running_apps(system, &installed, self_bundle_id.as_deref());

        for name in scan.protected() {
            println!(
                "warning: {name} is the terminal running this script, not quitting it (restart it manually after the upgrade)."
            );
        }

        if scan.is_empty() {
            return QuitChoice::Quit;
        }

        println!("Running apps that will be quit and reopened:");
        for app in scan.running() {
            println!("   - {}", app.name());
        }

        let choice = if options.assume_yes() {
            QuitChoice::Quit
        } else {
            ask_quit_choice()
        };

        if choice != QuitChoice::Quit {
            return choice;
        }

        let policy = QuitPolicy::from_env(options.force_kill());
        let reopen_ids = quit_apps(system, scan.running(), &policy);
        self.reopen.extend(reopen_ids);
        QuitChoice::Quit
    }
}

fn ask_quit_choice() -> QuitChoice {
    print!("Quit these apps and upgrade casks? [Y]es / [s]kip quitting / [n]o: ");
    let _ = io::stdout().flush();

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
        outdated(CASKS, &["--cask", "--greedy"])
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let outdated = run_brew(
            CASKS,
            CommandSpec::new("brew").args(["outdated", "--cask", "--greedy", "--quiet"]),
        )?;
        let casks = lines(outdated.stdout());

        if casks.is_empty() {
            println!("No outdated casks.");
            return Ok(());
        }

        println!("Casks to upgrade: {}", casks.join(" "));

        let mut info = CommandSpec::new("brew")
            .args(["info", "--cask", "--json=v2"])
            .retry_transient_network(NETWORK_RETRIES);
        for cask in &casks {
            info = info.arg(cask);
        }
        let mut choice = QuitChoice::Quit;
        match info.run() {
            Ok(output) => match cask_app_artifacts(output.stdout()) {
                Ok(apps) => {
                    report_cask_apps(&apps);
                    choice = self.quit_running_apps(&apps, options);
                }
                Err(error) => eprintln!("freshen: could not read cask metadata: {error}"),
            },
            Err(error) => eprintln!("freshen: could not inspect casks: {error}"),
        }

        if choice == QuitChoice::SkipCasks {
            println!("warning: Cask upgrade skipped by user.");
            return Ok(());
        }

        run_brew(
            CASKS,
            CommandSpec::new("brew").args(["upgrade", "--cask", "--greedy"]),
        )
        .map(|_| ())
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
        cleanup::run(
            AUTOREMOVE,
            "Homebrew autoremove",
            CommandSpec::new("brew")
                .arg("autoremove")
                .retry_transient_network(NETWORK_RETRIES),
            options,
        )
        .map(|_| ())
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
        cleanup::run(
            CLEANUP,
            "Homebrew cleanup",
            CommandSpec::new("brew")
                .args(["cleanup", "--prune=all"])
                .retry_transient_network(NETWORK_RETRIES),
            options,
        )
        .map(|_| ())
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

struct ReopenStep {
    reopen: ReopenQueue,
    system: Rc<dyn AppSystem>,
}

impl ReopenStep {
    fn new(reopen: ReopenQueue) -> Self {
        Self {
            reopen,
            system: Rc::new(MacAppSystem),
        }
    }

    #[cfg(test)]
    fn with_system(reopen: ReopenQueue, system: Rc<dyn AppSystem>) -> Self {
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
    use super::{
        cask_app_artifacts, installed_app_paths, parse_outdated, register, CaskError, CasksStep,
        ReopenQueue, ReopenStep, CASKS, FORMULAE, REOPEN,
    };
    use crate::apps::AppSystem;
    use crate::platform::Platform;
    use crate::registry::Registry;
    use crate::step::{Step, StepId};
    use std::cell::RefCell;
    use std::path::Path;
    use std::rc::Rc;

    #[derive(Default)]
    struct FakeAppSystem {
        opened: RefCell<Vec<String>>,
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

        fn quit(&self, _bundle_id: &str) {}

        fn force_kill(&self, _pids: &[u32]) {}

        fn open(&self, bundle_id: &str) -> bool {
            self.opened.borrow_mut().push(bundle_id.to_owned());
            true
        }
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
    fn registry_runs_formulae_before_casks_and_cleanup_last() {
        assert_eq!(
            ordered_ids(Platform::Macos),
            [
                "brew",
                "brew-formulae",
                "brew-casks",
                "brew-autoremove",
                "brew-cleanup",
                "brew-reopen",
            ]
        );
        assert_eq!(
            ordered_ids(Platform::Linux),
            [
                "brew",
                "brew-formulae",
                "brew-casks",
                "brew-autoremove",
                "brew-cleanup",
                "brew-reopen",
            ]
        );
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

    #[test]
    fn an_interrupted_cask_step_reopens_the_apps_it_quit() {
        let queue = ReopenQueue::default();
        queue.extend(["com.firefox".to_owned(), "com.editor".to_owned()]);
        let system = Rc::new(FakeAppSystem::default());
        let step = CasksStep::with_system(queue.clone(), system.clone());

        step.restore()
            .unwrap_or_else(|error| panic!("restore failed: {error}"));

        assert_eq!(
            system.opened.borrow().as_slice(),
            ["com.firefox", "com.editor"]
        );
        assert!(queue.bundle_ids().is_empty());
    }

    #[test]
    fn the_reopen_step_restores_apps_after_an_interrupt() {
        let queue = ReopenQueue::default();
        queue.extend(["com.firefox".to_owned()]);
        let system = Rc::new(FakeAppSystem::default());
        let step = ReopenStep::with_system(queue.clone(), system.clone());

        step.restore()
            .unwrap_or_else(|error| panic!("restore failed: {error}"));

        assert_eq!(system.opened.borrow().as_slice(), ["com.firefox"]);
        assert!(queue.bundle_ids().is_empty());
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
}
