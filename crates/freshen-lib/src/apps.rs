//! Quitting and reopening macOS applications around a cask upgrade.
//!
//! The zsh implementation reads the bundle id of every installed cask app,
//! quits the ones that are running, waits for them to exit and reopens exactly
//! those apps afterwards. It never quits the terminal that is running the
//! script, and only force-kills an app when the user asked for it.
//!
//! All operating-system access goes through [`AppSystem`], so the policy is
//! tested with a fake process table instead of the real machine.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::output::human;
use crate::process::CommandSpec;

/// Seconds `osascript` waits for an app before the quit command gives up.
const APPLESCRIPT_TIMEOUT_SECS: u64 = 3;

/// Default seconds to wait for an app to quit, matching `QUIT_TIMEOUT`.
const DEFAULT_QUIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Default time between checks while waiting for an app to quit.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Default wait after a force kill, matching the zsh script.
const DEFAULT_FORCE_KILL_GRACE: Duration = Duration::from_secs(2);

/// An installed application bundle that is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningApp {
    path: PathBuf,
    bundle_id: String,
    name: String,
}

impl RunningApp {
    /// Creates a running app from its bundle path and bundle identifier.
    #[must_use]
    pub fn new(path: PathBuf, bundle_id: impl Into<String>) -> Self {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        Self {
            path,
            bundle_id: bundle_id.into(),
            name,
        }
    }

    /// Path of the `.app` bundle.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bundle identifier used for the AppleScript quit and the reopen.
    #[must_use]
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    /// File name shown in the "Quitting:" line, e.g. `Firefox.app`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The result of inspecting the apps a cask upgrade will replace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppScan {
    running: Vec<RunningApp>,
    protected: Vec<String>,
}

impl AppScan {
    /// Installed apps that are currently running and may be quit.
    #[must_use]
    pub fn running(&self) -> &[RunningApp] {
        &self.running
    }

    /// Installed apps that were not quit because they are the host terminal.
    #[must_use]
    pub fn protected(&self) -> &[String] {
        &self.protected
    }

    /// Returns `true` when there is nothing running to quit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }
}

/// How long to wait for a graceful quit and whether a stuck app may be killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuitPolicy {
    timeout: Duration,
    poll_interval: Duration,
    force_kill_grace: Duration,
    force_kill: bool,
}

impl QuitPolicy {
    /// Creates a policy with the default poll interval and force-kill grace.
    #[must_use]
    pub fn new(timeout: Duration, force_kill: bool) -> Self {
        Self {
            timeout,
            poll_interval: DEFAULT_POLL_INTERVAL,
            force_kill_grace: DEFAULT_FORCE_KILL_GRACE,
            force_kill,
        }
    }

    /// Reads `QUIT_TIMEOUT` (seconds) like the zsh script, falling back to 20.
    #[must_use]
    pub fn from_env(force_kill: bool) -> Self {
        let timeout = env::var("QUIT_TIMEOUT")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_QUIT_TIMEOUT);
        Self::new(timeout, force_kill)
    }

    /// Replaces the interval between two "is it still running?" checks.
    #[must_use]
    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = if poll_interval.is_zero() {
            Duration::from_millis(1)
        } else {
            poll_interval
        };
        self
    }

    /// Replaces the wait after a force kill.
    #[must_use]
    pub fn with_force_kill_grace(mut self, grace: Duration) -> Self {
        self.force_kill_grace = grace;
        self
    }

    /// How long to wait for a graceful quit.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Returns `true` when an app that does not quit may be killed.
    #[must_use]
    pub fn force_kill(&self) -> bool {
        self.force_kill
    }
}

/// The answer to the "quit these apps?" prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitChoice {
    /// Quit the apps and upgrade the casks.
    Quit,
    /// Upgrade the casks without quitting anything.
    SkipQuit,
    /// Skip the cask upgrade entirely.
    SkipCasks,
}

/// Parses a prompt answer with the same choices as the zsh script.
///
/// An empty answer means yes, as pressing Return does in `update-all.sh`.
#[must_use]
pub fn quit_choice(answer: &str) -> QuitChoice {
    match answer.trim().to_ascii_lowercase().as_str() {
        "" | "y" | "yes" => QuitChoice::Quit,
        "s" | "skip" => QuitChoice::SkipQuit,
        _ => QuitChoice::SkipCasks,
    }
}

/// The operating-system operations the quit/reopen policy needs.
///
/// The real implementation is [`MacAppSystem`]. Tests provide a fake process
/// table and record the quit, kill and open calls.
pub trait AppSystem: Send + Sync {
    /// Reads `CFBundleIdentifier` from the bundle's `Contents/Info`.
    fn bundle_id(&self, app: &Path) -> Option<String>;

    /// Returns `true` when the bundle's main executable is running.
    fn is_running(&self, app: &Path) -> bool;

    /// Pids of processes running the bundle's main executable.
    fn pids(&self, app: &Path) -> Vec<u32>;

    /// Asks the application to quit gracefully.
    fn quit(&self, bundle_id: &str);

    /// Force-kills the given pids.
    fn force_kill(&self, pids: &[u32]);

    /// Reopens the application with `open -g -b`.
    fn open(&self, bundle_id: &str) -> bool;
}

/// Finds the running apps among `installed` bundles.
///
/// Apps without a bundle id are ignored, and an app whose bundle id matches
/// `self_bundle_id` is reported in [`AppScan::protected`] instead of being
/// considered for quitting.
#[must_use]
pub fn find_running_apps<S: AppSystem + ?Sized>(
    system: &S,
    installed: &[PathBuf],
    self_bundle_id: Option<&str>,
) -> AppScan {
    let mut scan = AppScan::default();

    for path in installed {
        let Some(bundle_id) = system.bundle_id(path) else {
            continue;
        };
        if self_bundle_id.is_some_and(|self_id| self_id == bundle_id) {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            scan.protected.push(name);
            continue;
        }
        if system.is_running(path) {
            scan.running.push(RunningApp::new(path.clone(), bundle_id));
        }
    }

    scan
}

/// Quits every app gracefully and returns the bundle ids to reopen.
///
/// A bundle id is recorded before the quit is attempted, exactly like the zsh
/// script, so an app that refuses to quit is still reopened (it never closed).
#[must_use]
pub fn quit_apps<S: AppSystem + ?Sized>(
    system: &S,
    apps: &[RunningApp],
    policy: &QuitPolicy,
) -> Vec<String> {
    let mut reopen = Vec::with_capacity(apps.len());

    for app in apps {
        human!("Quitting: {}", app.name);
        reopen.push(app.bundle_id.clone());
        quit_one(system, app, policy);
    }

    reopen
}

/// Reopens the apps with the given bundle ids, warning about failures.
pub fn reopen_apps<S: AppSystem + ?Sized>(system: &S, bundle_ids: &[String]) {
    for bundle_id in bundle_ids {
        human!("Opening: {bundle_id}");
        if !system.open(bundle_id) {
            warn(&format!("Could not open: {bundle_id}"));
        }
    }
}

fn quit_one<S: AppSystem + ?Sized>(system: &S, app: &RunningApp, policy: &QuitPolicy) -> bool {
    system.quit(app.bundle_id());

    let mut running = system.is_running(app.path());
    let mut waited = Duration::ZERO;
    while running && waited < policy.timeout {
        std::thread::sleep(policy.poll_interval);
        waited += policy.poll_interval;
        running = system.is_running(app.path());
    }

    if !running {
        return true;
    }

    if policy.force_kill {
        warn(&format!("{} did not quit, force-killing", app.name));
        let pids = system.pids(app.path());
        if !pids.is_empty() {
            system.force_kill(&pids);
            std::thread::sleep(policy.force_kill_grace);
        }
        return true;
    }

    warn(&format!(
        "{} did not quit (use --force-kill to force). Upgrade will be attempted anyway.",
        app.name
    ));
    false
}

fn warn(message: &str) {
    human!("warning: {message}");
}

fn executable_prefix(app: &Path) -> String {
    format!("{}/Contents/MacOS/", app.display())
}

/// Pids whose command line runs `app`'s main executable.
///
/// Helper and agent processes that share the bundle id but live in another
/// executable are not matched, which is why the zsh script checks
/// `Contents/MacOS/` instead of AppleScript's "is running".
#[must_use]
pub fn pids_from_processes(processes: &str, app: &Path) -> Vec<u32> {
    let needle = executable_prefix(app);
    processes
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let mut fields = line.splitn(2, char::is_whitespace);
            let pid = fields.next()?.parse::<u32>().ok()?;
            let command = fields.next().unwrap_or_default();
            command.contains(&needle).then_some(pid)
        })
        .collect()
}

fn bundle_id_command(app: &Path) -> CommandSpec {
    CommandSpec::new("defaults")
        .arg("read")
        .arg(app.join("Contents/Info"))
        .arg("CFBundleIdentifier")
}

fn process_list_command() -> CommandSpec {
    CommandSpec::new("ps").args(["-axo", "command="])
}

fn pid_list_command() -> CommandSpec {
    CommandSpec::new("ps").args(["-axo", "pid=,command="])
}

fn quit_command(bundle_id: &str) -> CommandSpec {
    CommandSpec::new("osascript")
        .arg("-e")
        .arg(format!(
            "with timeout of {APPLESCRIPT_TIMEOUT_SECS} seconds"
        ))
        .arg("-e")
        .arg(format!("tell application id \"{bundle_id}\" to quit"))
        .arg("-e")
        .arg("end timeout")
}

fn kill_command(pids: &[u32]) -> CommandSpec {
    let mut spec = CommandSpec::new("kill").arg("-9");
    for pid in pids {
        spec = spec.arg(pid.to_string());
    }
    spec
}

fn open_command(bundle_id: &str) -> CommandSpec {
    CommandSpec::new("open").args(["-g", "-b"]).arg(bundle_id)
}

/// The real macOS implementation, using the same commands as the zsh script.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacAppSystem;

impl MacAppSystem {
    fn process_list(&self) -> String {
        process_list_command()
            .run()
            .map(|output| output.stdout().to_owned())
            .unwrap_or_default()
    }
}

impl AppSystem for MacAppSystem {
    fn bundle_id(&self, app: &Path) -> Option<String> {
        let output = bundle_id_command(app).run().ok()?;
        let bundle_id = output.stdout().trim();
        if bundle_id.is_empty() {
            None
        } else {
            Some(bundle_id.to_owned())
        }
    }

    fn is_running(&self, app: &Path) -> bool {
        let needle = executable_prefix(app);
        self.process_list().contains(&needle)
    }

    fn pids(&self, app: &Path) -> Vec<u32> {
        let output = pid_list_command().run();
        match output {
            Ok(output) => pids_from_processes(output.stdout(), app),
            Err(_) => Vec::new(),
        }
    }

    fn quit(&self, bundle_id: &str) {
        let _ = quit_command(bundle_id).run();
    }

    fn force_kill(&self, pids: &[u32]) {
        if !pids.is_empty() {
            let _ = kill_command(pids).run();
        }
    }

    fn open(&self, bundle_id: &str) -> bool {
        open_command(bundle_id).run().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};
    use std::time::Duration;

    use super::{
        bundle_id_command, find_running_apps, kill_command, open_command, pid_list_command,
        pids_from_processes, process_list_command, quit_apps, quit_choice, quit_command,
        reopen_apps, AppSystem, QuitChoice, QuitPolicy,
    };

    #[derive(Default)]
    struct FakeSystem {
        bundle_ids: Mutex<HashMap<PathBuf, String>>,
        running: Mutex<HashMap<PathBuf, Vec<u32>>>,
        quit_stops_apps: bool,
        open_succeeds: bool,
        quit_calls: Mutex<Vec<String>>,
        killed: Mutex<Vec<Vec<u32>>>,
        opened: Mutex<Vec<String>>,
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    impl FakeSystem {
        fn new() -> Self {
            Self {
                open_succeeds: true,
                ..Self::default()
            }
        }

        fn with_app(self, app: &str, bundle_id: &str, pids: &[u32]) -> Self {
            let path = PathBuf::from(app);
            lock(&self.bundle_ids).insert(path.clone(), bundle_id.to_owned());
            lock(&self.running).insert(path, pids.to_vec());
            self
        }

        fn with_bundle_id(self, app: &str, bundle_id: &str) -> Self {
            lock(&self.bundle_ids).insert(PathBuf::from(app), bundle_id.to_owned());
            self
        }

        fn open_fails(mut self) -> Self {
            self.open_succeeds = false;
            self
        }
    }

    impl AppSystem for FakeSystem {
        fn bundle_id(&self, app: &std::path::Path) -> Option<String> {
            lock(&self.bundle_ids).get(app).cloned()
        }

        fn is_running(&self, app: &std::path::Path) -> bool {
            lock(&self.running)
                .get(app)
                .is_some_and(|pids| !pids.is_empty())
        }

        fn pids(&self, app: &std::path::Path) -> Vec<u32> {
            lock(&self.running).get(app).cloned().unwrap_or_default()
        }

        fn quit(&self, bundle_id: &str) {
            lock(&self.quit_calls).push(bundle_id.to_owned());
            if self.quit_stops_apps {
                lock(&self.running).clear();
            }
        }

        fn force_kill(&self, pids: &[u32]) {
            lock(&self.killed).push(pids.to_vec());
            lock(&self.running).clear();
        }

        fn open(&self, bundle_id: &str) -> bool {
            lock(&self.opened).push(bundle_id.to_owned());
            self.open_succeeds
        }
    }

    fn installed(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn running(system: &FakeSystem, app: &str) -> bool {
        system.is_running(&PathBuf::from(app))
    }

    fn fast_policy(force_kill: bool) -> QuitPolicy {
        QuitPolicy::new(Duration::ZERO, force_kill)
            .with_poll_interval(Duration::from_millis(1))
            .with_force_kill_grace(Duration::ZERO)
    }

    #[test]
    fn finds_running_apps_and_protects_the_host_terminal() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .with_app("/Applications/Terminal.app", "com.terminal", &[20])
            .with_bundle_id("/Applications/Editor.app", "com.editor");
        let apps = installed(&[
            "/Applications/Firefox.app",
            "/Applications/Terminal.app",
            "/Applications/Editor.app",
        ]);

        let scan = find_running_apps(&system, &apps, Some("com.terminal"));

        assert_eq!(scan.running().len(), 1);
        assert_eq!(scan.running()[0].name(), "Firefox.app");
        assert_eq!(scan.running()[0].bundle_id(), "com.firefox");
        assert_eq!(scan.protected(), ["Terminal.app"]);
        assert!(lock(&system.quit_calls).is_empty());
    }

    #[test]
    fn apps_without_a_bundle_id_are_ignored() {
        let system = FakeSystem::new();
        let mut running_apps = lock(&system.running);
        running_apps.insert(PathBuf::from("/Applications/Unknown.app"), vec![7]);
        drop(running_apps);

        let scan = find_running_apps(&system, &installed(&["/Applications/Unknown.app"]), None);

        assert!(scan.is_empty());
        assert!(scan.protected().is_empty());
    }

    #[test]
    fn apps_that_are_not_running_are_not_quit() {
        let system = FakeSystem::new().with_bundle_id("/Applications/Editor.app", "com.editor");

        let scan = find_running_apps(&system, &installed(&["/Applications/Editor.app"]), None);

        assert!(scan.is_empty());
    }

    #[test]
    fn a_graceful_quit_waits_until_the_app_exits_without_killing() {
        let system = FakeSystem::new().with_app("/Applications/Firefox.app", "com.firefox", &[10]);
        let system = FakeSystem {
            quit_stops_apps: true,
            ..system
        };
        let scan = find_running_apps(&system, &installed(&["/Applications/Firefox.app"]), None);

        let reopen = quit_apps(
            &system,
            scan.running(),
            &QuitPolicy::new(Duration::from_secs(20), false),
        );

        assert_eq!(reopen, ["com.firefox"]);
        assert_eq!(lock(&system.quit_calls).as_slice(), ["com.firefox"]);
        assert!(lock(&system.killed).is_empty());
        assert!(!running(&system, "/Applications/Firefox.app"));
    }

    #[test]
    fn a_timeout_without_force_kill_leaves_the_app_running() {
        let system = FakeSystem::new().with_app("/Applications/Firefox.app", "com.firefox", &[10]);
        let scan = find_running_apps(&system, &installed(&["/Applications/Firefox.app"]), None);

        let reopen = quit_apps(&system, scan.running(), &fast_policy(false));

        assert_eq!(reopen, ["com.firefox"]);
        assert!(lock(&system.killed).is_empty());
        assert!(running(&system, "/Applications/Firefox.app"));
    }

    #[test]
    fn a_timeout_with_force_kill_kills_the_main_executable_pids() {
        let system =
            FakeSystem::new().with_app("/Applications/Firefox.app", "com.firefox", &[10, 11]);
        let scan = find_running_apps(&system, &installed(&["/Applications/Firefox.app"]), None);

        let reopen = quit_apps(&system, scan.running(), &fast_policy(true));

        assert_eq!(reopen, ["com.firefox"]);
        assert_eq!(lock(&system.killed).as_slice(), [vec![10, 11]]);
    }

    #[test]
    fn reopens_only_the_bundle_ids_that_were_quit() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .with_app("/Applications/Terminal.app", "com.terminal", &[20]);
        let system = FakeSystem {
            quit_stops_apps: true,
            ..system
        };
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app", "/Applications/Terminal.app"]),
            Some("com.terminal"),
        );

        let reopen = quit_apps(&system, scan.running(), &fast_policy(false));
        reopen_apps(&system, &reopen);

        assert_eq!(reopen, ["com.firefox"]);
        assert_eq!(lock(&system.opened).as_slice(), ["com.firefox"]);
    }

    #[test]
    fn a_failed_reopen_is_reported_but_does_not_stop_the_others() {
        let system = FakeSystem::new().open_fails();

        reopen_apps(&system, &["com.first".to_owned(), "com.second".to_owned()]);

        assert_eq!(lock(&system.opened).as_slice(), ["com.first", "com.second"]);
    }

    #[test]
    fn prompt_answers_match_the_zsh_script() {
        for answer in ["", "y", "Y", "yes", "YES", " yes "] {
            assert_eq!(quit_choice(answer), QuitChoice::Quit, "answer {answer:?}");
        }
        for answer in ["s", "S", "skip", "SKIP"] {
            assert_eq!(
                quit_choice(answer),
                QuitChoice::SkipQuit,
                "answer {answer:?}"
            );
        }
        for answer in ["n", "no", "anything else"] {
            assert_eq!(
                quit_choice(answer),
                QuitChoice::SkipCasks,
                "answer {answer:?}"
            );
        }
    }

    #[test]
    fn pids_only_match_the_main_executable_not_helpers() {
        let processes = "\
  100 /Applications/Firefox.app/Contents/MacOS/firefox
  101 /Applications/Firefox.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper
  102 /Applications/Firefox.app/Contents/MacOS/firefox --background
  103 /Applications/Other.app/Contents/MacOS/firefox";

        let pids =
            pids_from_processes(processes, std::path::Path::new("/Applications/Firefox.app"));

        assert_eq!(pids, [100, 102]);
    }

    #[test]
    fn command_transcripts_match_the_zsh_script() {
        let app = std::path::Path::new("/Applications/Firefox.app");

        assert_eq!(
            bundle_id_command(app).arguments(),
            [
                "read",
                "/Applications/Firefox.app/Contents/Info",
                "CFBundleIdentifier"
            ]
            .map(std::ffi::OsString::from)
            .to_vec()
        );
        assert_eq!(
            process_list_command().arguments(),
            ["-axo", "command="].map(std::ffi::OsString::from).to_vec()
        );
        assert_eq!(
            pid_list_command().arguments(),
            ["-axo", "pid=,command="]
                .map(std::ffi::OsString::from)
                .to_vec()
        );
        assert_eq!(
            quit_command("com.example.firefox").arguments(),
            [
                "-e",
                "with timeout of 3 seconds",
                "-e",
                "tell application id \"com.example.firefox\" to quit",
                "-e",
                "end timeout",
            ]
            .map(std::ffi::OsString::from)
            .to_vec()
        );
        assert_eq!(
            kill_command(&[10, 11]).arguments(),
            ["-9", "10", "11"].map(std::ffi::OsString::from).to_vec()
        );
        assert_eq!(
            open_command("com.example.firefox").arguments(),
            ["-g", "-b", "com.example.firefox"]
                .map(std::ffi::OsString::from)
                .to_vec()
        );
    }
}
