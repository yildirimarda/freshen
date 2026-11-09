//! The Google Cloud CLI step: `gcloud components update --quiet` upgrades the
//! gcloud components when gcloud is installed outside a system package
//! manager.
//!
//! The step runs `gcloud components update --quiet` whenever `gcloud` is
//! available on `PATH` and is not owned by a system package manager
//! (Homebrew, apt/dpkg, dnf, pacman, winget, Chocolatey or Scoop). Those
//! managers keep their own gcloud packages current, so freshen leaves them
//! alone. It is skipped entirely when gcloud is absent, so it is safe to
//! register on every machine.
//!
//! gcloud manages user-level components, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Operation, Step, StepError, StepId};
use std::path::Path;
use std::sync::Arc;

/// The Google Cloud CLI step.
pub const GCLOUD: StepId = "gcloud";

/// gcloud runs on every platform freshen supports.
const GCLOUD_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the Google Cloud CLI step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(GcloudStep::new())
}

/// One invocation of `gcloud` and everything it printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcloudOutput {
    success: bool,
    text: String,
}

impl GcloudOutput {
    /// A successful `gcloud` invocation.
    #[must_use]
    fn success(text: impl Into<String>) -> Self {
        Self {
            success: true,
            text: text.into(),
        }
    }

    /// A failed `gcloud` invocation.
    #[must_use]
    fn failure(text: impl Into<String>) -> Self {
        Self {
            success: false,
            text: text.into(),
        }
    }

    /// Whether the command finished successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Everything the command printed, on stdout and on stderr.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// The `gcloud` commands the Google Cloud CLI step needs.
///
/// The trait exists so the step can be tested against recorded transcripts
/// without the real `gcloud` binary.
pub trait GcloudClient: Send + Sync {
    /// The output of `gcloud components update --quiet`.
    fn update(&self) -> GcloudOutput;
}

/// The real `gcloud` command line.
struct GcloudCommandClient;

impl GcloudClient for GcloudCommandClient {
    fn update(&self) -> GcloudOutput {
        run(CommandSpec::new("gcloud")
            .args(["components", "update", "--quiet"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES))
    }
}

/// Runs a `gcloud` command and collects everything it printed.
fn run(command: CommandSpec) -> GcloudOutput {
    match command.run() {
        Ok(output) => GcloudOutput::success(combined(output.stdout(), output.stderr())),
        Err(error) => {
            let printed = error
                .output()
                .map(|output| combined(output.stdout(), output.stderr()))
                .unwrap_or_default();
            if printed.trim().is_empty() {
                GcloudOutput::failure(error.to_string())
            } else {
                GcloudOutput::failure(printed)
            }
        }
    }
}

/// Joins what a command printed on stdout and on stderr.
fn combined(stdout: &str, stderr: &str) -> String {
    let mut text = stdout.to_owned();
    if !stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(stderr);
    }
    text
}

/// The Google Cloud CLI step.
struct GcloudStep {
    client: Arc<dyn GcloudClient>,
}

impl GcloudStep {
    fn new() -> Self {
        Self {
            client: Arc::new(GcloudCommandClient),
        }
    }

    #[cfg(test)]
    fn with_client(client: Arc<dyn GcloudClient>) -> Self {
        Self { client }
    }

    /// Upgrades the gcloud components.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("gcloud: components update --quiet");
        let output = self.client.update();
        if !output.is_success() {
            failures.push(StepError::new(
                GCLOUD,
                Operation::Run,
                format!(
                    "`gcloud components update --quiet` failed: {}",
                    output.text().trim()
                ),
            ));
        }
    }
}

impl Step for GcloudStep {
    fn id(&self) -> StepId {
        GCLOUD
    }

    fn display_name(&self) -> &str {
        "gcloud"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_gcloud())
    }

    /// gcloud has no read-only "what is outdated" query, so report up to date.
    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &GCLOUD_PLATFORMS
    }

    /// gcloud manages user-level components, so it must never run as root.
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

/// Whether gcloud is installed outside a system package manager.
fn detect_gcloud() -> Detection {
    let Some(gcloud_path) = crate::executable::resolve("gcloud") else {
        return Detection::NotInstalled;
    };
    if gcloud_installed_with_system_package_manager(&gcloud_path) {
        return Detection::NotInstalled;
    }
    Detection::Installed
}

/// Whether gcloud is owned by a system package manager, in which case that
/// manager's own upgrade keeps gcloud current and `gcloud components update`
/// is skipped so the two do not fight over the installation.
fn gcloud_installed_with_system_package_manager(gcloud_path: &Path) -> bool {
    gcloud_installed_with_brew(gcloud_path)
        || gcloud_installed_with_dpkg()
        || gcloud_installed_with_dnf()
        || gcloud_installed_with_pacman()
        || gcloud_installed_with_winget()
        || gcloud_installed_with_choco()
        || gcloud_installed_with_scoop()
}

/// Whether Homebrew owns the gcloud installation (the `gcloud` binary lives
/// inside a Homebrew prefix).
fn gcloud_installed_with_brew(gcloud_path: &Path) -> bool {
    crate::homebrew::brew_installs()
        .iter()
        .any(|install| gcloud_path.starts_with(&install.prefix))
}

/// Whether dpkg (and therefore apt on Debian/Ubuntu) owns the gcloud
/// installation (`dpkg -s google-cloud-cli`).
fn gcloud_installed_with_dpkg() -> bool {
    command_exists("dpkg")
        && CommandSpec::new("dpkg")
            .args(["-s", "google-cloud-cli"])
            .run()
            .is_ok()
}

/// Whether dnf owns the gcloud installation.
fn gcloud_installed_with_dnf() -> bool {
    command_exists("dnf")
        && CommandSpec::new("dnf")
            .args(["list", "installed", "google-cloud-cli"])
            .run()
            .is_ok()
}

/// Whether pacman owns the gcloud installation.
fn gcloud_installed_with_pacman() -> bool {
    command_exists("pacman")
        && CommandSpec::new("pacman")
            .args(["-Q", "google-cloud-sdk"])
            .run()
            .is_ok()
}

/// Whether winget owns the gcloud installation.
fn gcloud_installed_with_winget() -> bool {
    command_exists("winget")
        && CommandSpec::new("winget")
            .args(["list", "Google.CloudSDK"])
            .run()
            .is_ok()
}

/// Whether Chocolatey owns the gcloud installation.
fn gcloud_installed_with_choco() -> bool {
    command_exists("choco")
        && CommandSpec::new("choco")
            .args(["list", "--local-only", "googlecloudsdk"])
            .run()
            .is_ok()
}

/// Whether Scoop owns the gcloud installation.
fn gcloud_installed_with_scoop() -> bool {
    command_exists("scoop")
        && CommandSpec::new("scoop")
            .args(["list", "google-cloud-sdk"])
            .run()
            .is_ok()
}

fn command_exists(program: &str) -> bool {
    crate::executable::command_exists(program)
}

#[cfg(test)]
mod tests {
    use super::{
        detect_gcloud, gcloud_installed_with_system_package_manager, GcloudClient, GcloudOutput,
        GcloudStep, GCLOUD,
    };
    use crate::options::RunOptions;
    use crate::step::{Detection, Operation, Step};
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    /// A `gcloud` that replays recorded transcripts.
    struct FakeGcloud {
        update: GcloudOutput,
        calls: Mutex<Vec<String>>,
    }

    impl FakeGcloud {
        fn new(update: GcloudOutput) -> Self {
            Self {
                update,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn ok() -> Self {
            Self::new(GcloudOutput::success(
                "Updated component: core\nUpdated component: gcloud-cli\n",
            ))
        }

        fn failed() -> Self {
            Self::new(GcloudOutput::failure(
                "ERROR: (gcloud.components.update) Some components could not be updated\n",
            ))
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .map(|calls| calls.clone())
                .unwrap_or_default()
        }

        fn record(&self, call: &str) {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(call.to_owned());
            }
        }
    }

    impl GcloudClient for FakeGcloud {
        fn update(&self) -> GcloudOutput {
            self.record("components update --quiet");
            self.update.clone()
        }
    }

    fn step_with(client: Arc<FakeGcloud>) -> GcloudStep {
        GcloudStep::with_client(client)
    }

    #[test]
    fn check_reports_up_to_date() {
        let step = step_with(Arc::new(FakeGcloud::ok()));
        assert!(step
            .check()
            .unwrap_or_else(|error| panic!("check failed: {error}"))
            .is_up_to_date());
    }

    #[test]
    fn run_updates_gcloud_components() {
        let client = Arc::new(FakeGcloud::ok());
        let step = step_with(client.clone());
        assert!(step.run_with_options(&RunOptions::new()).is_ok());
        assert_eq!(client.calls(), ["components update --quiet"]);
    }

    #[test]
    fn run_reports_a_failed_component_update() {
        let client = Arc::new(FakeGcloud::failed());
        let step = step_with(client);
        let error = match step.run_with_options(&RunOptions::new()) {
            Err(error) => error,
            Ok(()) => panic!("expected a failed component update"),
        };
        assert_eq!(error.step(), GCLOUD);
        assert_eq!(error.operation(), Operation::Run);
    }

    #[test]
    fn is_user_level() {
        assert!(GcloudStep::new().is_user_level());
    }

    /// A stub directory on `PATH` whose commands always exit 0 and log their
    /// invocation to `$FRESHEN_GCLOUD_LOG`.
    fn stub_dir() -> (PathBuf, PathBuf) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = env::temp_dir().join(format!("freshen-gcloud-{}-{}", std::process::id(), id));
        fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        let log = dir.join("calls.log");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = "#!/bin/sh\n\
                 {\n\
                   printf '%s' \"$0\"\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 } >> \"$FRESHEN_GCLOUD_LOG\"\n\
                 exit 0\n";
            let stub = dir.join("gcloud");
            fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            fs::set_permissions(&stub, fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n>>\"%FRESHEN_GCLOUD_LOG%\" echo %0 %*\r\nexit /b 0\r\n";
            let stub = dir.join("gcloud.cmd");
            fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        (dir, log)
    }

    /// The `PATH` list separator for the current platform.
    const PATH_LIST_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

    /// Runs `f` with `gcloud` and the given package managers on `PATH`,
    /// restoring `PATH` afterwards.
    fn with_path_guard(pms: &[&str], f: impl FnOnce()) {
        let previous = env::var("PATH").ok();
        let mut dirs: Vec<PathBuf> = Vec::new();
        for pm in pms {
            let (dir, _log) = stub_dir();
            write_stub(pm, &dir);
            dirs.push(dir);
        }
        let (gcloud_dir, _gcloud_log) = stub_dir();
        write_stub("gcloud", &gcloud_dir);
        dirs.push(gcloud_dir);

        let mut joined = String::new();
        for dir in &dirs {
            if !joined.is_empty() {
                joined.push(PATH_LIST_SEPARATOR);
            }
            joined.push_str(
                dir.to_str()
                    .unwrap_or_else(|| panic!("non-UTF-8 stub path: {}", dir.display())),
            );
        }
        if let Some(previous) = &previous {
            joined.push(PATH_LIST_SEPARATOR);
            joined.push_str(previous);
        }
        env::set_var("PATH", &joined);
        env::set_var("FRESHEN_GCLOUD_LOG", "");

        let result = f;

        env::remove_var("FRESHEN_GCLOUD_LOG");
        match previous {
            Some(previous) => env::set_var("PATH", &previous),
            None => env::remove_var("PATH"),
        }

        for dir in &dirs {
            let _ = fs::remove_dir_all(dir);
        }
        let _ = result;
    }

    fn write_stub(pm: &str, dir: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = "#!/bin/sh\nexit 0\n";
            let stub = dir.join(pm);
            fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            fs::set_permissions(&stub, fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\nexit /b 0\r\n";
            let stub = dir.join(format!("{pm}.cmd"));
            fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }
    }

    #[test]
    fn detect_reports_not_installed_when_gcloud_is_absent() {
        let previous = env::var("PATH").ok();
        env::set_var("PATH", "");
        let result = detect_gcloud();
        match previous {
            Some(previous) => env::set_var("PATH", &previous),
            None => env::remove_var("PATH"),
        }
        assert_eq!(result, Detection::NotInstalled);
    }

    #[test]
    fn detect_reports_installed_when_gcloud_is_not_system_managed() {
        with_path_guard(&[], || {
            assert_eq!(detect_gcloud(), Detection::Installed);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_brew_owns_gcloud() {
        // Put the stub gcloud inside a fake Homebrew prefix, next to a stub
        // `bin/brew` so Homebrew reports itself as installed at that prefix.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let previous = env::var("PATH").ok();
        let dir =
            env::temp_dir().join(format!("freshen-gcloud-brew-{}-{}", std::process::id(), id));
        fs::create_dir_all(dir.join("bin")).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        write_stub("gcloud", dir.join("bin").as_path());
        write_stub("brew", dir.join("bin").as_path());
        env::set_var(
            "FRESHEN_BREW_PREFIXES",
            dir.to_str()
                .unwrap_or_else(|| panic!("non-UTF-8 stub path: {}", dir.display())),
        );
        env::set_var(
            "PATH",
            dir.join("bin")
                .to_str()
                .unwrap_or_else(|| panic!("non-UTF-8 stub path: {}", dir.display())),
        );

        let result = detect_gcloud();

        env::remove_var("FRESHEN_BREW_PREFIXES");
        match previous {
            Some(previous) => env::set_var("PATH", &previous),
            None => env::remove_var("PATH"),
        }
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(result, Detection::NotInstalled);
    }

    #[test]
    fn detect_reports_not_installed_when_dpkg_owns_gcloud() {
        with_path_guard(&["dpkg"], || {
            assert_eq!(detect_gcloud(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_dnf_owns_gcloud() {
        with_path_guard(&["dnf"], || {
            assert_eq!(detect_gcloud(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_pacman_owns_gcloud() {
        with_path_guard(&["pacman"], || {
            assert_eq!(detect_gcloud(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_winget_owns_gcloud() {
        with_path_guard(&["winget"], || {
            assert_eq!(detect_gcloud(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_choco_owns_gcloud() {
        with_path_guard(&["choco"], || {
            assert_eq!(detect_gcloud(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_scoop_owns_gcloud() {
        with_path_guard(&["scoop"], || {
            assert_eq!(detect_gcloud(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_still_reports_installed_when_an_unrelated_package_manager_is_present() {
        with_path_guard(&["apt"], || {
            assert_eq!(detect_gcloud(), Detection::Installed);
        });
    }

    #[test]
    fn gcloud_installed_with_system_package_manager_probes_each_manager() {
        // No package manager is present, so nothing is owned.
        assert!(!gcloud_installed_with_system_package_manager(Path::new(
            "/tmp/gcloud"
        )));
    }
}
