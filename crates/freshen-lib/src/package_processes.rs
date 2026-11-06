//! Stopping running processes whose executables live under the install
//! location of an outdated package before upgrading it.
//!
//! On Windows, winget, Chocolatey and Scoop install applications to package
//! directories. If an application is still running from its old install
//! location when freshen upgrades the package, files may be locked or replaced
//! while in use. Before upgrading a package, freshen enumerates the running
//! processes, finds the ones whose executable lives under the package's
//! install location, and asks them to quit (gracefully) and then force-kills
//! them when the policy allows.
//!
//! All operating-system access goes through [`ProcessSystem`], so the matching
//! and stop policy are tested with a fake process table instead of the real
//! machine.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crate::process::CommandSpec;

#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

/// A running process, identified by its pid and the path of its executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    /// Operating-system process id.
    pub pid: u32,
    /// Full path of the executable the process is running from.
    pub executable: PathBuf,
}

/// An outdated package together with the directory its executable lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLocation {
    /// Display name of the package.
    pub name: String,
    /// Directory that contains the package's executable, e.g.
    /// `C:\Program Files\Git` or `~/scoop/apps/git/current`.
    pub install_location: PathBuf,
}

/// How long to wait for a graceful stop and whether a stuck process may be
/// killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopPolicy {
    /// How long to wait for the process to exit after the graceful quit.
    timeout: Duration,
    /// How long to wait between "is it still running?" checks.
    poll_interval: Duration,
    /// Whether a process that does not quit may be force-killed.
    force_kill: bool,
    /// How long to wait after a force kill before giving up.
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

    /// Reads `DESKTOP_STOP_TIMEOUT` (seconds), falling back to 20 seconds.
    #[must_use]
    pub fn from_env(force_kill: bool) -> Self {
        let timeout = std::env::var("DESKTOP_STOP_TIMEOUT")
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

    /// Returns `true` when a process that does not stop may be force-killed.
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

/// The operating-system operations for enumerating and stopping processes.
///
/// The real implementation is [`HostProcessSystem`]. Tests provide a fake
/// process table and record the quit/kill actions instead of touching the
/// machine.
pub trait ProcessSystem: Send + Sync {
    /// A snapshot of the running processes.
    fn processes(&self) -> Vec<Process>;

    /// Asks the process with `pid` to quit gracefully. Returns `true` when
    /// the request was delivered.
    fn quit(&self, pid: u32) -> bool;

    /// Force-kills the process with `pid`. Returns `true` when the kill was
    /// delivered.
    fn kill(&self, pid: u32) -> bool;

    /// Returns `true` when `pid` is still running.
    fn is_alive(&self, pid: u32) -> bool;

    /// The pid of the running freshen process, so it is never stopped.
    fn current_pid(&self) -> u32;
}

/// Returns the running processes whose executable lives under any of the
/// install locations.
///
/// A process matches when its executable path starts with an install location
/// directory. The freshen process itself is never included.
#[must_use]
pub fn processes_under_locations(
    system: &dyn ProcessSystem,
    locations: &[PackageLocation],
) -> Vec<Process> {
    let current_pid = system.current_pid();
    system
        .processes()
        .into_iter()
        .filter(|process| process.pid != current_pid)
        .filter(|process| {
            locations
                .iter()
                .any(|location| process.executable.starts_with(&location.install_location))
        })
        .collect()
}

/// Stops every running process whose executable lives under any of the
/// install locations.
///
/// Each process is asked to quit gracefully and polled until it exits or the
/// policy's timeout elapses. When `force_kill` is set and the process is
/// still running, it is force-killed.
///
/// Returns the processes that were stopped and those that are still running
/// after the stop attempt.
#[must_use]
pub fn stop_processes_under_locations(
    system: &dyn ProcessSystem,
    locations: &[PackageLocation],
    policy: &StopPolicy,
) -> (Vec<Process>, Vec<Process>) {
    let processes = processes_under_locations(system, locations);
    let mut stopped = Vec::new();
    let mut still_running = Vec::new();

    for process in &processes {
        let _ = system.quit(process.pid);

        let mut waited = Duration::ZERO;
        let mut running = system.is_alive(process.pid);
        while running && waited < policy.timeout() {
            thread::sleep(policy.poll_interval());
            waited += policy.poll_interval();
            running = system.is_alive(process.pid);
        }

        if !running {
            stopped.push(process.clone());
            continue;
        }

        if policy.force_kill() {
            let _ = system.kill(process.pid);
            thread::sleep(policy.force_kill_grace());
            running = system.is_alive(process.pid);
        }

        if running {
            still_running.push(process.clone());
        } else {
            stopped.push(process.clone());
        }
    }

    (stopped, still_running)
}

/// Extracts the executable path from a process command line.
///
/// On Windows the command line may quote the executable path (for example
/// `"C:\Program Files\App\app.exe" --arg`), in which case the quoted part is
/// the executable. Otherwise the first whitespace-delimited token is the
/// executable. The `.exe` suffix is preserved so install-location matching
/// works on Windows.
fn executable_from_command(command: &str) -> Option<PathBuf> {
    let command = command.trim_start();
    if command.is_empty() {
        return None;
    }
    let executable = if let Some(quoted) = command.strip_prefix('"') {
        quoted.split('"').next().unwrap_or("")
    } else {
        command.split_whitespace().next().unwrap_or("")
    };
    let executable = executable.trim();
    if executable.is_empty() {
        return None;
    }
    Some(PathBuf::from(executable))
}

/// The real host freshen is running on.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostProcessSystem;

impl ProcessSystem for HostProcessSystem {
    fn processes(&self) -> Vec<Process> {
        #[cfg(unix)]
        {
            unix_processes()
        }
        #[cfg(windows)]
        {
            windows_processes()
        }
        #[cfg(not(any(unix, windows)))]
        {
            Vec::new()
        }
    }

    fn quit(&self, pid: u32) -> bool {
        #[cfg(unix)]
        {
            signal::kill(Pid::from_raw(pid as i32), Some(Signal::SIGTERM)).is_ok()
        }
        #[cfg(windows)]
        {
            CommandSpec::new("taskkill")
                .arg("/PID")
                .arg(pid.to_string())
                .run()
                .is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = pid;
            false
        }
    }

    fn kill(&self, pid: u32) -> bool {
        #[cfg(unix)]
        {
            signal::kill(Pid::from_raw(pid as i32), Some(Signal::SIGKILL)).is_ok()
        }
        #[cfg(windows)]
        {
            CommandSpec::new("taskkill")
                .arg("/PID")
                .arg(pid.to_string())
                .run()
                .is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = pid;
            false
        }
    }

    fn is_alive(&self, pid: u32) -> bool {
        #[cfg(unix)]
        {
            signal::kill(Pid::from_raw(pid as i32), None::<Signal>).is_ok()
        }
        #[cfg(windows)]
        {
            CommandSpec::new("taskkill")
                .arg("/PID")
                .arg(pid.to_string())
                .run()
                .is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = pid;
            false
        }
    }

    fn current_pid(&self) -> u32 {
        std::process::id()
    }
}

#[cfg(unix)]
fn unix_processes() -> Vec<Process> {
    let Ok(output) = CommandSpec::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .neutral_locale()
        .run()
    else {
        return Vec::new();
    };
    parse_process_table(output.stdout())
}

#[cfg(unix)]
fn parse_process_table(output: &str) -> Vec<Process> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(3, char::is_whitespace);
            let pid_str = parts.next()?;
            let _ppid = parts.next()?;
            let command = parts.next()?.trim();
            let pid: u32 = pid_str.parse().ok()?;
            let executable = executable_from_command(command)?;
            Some(Process { pid, executable })
        })
        .collect()
}

#[cfg(windows)]
fn windows_processes() -> Vec<Process> {
    let script = "Get-CimInstance Win32_Process | ForEach-Object { \
                  \"$($_.ProcessId)$([char]31)$($_.ParentProcessId)$([char]31)$($_.CommandLine)\" }";
    let powershell =
        crate::shell::windows::powershell().map_or("powershell", |shell| shell.command());
    let Ok(output) = CommandSpec::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .neutral_locale()
        .run()
    else {
        return Vec::new();
    };
    parse_windows_process_table(output.stdout())
}

#[cfg(windows)]
fn parse_windows_process_table(output: &str) -> Vec<Process> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut fields = line.split(char::from_u32(31)?);
            let pid_str = fields.next()?;
            let _ppid = fields.next()?;
            let command = fields.next()?;
            let pid: u32 = pid_str.parse().ok()?;
            let executable = executable_from_command(command)?;
            Some(Process { pid, executable })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        executable_from_command, processes_under_locations, stop_processes_under_locations,
        PackageLocation, Process, ProcessSystem, StopPolicy,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// A fake process table for testing the stopping policy.
    struct FakeProcessSystem {
        processes: Vec<Process>,
        alive: Mutex<HashMap<u32, bool>>,
        current_pid: u32,
        signals: Arc<Mutex<Vec<(u32, bool)>>>,
    }

    impl FakeProcessSystem {
        fn new() -> Self {
            Self {
                processes: Vec::new(),
                alive: Mutex::new(HashMap::new()),
                current_pid: 999,
                signals: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_process(mut self, pid: u32, executable: impl Into<PathBuf>) -> Self {
            self.processes.push(Process {
                pid,
                executable: executable.into(),
            });
            self.alive
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(pid, true);
            self
        }

        fn with_current_pid(mut self, pid: u32) -> Self {
            self.current_pid = pid;
            self
        }
    }

    impl ProcessSystem for FakeProcessSystem {
        fn processes(&self) -> Vec<Process> {
            self.processes.clone()
        }

        fn quit(&self, pid: u32) -> bool {
            self.signals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((pid, false));
            true
        }

        fn kill(&self, pid: u32) -> bool {
            self.signals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((pid, true));
            let mut alive = self
                .alive
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            alive.insert(pid, false);
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

    fn location(name: &str, path: &str) -> PackageLocation {
        PackageLocation {
            name: name.to_owned(),
            install_location: PathBuf::from(path),
        }
    }

    #[test]
    fn finds_processes_under_install_locations() {
        let system = FakeProcessSystem::new()
            .with_process(100, "/Program Files/Git/bin/git.exe")
            .with_process(200, "/Program Files/Git/cmd/git.exe")
            .with_process(300, "/Users/Alice/scoop/apps/curl/current/bin/curl.exe")
            .with_process(400, "/Windows/System32/notepad.exe");
        let locations = vec![
            location("git", "/Program Files/Git"),
            location("curl", "/Users/Alice/scoop/apps/curl/current"),
        ];

        let found = processes_under_locations(&system, &locations);

        assert_eq!(found.len(), 3);
        assert!(found.iter().any(|p| p.pid == 100));
        assert!(found.iter().any(|p| p.pid == 200));
        assert!(found.iter().any(|p| p.pid == 300));
        assert!(!found.iter().any(|p| p.pid == 400));
    }

    #[test]
    fn ignores_the_freshen_process_itself() {
        let system = FakeProcessSystem::new()
            .with_process(999, "/Program Files/Git/bin/git.exe")
            .with_process(100, "/Program Files/Git/bin/git.exe");
        let locations = vec![location("git", "/Program Files/Git")];

        let found = processes_under_locations(&system, &locations);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, 100);
    }

    #[test]
    fn ignores_processes_outside_install_locations() {
        let system = FakeProcessSystem::new().with_process(100, "/Windows/notepad.exe");
        let locations = vec![location("git", "/Program Files/Git")];

        let found = processes_under_locations(&system, &locations);
        assert!(found.is_empty());
    }

    #[test]
    fn stops_processes_with_graceful_quit_and_waits() {
        let system = FakeProcessSystem::new().with_process(100, "/Program Files/Git/bin/git.exe");
        let locations = vec![location("git", "/Program Files/Git")];
        let policy = StopPolicy::new(Duration::from_secs(1), true);

        let (stopped, still_running) = stop_processes_under_locations(&system, &locations, &policy);

        assert_eq!(stopped.len(), 1);
        assert_eq!(stopped[0].pid, 100);
        assert!(still_running.is_empty());
        assert_eq!(
            system
                .signals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            [(100, false), (100, true)],
            "quit then force-kill when quit does not stop the process"
        );
    }

    #[test]
    fn force_kills_process_that_does_not_quit() {
        let system = FakeProcessSystem::new().with_process(100, "/Program Files/Git/bin/git.exe");
        let locations = vec![location("git", "/Program Files/Git")];
        let policy = StopPolicy::new(Duration::from_millis(10), true)
            .with_poll_interval(Duration::from_millis(1));

        let (stopped, still_running) = stop_processes_under_locations(&system, &locations, &policy);

        assert_eq!(stopped.len(), 1);
        assert!(still_running.is_empty());
        assert_eq!(
            system
                .signals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            [(100, false), (100, true)]
        );
    }

    #[test]
    fn keeps_process_running_when_force_kill_is_disabled() {
        let system = FakeProcessSystem::new().with_process(100, "/Program Files/Git/bin/git.exe");
        let locations = vec![location("git", "/Program Files/Git")];
        let policy = StopPolicy::new(Duration::from_millis(10), false)
            .with_poll_interval(Duration::from_millis(1));

        let (stopped, still_running) = stop_processes_under_locations(&system, &locations, &policy);

        assert!(stopped.is_empty());
        assert_eq!(still_running.len(), 1);
        assert_eq!(
            system
                .signals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            [(100, false)]
        );
    }

    #[test]
    fn never_stops_the_freshen_process() {
        let system = FakeProcessSystem::new()
            .with_current_pid(999)
            .with_process(999, "/Program Files/Git/bin/git.exe");
        let locations = vec![location("git", "/Program Files/Git")];
        let policy = StopPolicy::new(Duration::from_millis(10), true)
            .with_poll_interval(Duration::from_millis(1));

        let (_stopped, still_running) =
            stop_processes_under_locations(&system, &locations, &policy);

        assert!(system
            .signals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .is_empty());
        assert!(
            still_running.is_empty(),
            "freshen process must not appear in results"
        );
    }

    #[test]
    fn no_locations_means_no_processes_stopped() {
        let system = FakeProcessSystem::new().with_process(100, "/Program Files/Git/bin/git.exe");
        let locations: Vec<PackageLocation> = Vec::new();
        let policy = StopPolicy::new(Duration::from_secs(1), true);

        let (stopped, still_running) = stop_processes_under_locations(&system, &locations, &policy);

        assert!(stopped.is_empty());
        assert!(
            still_running.is_empty(),
            "no processes matched when no locations"
        );
    }

    #[test]
    fn extracts_executable_from_quoted_windows_command_line() {
        let executable =
            match executable_from_command(r#""C:\Program Files\Git\bin\git.exe" --some-arg"#) {
                Some(executable) => executable,
                None => panic!("quoted command line should parse"),
            };
        assert_eq!(
            executable,
            PathBuf::from(r"C:\Program Files\Git\bin\git.exe")
        );
    }

    #[test]
    fn extracts_executable_from_unquoted_command_line() {
        let executable = match executable_from_command("/usr/bin/firefox --safe-mode") {
            Some(executable) => executable,
            None => panic!("unquoted command line should parse"),
        };
        assert_eq!(executable, PathBuf::from("/usr/bin/firefox"));
    }

    #[test]
    fn ignores_empty_command_lines() {
        assert!(executable_from_command("").is_none());
        assert!(executable_from_command("   ").is_none());
    }

    #[test]
    fn stop_policy_from_env_defaults_to_20_seconds() {
        let policy = StopPolicy::from_env(false);
        assert_eq!(policy.timeout(), Duration::from_secs(20));
        assert!(!policy.force_kill());
    }

    #[test]
    fn stop_policy_respects_force_kill() {
        let policy = StopPolicy::new(Duration::from_secs(5), true);
        assert!(policy.force_kill());
        assert_eq!(policy.force_kill_grace(), Duration::from_secs(2));
    }
}
