//! Homebrew steps: update, formulae, casks, autoremove and cleanup.
//!
//! The zsh implementation runs one long Homebrew block. Splitting it into
//! steps keeps the behaviour (formulae before casks, so apps stay open during
//! the slow formula upgrade) while letting the runner record a failure per
//! command instead of aborting the rest of the block.
//!
//! Casks are inspected with `brew info --cask --json=v2` before the upgrade so
//! the app artifacts a cask will replace are known. Only the artifact names are
//! collected here; quitting and reopening the running apps is a separate step.

use std::env;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::platform::Platform;
use crate::process::{CommandSpec, ProcessOutput};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

const BREW: StepId = "brew";
const FORMULAE: StepId = "brew-formulae";
const CASKS: StepId = "brew-casks";
const AUTOREMOVE: StepId = "brew-autoremove";
const CLEANUP: StepId = "brew-cleanup";

const HOMEBREW_PLATFORMS: [Platform; 2] = [Platform::Macos, Platform::Linux];

/// Registers every Homebrew step, in the order the zsh script runs them.
///
/// Prerequisites make the ordering explicit: formulae after the update, casks
/// after formulae, autoremove and cleanup last.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry
        .with(UpdateStep)
        .with(FormulaeStep)
        .with(CasksStep)
        .with(AutoremoveStep)
        .with(CleanupStep)
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
    spec.run()
        .map_err(|error| StepError::run(step, error.to_string()))
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
    let output = run_brew(step, CommandSpec::new("brew").args(["outdated"]).args(args))?;
    Ok(CheckReport::from_items(
        lines(output.stdout()).into_iter().map(OutdatedItem::new),
    ))
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
        outdated(FORMULAE, &["--formula", "--quiet"])
    }

    fn run(&self) -> Result<(), StepError> {
        run_brew(
            FORMULAE,
            CommandSpec::new("brew").args(["upgrade", "--formula"]),
        )
        .map(|_| ())
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[BREW]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

struct CasksStep;

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
        outdated(CASKS, &["--cask", "--greedy", "--quiet"])
    }

    fn run(&self) -> Result<(), StepError> {
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

        let mut info = CommandSpec::new("brew").args(["info", "--cask", "--json=v2"]);
        for cask in &casks {
            info = info.arg(cask);
        }
        match info.run() {
            Ok(output) => match cask_app_artifacts(output.stdout()) {
                Ok(apps) => report_cask_apps(&apps),
                Err(error) => eprintln!("freshen: could not read cask metadata: {error}"),
            },
            Err(error) => eprintln!("freshen: could not inspect casks: {error}"),
        }

        run_brew(
            CASKS,
            CommandSpec::new("brew").args(["upgrade", "--cask", "--greedy"]),
        )
        .map(|_| ())
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
        run_brew(AUTOREMOVE, CommandSpec::new("brew").arg("autoremove")).map(|_| ())
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
        run_brew(
            CLEANUP,
            CommandSpec::new("brew").args(["cleanup", "--prune=all"]),
        )
        .map(|_| ())
    }

    fn prerequisites(&self) -> &'static [StepId] {
        &[AUTOREMOVE]
    }

    fn platforms(&self) -> &'static [Platform] {
        &HOMEBREW_PLATFORMS
    }
}

#[cfg(test)]
mod tests {
    use super::{cask_app_artifacts, installed_app_paths, register, CaskError, CASKS, FORMULAE};
    use crate::platform::Platform;
    use crate::registry::Registry;
    use crate::step::StepId;

    fn ordered_ids(platform: Platform) -> Vec<StepId> {
        register(Registry::new())
            .ordered_for(platform)
            .unwrap_or_else(|error| panic!("registry order failed: {error}"))
            .iter()
            .map(|step| step.id())
            .collect()
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
