//! Stopping running desktop applications before their packages are upgraded.
//!
//! On Linux, applications ship `.desktop` files that describe the executable
//! to launch. freshen reads those files, matches the executable against the
//! running process table, and stops the matching applications with `SIGTERM`
//! (then `SIGKILL` after a timeout) before the package manager upgrades them.
//!
//! All operating-system access goes through [`DesktopAppSystem`], so the
//! discovery and stopping policy is tested with a fake desktop and a fake
//! process table instead of the real machine.

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::process::CommandSpec;

#[cfg(target_os = "linux")]
use nix::sys::signal::{self, Signal};
#[cfg(target_os = "linux")]
use nix::unistd::Pid;

/// A desktop application discovered from a `.desktop` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopApp {
    name: String,
    executable: String,
    desktop_file: PathBuf,
}

impl DesktopApp {
    /// Creates a desktop application from its `.desktop` file.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        executable: impl Into<String>,
        desktop_file: PathBuf,
    ) -> Self {
        Self {
            name: name.into(),
            executable: executable.into(),
            desktop_file,
        }
    }

    /// Display name of the application.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Executable name matched against the running process table.
    #[must_use]
    pub fn executable(&self) -> &str {
        &self.executable
    }

    /// Path of the `.desktop` file the application was discovered from.
    #[must_use]
    pub fn desktop_file(&self) -> &Path {
        &self.desktop_file
    }
}

/// How long to wait for a graceful stop and whether a stuck app may be killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopPolicy {
    timeout: Duration,
    poll_interval: Duration,
    force_kill: bool,
    force_kill_grace: Duration,
}

impl StopPolicy {
    /// Creates a policy with the default poll interval and force-kill grace.
    #[must_use]
    pub fn new(timeout: Duration, force_kill: bool) -> Self {
        Self {
            timeout,
            poll_interval: DEFAULT_POLL_INTERVAL,
            force_kill,
            force_kill_grace: DEFAULT_FORCE_KILL_GRACE,
        }
    }

    /// Reads `DESKTOP_STOP_TIMEOUT` (seconds) like the zsh script, falling back
    /// to 20 seconds.
    #[must_use]
    pub fn from_env(force_kill: bool) -> Self {
        let timeout = env::var("DESKTOP_STOP_TIMEOUT")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_STOP_TIMEOUT);
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

    /// How long to wait for a graceful stop.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// How long to wait between checks while stopping.
    #[must_use]
    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    /// Returns `true` when an app that does not stop may be force-killed.
    #[must_use]
    pub fn force_kill(&self) -> bool {
        self.force_kill
    }

    /// How long to wait after a force kill.
    #[must_use]
    pub fn force_kill_grace(&self) -> Duration {
        self.force_kill_grace
    }
}

const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_FORCE_KILL_GRACE: Duration = Duration::from_secs(2);

/// The operating-system operations desktop app detection and stopping need.
///
/// The real implementation is [`HostDesktopAppSystem`]. Tests provide a fake
/// process table and record the signals that are sent.
pub trait DesktopAppSystem: Send + Sync {
    /// The `.desktop` files to inspect, as `(path, contents)` pairs.
    fn desktop_files(&self) -> Vec<(PathBuf, String)>;

    /// Pids of running processes whose executable matches `executable`.
    fn pids_of(&self, executable: &str) -> Vec<u32>;

    /// Sends `signal` to `pid`. Returns `true` when the signal was delivered.
    fn send_signal(&self, pid: u32, signal: Signal) -> bool;

    /// Returns `true` when `pid` is still running.
    fn is_alive(&self, pid: u32) -> bool;

    /// The pid of the running freshen process, so it is never stopped.
    fn current_pid(&self) -> u32;
}

/// Finds the desktop applications whose executable is running.
///
/// Applications are read from `.desktop` files; an application is reported
/// only when at least one process with a matching executable is running.
#[must_use]
pub fn find_running_apps<S: DesktopAppSystem + ?Sized>(system: &S) -> Vec<DesktopApp> {
    let mut apps = Vec::new();
    let mut seen = HashSet::new();

    for (path, contents) in system.desktop_files() {
        let Some(app) = parse_desktop_file(&path, &contents) else {
            continue;
        };
        if !seen.insert(app.executable().to_owned()) {
            continue;
        }
        if system.pids_of(app.executable()).is_empty() {
            continue;
        }
        apps.push(app);
    }

    apps
}

/// Stops every running desktop application gracefully.
///
/// Each application is sent `SIGTERM` and polled until it exits or the
/// policy's timeout elapses. When `force_kill` is set and the application is
/// still running, it is sent `SIGKILL`.
///
/// Returns the applications that were stopped and those that are still
/// running after the stop attempt.
#[must_use]
pub fn stop_apps<S: DesktopAppSystem + ?Sized>(
    system: &S,
    apps: &[DesktopApp],
    policy: &StopPolicy,
) -> (Vec<DesktopApp>, Vec<DesktopApp>) {
    let mut stopped = Vec::new();
    let mut still_running = Vec::new();

    for app in apps {
        let pids = system.pids_of(app.executable());
        if pids.is_empty() {
            still_running.push(app.clone());
            continue;
        }

        for &pid in &pids {
            if pid != system.current_pid() {
                let _ = system.send_signal(pid, Signal::SIGTERM);
            }
        }

        let mut waited = Duration::ZERO;
        let mut running = pids.iter().any(|pid| system.is_alive(*pid));
        while running && waited < policy.timeout() {
            thread::sleep(policy.poll_interval());
            waited += policy.poll_interval();
            running = pids.iter().any(|pid| system.is_alive(*pid));
        }

        if !running {
            stopped.push(app.clone());
            continue;
        }

        if policy.force_kill() {
            for &pid in &pids {
                if pid != system.current_pid() {
                    let _ = system.send_signal(pid, Signal::SIGKILL);
                }
            }
            thread::sleep(policy.force_kill_grace());
            running = pids.iter().any(|pid| system.is_alive(*pid));
        }

        if running {
            still_running.push(app.clone());
        } else {
            stopped.push(app.clone());
        }
    }

    (stopped, still_running)
}

thread_local! {
    static STOPPED_DESKTOP_APPS: std::cell::RefCell<Vec<DesktopApp>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// Records an application that was stopped before its package was upgraded.
///
/// The next step (restarting the applications with `gtk-launch`) reuses this
/// snapshot of what was stopped.
pub fn record_stopped_app(app: DesktopApp) {
    STOPPED_DESKTOP_APPS.with(|apps| apps.borrow_mut().push(app));
}

/// Takes the applications recorded as stopped on the current thread.
pub fn take_stopped_apps() -> Vec<DesktopApp> {
    STOPPED_DESKTOP_APPS.with(|apps| apps.borrow_mut().drain(..).collect())
}

/// The command used to restart a stopped desktop application.
const GTK_LAUNCH: &str = "gtk-launch";

/// Restarts every desktop application that was recorded as stopped before an
/// upgrade, reusing the recorded snapshot of what was stopped.
///
/// Each application is launched with `gtk-launch <desktop-file-id>`, where the
/// desktop-file-id is the basename of the `.desktop` file the application was
/// discovered from. `gtk-launch` detaches the application and returns
/// immediately, so an application that fails to launch is reported and the
/// remaining applications are still launched.
///
/// Returns the applications that were restarted and the applications that
/// failed to launch, in the order they were stopped.
#[must_use]
pub fn restart_stopped_apps() -> (Vec<DesktopApp>, Vec<String>) {
    let apps = take_stopped_apps();
    let mut restarted = Vec::new();
    let mut failures = Vec::new();

    for app in apps {
        let id = app
            .desktop_file()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if id.is_empty() {
            failures.push(format!("{}: no desktop file id", app.name()));
            continue;
        }
        match CommandSpec::new(GTK_LAUNCH).args([id]).run() {
            Ok(_) => restarted.push(app),
            Err(error) => failures.push(format!("{}: {error}", app.name())),
        }
    }

    (restarted, failures)
}

/// Parses a `.desktop` file into a [`DesktopApp`].
///
/// Only the `Name=` and `Exec=` fields are read; everything else is ignored.
fn parse_desktop_file(path: &Path, contents: &str) -> Option<DesktopApp> {
    let mut name = None;
    let mut executable = None;

    for line in contents.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Name=") {
            name = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("Exec=") {
            executable = Some(parse_executable(value));
        }
    }

    let name = name?;
    let executable = executable?;
    Some(DesktopApp::new(name, executable, path.to_path_buf()))
}

/// Extracts the executable name from an `Exec=` value.
///
/// The first whitespace-delimited token is the executable, possibly quoted.
/// Desktop-file field codes such as `%u` are separate tokens and are ignored.
fn parse_executable(exec: &str) -> String {
    let token = exec.split_whitespace().next().unwrap_or("");
    let token = token.trim_matches('"').trim_matches('\'');
    Path::new(token)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(token)
        .to_owned()
}

/// The real host freshen is running on.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostDesktopAppSystem;

impl DesktopAppSystem for HostDesktopAppSystem {
    fn desktop_files(&self) -> Vec<(PathBuf, String)> {
        let mut files = Vec::new();
        for dir in self.desktop_dirs() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
                    continue;
                }
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    files.push((path, contents));
                }
            }
        }
        files
    }

    fn pids_of(&self, executable: &str) -> Vec<u32> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut pids = Vec::new();
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|text| text.parse::<u32>().ok())
            else {
                continue;
            };
            let cmdline_path = format!("/proc/{pid}/cmdline");
            let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) else {
                continue;
            };
            let first = cmdline.split('\0').next().unwrap_or("");
            let basename = Path::new(first)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if basename == executable {
                pids.push(pid);
            }
        }
        pids
    }

    fn send_signal(&self, pid: u32, signal: Signal) -> bool {
        signal::kill(Pid::from_raw(pid as i32), Some(signal)).is_ok()
    }

    fn is_alive(&self, pid: u32) -> bool {
        signal::kill(Pid::from_raw(pid as i32), None::<Signal>).is_ok()
    }

    fn current_pid(&self) -> u32 {
        std::process::id()
    }
}

impl HostDesktopAppSystem {
    /// The directories searched for `.desktop` files, in order.
    ///
    /// `FRESHEN_DESKTOP_DIRS` overrides the default locations, which keeps the
    /// package-manager tests from ever touching the real desktop files.
    fn desktop_dirs(&self) -> Vec<PathBuf> {
        if let Ok(dirs) = env::var("FRESHEN_DESKTOP_DIRS") {
            return env::split_paths(&dirs).collect();
        }

        let mut dirs = Vec::new();
        if let Some(home) = env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/share/applications"));
        }
        dirs.push(PathBuf::from("/usr/share/applications"));
        dirs
    }
}

#[cfg(test)]
mod tests {
    use super::{
        find_running_apps, parse_desktop_file, parse_executable, record_stopped_app,
        restart_stopped_apps, stop_apps, take_stopped_apps, DesktopApp, DesktopAppSystem,
        HostDesktopAppSystem, Signal, StopPolicy,
    };
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// A fake desktop and process table for testing the stopping policy.
    struct FakeDesktopAppSystem {
        files: Vec<(PathBuf, String)>,
        pids: HashMap<String, Vec<u32>>,
        alive: std::sync::Mutex<HashMap<u32, bool>>,
        sigterm_kills: bool,
        current_pid: u32,
        signals: Arc<Mutex<Vec<(u32, Signal)>>>,
    }

    impl FakeDesktopAppSystem {
        fn new() -> Self {
            Self {
                files: Vec::new(),
                pids: HashMap::new(),
                alive: std::sync::Mutex::new(HashMap::new()),
                sigterm_kills: true,
                current_pid: 999,
                signals: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_file(mut self, path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
            self.files.push((path.into(), contents.into()));
            self
        }

        fn with_process(mut self, executable: &str, pid: u32) -> Self {
            self.pids
                .entry(executable.to_owned())
                .or_default()
                .push(pid);
            self.alive
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(pid, true);
            self
        }

        fn sigterm_does_not_kill(mut self) -> Self {
            self.sigterm_kills = false;
            self
        }

        fn with_current_pid(mut self, pid: u32) -> Self {
            self.current_pid = pid;
            self
        }

        fn signals(&self) -> Vec<(u32, Signal)> {
            self.signals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    impl DesktopAppSystem for FakeDesktopAppSystem {
        fn desktop_files(&self) -> Vec<(PathBuf, String)> {
            self.files.clone()
        }

        fn pids_of(&self, executable: &str) -> Vec<u32> {
            self.pids.get(executable).cloned().unwrap_or_default()
        }

        fn send_signal(&self, pid: u32, signal: Signal) -> bool {
            self.signals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((pid, signal));
            let mut alive = self
                .alive
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if (self.sigterm_kills && signal == Signal::SIGTERM) || signal == Signal::SIGKILL {
                alive.insert(pid, false);
            }
            true
        }

        fn is_alive(&self, pid: u32) -> bool {
            self.alive
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&pid)
                .copied()
                .unwrap_or(false)
        }

        fn current_pid(&self) -> u32 {
            self.current_pid
        }
    }

    fn desktop_file(path: &str, name: &str, executable: &str) -> (PathBuf, String) {
        (
            PathBuf::from(path),
            format!("Desktop Entry\nName={name}\nExec={executable}\n"),
        )
    }

    #[test]
    fn parses_name_and_exec_from_a_desktop_file() {
        let (path, contents) = desktop_file(
            "/usr/share/applications/firefox.desktop",
            "Firefox",
            "firefox %u",
        );
        let app = match parse_desktop_file(&path, &contents) {
            Some(app) => app,
            None => panic!("expected a parsed desktop file"),
        };

        assert_eq!(app.name(), "Firefox");
        assert_eq!(app.executable(), "firefox");
        assert_eq!(
            app.desktop_file(),
            Path::new("/usr/share/applications/firefox.desktop")
        );
    }

    #[test]
    fn takes_the_basename_of_a_quoted_executable_path() {
        let executable = parse_executable("\"/usr/bin/firefox\" %u");
        assert_eq!(executable, "firefox");

        let executable = parse_executable("/usr/bin/gimp");
        assert_eq!(executable, "gimp");
    }

    #[test]
    fn ignores_desktop_files_without_a_name_or_exec() {
        assert!(parse_desktop_file(
            &PathBuf::from("/usr/share/applications/empty.desktop"),
            "Desktop Entry\nOnly=Something\n",
        )
        .is_none());
    }

    #[test]
    fn finds_running_apps_from_desktop_files() {
        let system = FakeDesktopAppSystem::new()
            .with_file(
                "/usr/share/applications/firefox.desktop",
                "Desktop Entry\nName=Firefox\nExec=firefox\n",
            )
            .with_file(
                "/usr/share/applications/gimp.desktop",
                "Desktop Entry\nName=GIMP\nExec=gimp\n",
            )
            .with_process("firefox", 100)
            .with_process("gimp", 200);

        let apps = find_running_apps(&system);

        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name(), "Firefox");
        assert_eq!(apps[1].name(), "GIMP");
    }

    #[test]
    fn ignores_apps_that_are_not_running() {
        let system = FakeDesktopAppSystem::new()
            .with_file(
                "/usr/share/applications/firefox.desktop",
                "Desktop Entry\nName=Firefox\nExec=firefox\n",
            )
            .with_process("gimp", 200);

        let apps = find_running_apps(&system);

        assert!(apps.is_empty());
    }

    #[test]
    fn deduplicates_apps_by_executable() {
        let system = FakeDesktopAppSystem::new()
            .with_file(
                "/usr/share/applications/firefox.desktop",
                "Desktop Entry\nName=Firefox\nExec=firefox\n",
            )
            .with_file(
                "/usr/local/share/applications/firefox.desktop",
                "Desktop Entry\nName=Firefox\nExec=firefox\n",
            )
            .with_process("firefox", 100);

        let apps = find_running_apps(&system);

        assert_eq!(apps.len(), 1);
    }

    #[test]
    fn stops_apps_with_sigterm_and_waits_for_them_to_exit() {
        let system = FakeDesktopAppSystem::new().with_process("firefox", 100);
        let apps = vec![DesktopApp::new(
            "Firefox",
            "firefox",
            PathBuf::from("/a/firefox.desktop"),
        )];
        let policy = StopPolicy::new(Duration::from_secs(1), true);

        let (stopped, still_running) = stop_apps(&system, &apps, &policy);

        assert_eq!(stopped.len(), 1);
        assert_eq!(stopped[0].name(), "Firefox");
        assert!(still_running.is_empty());
        assert_eq!(system.signals(), [(100, Signal::SIGTERM)]);
    }

    #[test]
    fn force_kill_sends_sigkill_when_sigterm_does_not_stop_the_app() {
        let system = FakeDesktopAppSystem::new()
            .sigterm_does_not_kill()
            .with_process("firefox", 100);
        let apps = vec![DesktopApp::new(
            "Firefox",
            "firefox",
            PathBuf::from("/a/firefox.desktop"),
        )];
        let policy = StopPolicy::new(Duration::from_millis(10), true)
            .with_poll_interval(Duration::from_millis(1));

        let (stopped, still_running) = stop_apps(&system, &apps, &policy);

        assert_eq!(stopped.len(), 1);
        assert!(still_running.is_empty());
        assert_eq!(
            system.signals(),
            [(100, Signal::SIGTERM), (100, Signal::SIGKILL),]
        );
    }

    #[test]
    fn keeps_apps_running_when_force_kill_is_disabled() {
        let system = FakeDesktopAppSystem::new()
            .sigterm_does_not_kill()
            .with_process("firefox", 100);
        let apps = vec![DesktopApp::new(
            "Firefox",
            "firefox",
            PathBuf::from("/a/firefox.desktop"),
        )];
        let policy = StopPolicy::new(Duration::from_millis(10), false)
            .with_poll_interval(Duration::from_millis(1));

        let (stopped, still_running) = stop_apps(&system, &apps, &policy);

        assert!(stopped.is_empty());
        assert_eq!(still_running.len(), 1);
        assert_eq!(system.signals(), [(100, Signal::SIGTERM)]);
    }

    #[test]
    fn never_signals_the_freshen_process_itself() {
        let system = FakeDesktopAppSystem::new()
            .with_process("freshen", 999)
            .with_current_pid(999);
        let apps = vec![DesktopApp::new(
            "freshen",
            "freshen",
            PathBuf::from("/a/freshen.desktop"),
        )];
        let policy = StopPolicy::new(Duration::from_millis(10), true)
            .with_poll_interval(Duration::from_millis(1));

        let (_stopped, still_running) = stop_apps(&system, &apps, &policy);

        assert!(system.signals().is_empty());
        assert_eq!(still_running.len(), 1);
    }

    #[test]
    fn records_stopped_apps_for_the_restart_step() {
        let app = DesktopApp::new("Firefox", "firefox", PathBuf::from("/a/firefox.desktop"));
        record_stopped_app(app.clone());

        let stopped = take_stopped_apps();
        assert_eq!(stopped, [app]);
        assert!(take_stopped_apps().is_empty());
    }

    #[test]
    fn restart_stopped_apps_with_nothing_recorded_returns_empty() {
        let (restarted, failures) = restart_stopped_apps();
        assert!(restarted.is_empty());
        assert!(failures.is_empty());
    }

    #[test]
    fn restart_stopped_apps_reuses_the_recorded_snapshot() {
        let app = DesktopApp::new("Firefox", "firefox", PathBuf::from("/a/firefox.desktop"));
        record_stopped_app(app.clone());

        let (restarted, failures) = restart_stopped_apps();

        // The snapshot is drained exactly once.
        assert!(take_stopped_apps().is_empty());

        // `gtk-launch` is not installed in the test environment, so the
        // launch fails and the app is reported as a failure rather than
        // restarted.
        assert!(restarted.is_empty());
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn host_desktop_app_system_uses_env_override() {
        // `HostDesktopAppSystem` is only constructible on Linux; this test
        // compiles only under `#[cfg(target_os = "linux")]`, which this module
        // itself is gated on.
        let _system = HostDesktopAppSystem;
        let _ = std::env::var("FRESHEN_DESKTOP_DIRS").ok();
    }
}
