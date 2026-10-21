//! Detecting another package manager that is already changing the machine.
//!
//! Running two package managers at once can corrupt package databases or fail
//! halfway through an upgrade. Before a run changes anything, this module
//! looks for the package managers freshen knows about:
//!
//! * Homebrew, including the `ruby .../brew.rb` process the `brew` wrapper
//!   starts;
//! * apt and dpkg, including a held `/var/lib/dpkg` or `/var/lib/apt` lock;
//! * winget and its elevation server on Windows.
//!
//! Detection goes through [`PackageManagerProbe`], so tests describe a busy
//! machine instead of reading the process table of the machine they run on.
//! [`HostPackageManagerProbe`] reads the real host: `/proc` and `/proc/locks`
//! on Linux, `ps` on macOS and `tasklist` on Windows.
//!
//! [`wait_for_package_managers`] gives a running package manager time to
//! finish. When it is still running after the configured wait, the caller
//! aborts the run with a message naming the process and its PID.

use std::fmt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(any(windows, all(unix, not(target_os = "linux"))))]
use crate::process::CommandSpec;

/// Default time a run waits for another package manager to finish.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(30);

/// Default interval between checks while waiting.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How often the wait looks for an interrupt while it sleeps.
const INTERRUPT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Lock files apt and dpkg take while they change the package database.
#[cfg(target_os = "linux")]
const APT_LOCK_FILES: [&str; 4] = [
    "/var/lib/dpkg/lock-frontend",
    "/var/lib/dpkg/lock",
    "/var/lib/apt/lists/lock",
    "/var/cache/apt/archives/lock",
];

/// One package manager process that is running on the host.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageManagerProcess {
    name: String,
    pid: u32,
}

impl PackageManagerProcess {
    /// Creates a process entry for `name` with process id `pid`.
    #[must_use]
    pub fn new(name: impl Into<String>, pid: u32) -> Self {
        Self {
            name: name.into(),
            pid,
        }
    }

    /// Human-readable name of the package manager or lock.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Process id holding the package manager or lock.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

impl fmt::Display for PackageManagerProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (PID {})", self.name, self.pid)
    }
}

/// Reads the package manager processes of the host.
pub trait PackageManagerProbe {
    /// Package managers that are running right now.
    fn running_package_managers(&self) -> Vec<PackageManagerProcess>;
}

/// The real host implementation of [`PackageManagerProbe`].
#[derive(Debug, Default, Clone, Copy)]
pub struct HostPackageManagerProbe;

impl PackageManagerProbe for HostPackageManagerProbe {
    fn running_package_managers(&self) -> Vec<PackageManagerProcess> {
        #[cfg(target_os = "linux")]
        {
            host_processes()
        }
        #[cfg(all(unix, not(target_os = "linux")))]
        {
            ps_processes()
        }
        #[cfg(windows)]
        {
            tasklist_processes()
        }
        #[cfg(not(any(unix, windows)))]
        {
            Vec::new()
        }
    }
}

/// How long to wait and how often to look while doing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageManagerConfig {
    enabled: bool,
    wait: Duration,
    poll: Duration,
}

impl Default for PackageManagerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            wait: DEFAULT_WAIT,
            poll: DEFAULT_POLL_INTERVAL,
        }
    }
}

impl PackageManagerConfig {
    /// Creates the default configuration: wait up to 30 seconds, checking
    /// every 500 milliseconds.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables or disables the check.
    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets the maximum time to wait for a running package manager.
    #[must_use]
    pub fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// Sets the interval between checks while waiting.
    #[must_use]
    pub fn with_poll(mut self, poll: Duration) -> Self {
        self.poll = poll;
        self
    }

    /// Returns `true` when the check is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Maximum time to wait.
    #[must_use]
    pub fn wait(&self) -> Duration {
        self.wait
    }

    /// Interval between checks while waiting.
    #[must_use]
    pub fn poll(&self) -> Duration {
        self.poll
    }
}

/// What happened while waiting for another package manager to finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitOutcome {
    /// No package manager was running.
    Idle,
    /// A package manager was running and finished within the wait.
    Waited {
        /// How long the run waited.
        waited: Duration,
        /// Processes that were running when the wait started.
        processes: Vec<PackageManagerProcess>,
    },
    /// A package manager was still running when the wait elapsed.
    Busy {
        /// How long the run waited.
        waited: Duration,
        /// Processes that were still running.
        processes: Vec<PackageManagerProcess>,
    },
    /// The user interrupted the wait before it finished.
    Interrupted {
        /// How long the run waited before the interrupt.
        waited: Duration,
        /// Processes that were still running when the wait was interrupted.
        processes: Vec<PackageManagerProcess>,
    },
}

impl WaitOutcome {
    /// Returns `true` for [`WaitOutcome::Busy`].
    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Busy { .. })
    }

    /// How long the run waited, when it had to wait at all.
    #[must_use]
    pub fn waited(&self) -> Option<Duration> {
        match self {
            Self::Idle => None,
            Self::Waited { waited, .. }
            | Self::Busy { waited, .. }
            | Self::Interrupted { waited, .. } => Some(*waited),
        }
    }

    /// Processes seen while waiting, in report order.
    #[must_use]
    pub fn processes(&self) -> &[PackageManagerProcess] {
        match self {
            Self::Idle => &[],
            Self::Waited { processes, .. }
            | Self::Busy { processes, .. }
            | Self::Interrupted { processes, .. } => processes,
        }
    }
}

/// Waits up to the configured time for every package manager to finish.
///
/// A disabled configuration returns [`WaitOutcome::Idle`] without probing the
/// host. With a zero wait the first probe decides immediately. A SIGINT or
/// SIGTERM during the wait stops it promptly and returns
/// [`WaitOutcome::Interrupted`].
#[must_use]
pub fn wait_for_package_managers(
    probe: &dyn PackageManagerProbe,
    config: &PackageManagerConfig,
) -> WaitOutcome {
    wait_for_package_managers_with(probe, config, crate::signals::requested)
}

fn wait_for_package_managers_with<F>(
    probe: &dyn PackageManagerProbe,
    config: &PackageManagerConfig,
    interrupt: F,
) -> WaitOutcome
where
    F: Fn() -> bool,
{
    if !config.is_enabled() {
        return WaitOutcome::Idle;
    }

    let running = normalize(probe.running_package_managers());
    if running.is_empty() {
        return WaitOutcome::Idle;
    }

    let wait = config.wait();
    if wait.is_zero() {
        return WaitOutcome::Busy {
            waited: Duration::ZERO,
            processes: running,
        };
    }

    let started = Instant::now();
    let mut current = running.clone();
    while started.elapsed() < wait {
        let remaining = wait.saturating_sub(started.elapsed());
        let delay = config.poll().min(remaining);
        if !sleep_unless_interrupted(delay, &interrupt) {
            return WaitOutcome::Interrupted {
                waited: started.elapsed(),
                processes: current,
            };
        }

        current = normalize(probe.running_package_managers());
        if current.is_empty() {
            return WaitOutcome::Waited {
                waited: started.elapsed(),
                processes: running,
            };
        }
    }

    WaitOutcome::Busy {
        waited: started.elapsed(),
        processes: current,
    }
}

/// Sleeps for `delay` in short slices so an interrupt is noticed promptly.
///
/// Returns `false` when the interrupt check fired before the delay elapsed.
fn sleep_unless_interrupted<F>(delay: Duration, interrupt: &F) -> bool
where
    F: Fn() -> bool,
{
    let started = Instant::now();

    loop {
        if interrupt() {
            return false;
        }

        let remaining = delay.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }

        thread::sleep(remaining.min(INTERRUPT_POLL_INTERVAL));
    }
}

fn normalize(mut processes: Vec<PackageManagerProcess>) -> Vec<PackageManagerProcess> {
    processes.sort_by(|left, right| {
        left.pid
            .cmp(&right.pid)
            .then_with(|| left.name.cmp(&right.name))
    });
    processes.dedup_by(|left, right| left.pid == right.pid);
    processes
}

/// Maps a command line or executable name to the package manager it runs.
///
/// The Homebrew `brew` command is a shell wrapper that replaces itself with
/// Ruby running `brew.rb`, so both the wrapper and that Ruby process count as
/// Homebrew.
fn classify_command(command: &str) -> Option<&'static str> {
    let program = command.split_whitespace().next()?;
    let base = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    let base = base.strip_suffix(".exe").unwrap_or(&base);

    if matches!(base, "brew" | "brew.rb") {
        return Some("brew");
    }
    if base.starts_with("ruby") && command.to_ascii_lowercase().contains("brew.rb") {
        return Some("brew");
    }

    match base {
        "apt" | "apt-get" | "aptitude" | "unattended-upgrade" | "unattended-upgr" => Some("apt"),
        "dpkg" | "dpkg-deb" | "dpkg-split" => Some("dpkg"),
        "winget" | "windowspackagemanagerserver" | "appinstallercli" => Some("winget"),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn host_processes() -> Vec<PackageManagerProcess> {
    let mut processes = proc_processes();
    processes.extend(apt_lock_processes());
    normalize(processes)
}

#[cfg(target_os = "linux")]
fn proc_processes() -> Vec<PackageManagerProcess> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };

        let directory = entry.path();
        let command = std::fs::read(directory.join("cmdline"))
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).replace('\0', " "))
            .map(|command| command.trim().to_owned())
            .filter(|command| !command.is_empty())
            .or_else(|| {
                std::fs::read_to_string(directory.join("comm"))
                    .ok()
                    .map(|command| command.trim().to_owned())
            });

        if let Some(name) = command.as_deref().and_then(classify_command) {
            found.push(PackageManagerProcess::new(name, pid));
        }
    }

    found
}

#[cfg(target_os = "linux")]
fn apt_lock_processes() -> Vec<PackageManagerProcess> {
    use std::os::unix::fs::MetadataExt;

    let Ok(locks) = std::fs::read_to_string("/proc/locks") else {
        return Vec::new();
    };
    let locks = parse_proc_locks(&locks);

    let mut found = Vec::new();
    for path in APT_LOCK_FILES {
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        let inode = metadata.ino();
        for (pid, locked) in &locks {
            if *locked == inode {
                found.push(PackageManagerProcess::new("dpkg lock", *pid));
            }
        }
    }

    found
}

/// Parses the `pid` and inode of every held lock in `/proc/locks`.
#[cfg(any(test, target_os = "linux"))]
fn parse_proc_locks(table: &str) -> Vec<(u32, u64)> {
    table
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let pid = fields.get(4)?.parse::<u32>().ok()?;
            let (_, inode) = fields.get(5)?.rsplit_once(':')?;
            let inode = inode.parse::<u64>().ok()?;
            Some((pid, inode))
        })
        .collect()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn ps_processes() -> Vec<PackageManagerProcess> {
    let Ok(output) = CommandSpec::new("ps")
        .args(["-axo", "pid=,command="])
        .neutral_locale()
        .run()
    else {
        return Vec::new();
    };

    parse_ps_processes(output.stdout())
}

/// Parses `ps -axo pid=,command=` output.
#[cfg(any(test, all(unix, not(target_os = "linux"))))]
fn parse_ps_processes(table: &str) -> Vec<PackageManagerProcess> {
    let mut found = Vec::new();
    for line in table.lines() {
        let line = line.trim_start();
        let Some(end) = line.find(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = line.get(..end).unwrap_or_default().parse::<u32>() else {
            continue;
        };
        let command = line.get(end..).unwrap_or_default().trim_start();
        if let Some(name) = classify_command(command) {
            found.push(PackageManagerProcess::new(name, pid));
        }
    }

    normalize(found)
}

#[cfg(windows)]
fn tasklist_processes() -> Vec<PackageManagerProcess> {
    let Ok(output) = CommandSpec::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .neutral_locale()
        .run()
    else {
        return Vec::new();
    };

    parse_tasklist(output.stdout())
}

/// Parses `tasklist /FO CSV /NH` output.
#[cfg(any(test, windows))]
fn parse_tasklist(table: &str) -> Vec<PackageManagerProcess> {
    let mut found = Vec::new();
    for line in table.lines() {
        let Some(rest) = line.trim().strip_prefix('"') else {
            continue;
        };
        let Some((name, rest)) = rest.split_once("\",\"") else {
            continue;
        };
        let Some((pid, _)) = rest.split_once("\",\"") else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if let Some(manager) = classify_command(name) {
            found.push(PackageManagerProcess::new(manager, pid));
        }
    }

    normalize(found)
}

#[cfg(test)]
mod tests {
    use super::{
        classify_command, normalize, parse_proc_locks, parse_ps_processes, parse_tasklist,
        wait_for_package_managers, wait_for_package_managers_with, PackageManagerConfig,
        PackageManagerProbe, PackageManagerProcess, WaitOutcome,
    };
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    struct FakeProbe {
        calls: Cell<usize>,
        states: Vec<Vec<PackageManagerProcess>>,
    }

    impl FakeProbe {
        fn new(states: Vec<Vec<PackageManagerProcess>>) -> Self {
            Self {
                calls: Cell::new(0),
                states,
            }
        }
    }

    impl PackageManagerProbe for FakeProbe {
        fn running_package_managers(&self) -> Vec<PackageManagerProcess> {
            let index = self.calls.get().min(self.states.len().saturating_sub(1));
            self.calls.set(self.calls.get() + 1);
            self.states.get(index).cloned().unwrap_or_default()
        }
    }

    fn process(name: &str, pid: u32) -> PackageManagerProcess {
        PackageManagerProcess::new(name, pid)
    }

    #[test]
    fn recognises_known_package_managers() {
        assert_eq!(classify_command("brew update"), Some("brew"));
        assert_eq!(
            classify_command("/opt/homebrew/bin/brew upgrade"),
            Some("brew")
        );
        assert_eq!(
            classify_command("ruby /opt/homebrew/Library/Homebrew/brew.rb upgrade"),
            Some("brew")
        );
        assert_eq!(
            classify_command("/usr/bin/apt-get install ripgrep"),
            Some("apt")
        );
        assert_eq!(
            classify_command("unattended-upgrade --dry-run"),
            Some("apt")
        );
        assert_eq!(
            classify_command("/usr/bin/dpkg --configure -a"),
            Some("dpkg")
        );
        assert_eq!(
            classify_command("dpkg-deb --info package.deb"),
            Some("dpkg")
        );
        assert_eq!(classify_command("winget.exe upgrade --all"), Some("winget"));
        assert_eq!(
            classify_command("WindowsPackageManagerServer.exe"),
            Some("winget")
        );
    }

    #[test]
    fn ignores_unrelated_processes() {
        assert_eq!(classify_command("/usr/bin/vim brew.rb"), None);
        assert_eq!(classify_command("freshen"), None);
        assert_eq!(classify_command("ruby script.rb"), None);
        assert_eq!(classify_command(""), None);
    }

    #[test]
    fn parses_ps_output_including_the_homebrew_ruby_process() {
        let table = "\
              123 /opt/homebrew/bin/brew update\n\
              456 ruby /opt/homebrew/Library/Homebrew/brew.rb upgrade\n\
              789 /usr/bin/vim notes.txt\n\
              321 /usr/bin/apt-get install ripgrep\n";

        assert_eq!(
            parse_ps_processes(table),
            vec![
                process("brew", 123),
                process("apt", 321),
                process("brew", 456)
            ]
        );
    }

    #[test]
    fn parses_tasklist_output() {
        let table = "\
            \"winget.exe\",\"4242\",\"Console\",\"1\",\"12,345 K\"\r\n\
            \"WindowsPackageManagerServer.exe\",\"4343\",\"Services\",\"0\",\"8,000 K\"\r\n\
            \"explorer.exe\",\"1\",\"Console\",\"1\",\"50,000 K\"\r\n";

        assert_eq!(
            parse_tasklist(table),
            vec![process("winget", 4242), process("winget", 4343)]
        );
    }

    #[test]
    fn parses_held_dpkg_and_apt_locks() {
        let table = "\
            1: POSIX  ADVISORY  WRITE 1234 08:01:123456 0 EOF\n\
            2: FLOCK  ADVISORY  WRITE 5678 08:01:789012 0 EOF\n";

        assert_eq!(
            parse_proc_locks(table),
            vec![(1234, 123456), (5678, 789012)]
        );
    }

    #[test]
    fn duplicate_entries_for_one_pid_are_collapsed() {
        let processes = normalize(vec![
            process("dpkg lock", 10),
            process("apt", 10),
            process("brew", 20),
        ]);

        assert_eq!(processes, vec![process("apt", 10), process("brew", 20)]);
    }

    #[test]
    fn idle_host_returns_immediately_without_waiting() {
        let probe = FakeProbe::new(vec![Vec::new()]);

        let outcome = wait_for_package_managers(
            &probe,
            &PackageManagerConfig::new().with_wait(Duration::from_secs(5)),
        );

        assert_eq!(outcome, WaitOutcome::Idle);
        assert_eq!(probe.calls.get(), 1);
    }

    #[test]
    fn a_disabled_check_never_probes_the_host() {
        let probe = FakeProbe::new(vec![vec![process("brew", 1)]]);

        let outcome =
            wait_for_package_managers(&probe, &PackageManagerConfig::new().with_enabled(false));

        assert_eq!(outcome, WaitOutcome::Idle);
        assert_eq!(probe.calls.get(), 0);
    }

    #[test]
    fn a_zero_wait_aborts_on_the_first_probe() {
        let probe = FakeProbe::new(vec![vec![process("brew", 1)]]);

        let outcome = wait_for_package_managers(
            &probe,
            &PackageManagerConfig::new().with_wait(Duration::ZERO),
        );

        assert_eq!(
            outcome,
            WaitOutcome::Busy {
                waited: Duration::ZERO,
                processes: vec![process("brew", 1)]
            }
        );
        assert_eq!(probe.calls.get(), 1);
    }

    #[test]
    fn waits_until_the_package_manager_finishes() {
        let probe = FakeProbe::new(vec![
            vec![process("brew", 1)],
            vec![process("brew", 1)],
            Vec::new(),
        ]);
        let config = PackageManagerConfig::new()
            .with_wait(Duration::from_secs(5))
            .with_poll(Duration::from_millis(1));

        let outcome = wait_for_package_managers(&probe, &config);

        match outcome {
            WaitOutcome::Waited { waited, processes } => {
                assert!(waited > Duration::ZERO);
                assert_eq!(processes, vec![process("brew", 1)]);
            }
            other => panic!("expected a completed wait, got {other:?}"),
        }
        assert_eq!(probe.calls.get(), 3);
    }

    #[test]
    fn an_interrupt_stops_the_wait_without_waiting_for_the_poll_interval() {
        let probe = FakeProbe::new(vec![vec![process("brew", 1)]]);
        let interrupted = Arc::new(AtomicBool::new(false));
        let setter = Arc::clone(&interrupted);
        let signal = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            setter.store(true, Ordering::SeqCst);
        });
        let config = PackageManagerConfig::new()
            .with_wait(Duration::from_secs(5))
            .with_poll(Duration::from_secs(5));

        let started = Instant::now();
        let outcome =
            wait_for_package_managers_with(&probe, &config, || interrupted.load(Ordering::SeqCst));
        let elapsed = started.elapsed();
        signal
            .join()
            .unwrap_or_else(|_| panic!("the interrupt thread panicked"));

        match outcome {
            WaitOutcome::Interrupted { waited, processes } => {
                assert!(waited < Duration::from_secs(2), "waited {waited:?}");
                assert_eq!(processes, vec![process("brew", 1)]);
            }
            other => panic!("expected an interrupted wait, got {other:?}"),
        }
        assert!(
            elapsed < Duration::from_secs(2),
            "the interrupt waited for the poll interval: {elapsed:?}"
        );
        assert_eq!(probe.calls.get(), 1);
    }

    #[test]
    fn reports_the_process_still_running_after_the_wait() {
        let probe = FakeProbe::new(vec![vec![process("apt", 7)]]);
        let config = PackageManagerConfig::new()
            .with_wait(Duration::from_millis(10))
            .with_poll(Duration::from_millis(1));

        let outcome = wait_for_package_managers(&probe, &config);

        match outcome {
            WaitOutcome::Busy { waited, processes } => {
                assert!(waited >= Duration::from_millis(5), "waited {waited:?}");
                assert_eq!(processes, vec![process("apt", 7)]);
            }
            other => panic!("expected a busy outcome, got {other:?}"),
        }
    }

    #[test]
    fn a_process_is_rendered_with_its_pid() {
        assert_eq!(process("brew", 42).to_string(), "brew (PID 42)");
    }
}
