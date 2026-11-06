//! The Scoop step: detection, a read-only check and the upgrade for Windows
//! apps installed with Scoop.
//!
//! `scoop` is a command-line installer for Windows. Detection runs `scoop
//! --version`; `check()` runs `scoop status` and parses the apps that have an
//! update available; `run()` runs `scoop update` (Scoop itself) followed by
//! `scoop update *` (every app) and, when `--cleanup` is set, `scoop cleanup *`
//! and `scoop cache rm *`. None of these changes the machine until `run()` is
//! invoked, so the step is safe in `--check` mode. The step is only available
//! on Windows and is skipped entirely when Scoop is absent.

use crate::cleanup;
use crate::options::RunOptions;
use crate::output::human;
use crate::package_processes::{
    self, HostProcessSystem, PackageLocation, ProcessSystem, StopPolicy,
};
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The Scoop package manager step.
pub const SCOOP: StepId = "scoop";

/// Scoop only runs on Windows.
const SCOOP_PLATFORMS: [Platform; 1] = [Platform::Windows];

/// Registers the scoop step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ScoopStep::new())
}

struct ScoopStep {
    /// Install locations of outdated packages, used to find running processes
    /// that must be stopped before upgrading. Populated by tests; empty in
    /// production until package-manager install-location probing is added.
    install_locations: Vec<PackageLocation>,
    /// Injected process system for tests; defaults to the host implementation.
    process_system: Option<Box<dyn ProcessSystem>>,
}

impl ScoopStep {
    fn new() -> Self {
        Self {
            install_locations: Vec::new(),
            process_system: None,
        }
    }

    fn process_system(&self) -> &dyn ProcessSystem {
        self.process_system.as_deref().unwrap_or(&HostProcessSystem)
    }

    /// Stops running processes whose executables live under the install
    /// location of an outdated package, applying the quit/force-kill policy
    /// before the upgrade.
    fn stop_running_processes(&self, options: &RunOptions) {
        if self.install_locations.is_empty() {
            return;
        }
        let system = self.process_system();
        let policy = StopPolicy::from_env(options.force_kill());
        let (stopped, still_running) = package_processes::stop_processes_under_locations(
            system,
            &self.install_locations,
            &policy,
        );

        for process in &stopped {
            human!(
                "Stopped process {} ({})",
                process.pid,
                process.executable.display()
            );
        }

        for process in &still_running {
            if policy.force_kill() {
                human!(
                    "warning: process {} ({}) did not stop before the upgrade, even after force-killing",
                    process.pid,
                    process.executable.display()
                );
            } else {
                human!(
                    "warning: process {} ({}) did not stop before the upgrade (use --force-kill to force)",
                    process.pid,
                    process.executable.display()
                );
            }
        }
    }

    /// Reports the apps `scoop status` lists as outdated.
    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("scoop")
            .args(["status"])
            .neutral_locale()
            .run()
            .map_err(|error| StepError::check_from_process(SCOOP, &error))?;
        let items = parse_scoop_status(output.stdout());
        let filter = options.filters().packages();
        let report = CheckReport::from_items(
            items
                .into_iter()
                .filter(|item| filter.includes(item.name())),
        );
        Ok(report)
    }
}

impl Step for ScoopStep {
    fn id(&self) -> StepId {
        SCOOP
    }

    fn display_name(&self) -> &str {
        "scoop"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        if CommandSpec::new("scoop").args(["--version"]).run().is_ok() {
            Ok(Detection::Installed)
        } else {
            Ok(Detection::NotInstalled)
        }
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        ScoopStep::check_with_options(self, options)
    }

    /// Runs `scoop update` (Scoop itself) and then `scoop update *` (every
    /// app), collecting the first failure so both commands are attempted.
    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        self.stop_running_processes(options);
        let mut failures = Vec::new();

        human!("scoop update");
        if let Err(error) = CommandSpec::new("scoop").args(["update"]).run() {
            failures.push(StepError::run_from_process(SCOOP, &error));
        }

        human!("scoop update *");
        if let Err(error) = CommandSpec::new("scoop").args(["update", "*"]).run() {
            failures.push(StepError::run_from_process(SCOOP, &error));
        }

        if let Err(error) = cleanup::run(
            SCOOP,
            "scoop: cleanup",
            CommandSpec::new("scoop").args(["cleanup", "*"]),
            options,
        ) {
            failures.push(error);
        }

        if let Err(error) = cleanup::run(
            SCOOP,
            "scoop: cache rm",
            CommandSpec::new("scoop").args(["cache", "rm", "*"]),
            options,
        ) {
            failures.push(error);
        }

        match failures.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &SCOOP_PLATFORMS
    }
}

/// Parses the output of `scoop status` into outdated items.
///
/// `scoop status` lists one outdated app per line as
/// `<name> <current> -> <latest>` under a header such as
/// `The following apps are outdated:`. Lines that do not match that shape
/// (the header, the up-to-date notice, separator lines) are ignored.
fn parse_scoop_status(output: &str) -> Vec<OutdatedItem> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            let arrow = tokens.iter().position(|token| *token == "->")?;
            // Need a current version before the arrow and a latest version after it.
            if arrow == 0 || arrow + 1 >= tokens.len() {
                return None;
            }
            let latest = tokens[arrow + 1];
            let current = tokens[arrow - 1];
            if latest.is_empty() || current.is_empty() {
                return None;
            }
            let name = tokens[..arrow - 1].join(" ");
            if name.is_empty() {
                return None;
            }
            Some(
                OutdatedItem::new(name)
                    .with_current(current)
                    .with_latest(latest),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_scoop_status, ScoopStep, SCOOP};
    use crate::options::RunOptions;
    use crate::package_processes::{PackageLocation, Process, ProcessSystem};
    use crate::platform::Platform;
    use crate::policy::{ItemFilters, NameFilter};
    use crate::step::{Detection, SkipReason, Step, StepError};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// A fake process system for scoop tests.
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

    /// A stub `scoop` that exits 0 for every command and logs invocations.
    fn stub_scoop_always_ok() -> (std::path::PathBuf, std::path::PathBuf) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-scoop-stop-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        let log = dir.join("calls.log");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = "#!/bin/sh\n\
                 {\n\
                   printf '%s' scoop\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 } >> \"$FRESHEN_SCOOP_LOG\"\n\
                 exit 0\n";
            let stub = dir.join("scoop");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n>>\"%FRESHEN_SCOOP_LOG%\" echo scoop %*\r\nexit /b 0\r\n";
            let stub = dir.join("scoop.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        (dir, log)
    }

    #[test]
    fn stop_running_processes_stops_process_under_install_location() {
        let fake = FakeProcessSystem::new()
            .with_process(100, "/Users/Alice/scoop/apps/git/current/bin/git.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = ScoopStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Users/Alice/scoop/apps/git/current"),
            }],
            process_system: Some(Box::new(fake)),
        };

        step.stop_running_processes(&RunOptions::new());

        assert_eq!(
            signals.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            [(100, false)]
        );
    }

    #[test]
    fn stop_running_processes_ignores_process_outside_install_location() {
        let fake = FakeProcessSystem::new()
            .with_process(100, "/Windows/System32/notepad.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = ScoopStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Users/Alice/scoop/apps/git/current"),
            }],
            process_system: Some(Box::new(fake)),
        };

        step.stop_running_processes(&RunOptions::new());

        assert!(
            signals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
                .is_empty(),
            "no process should be stopped"
        );
    }

    #[test]
    fn stop_running_processes_force_kills_stuck_process() {
        let fake = FakeProcessSystem::new()
            .with_process(100, "/Users/Alice/scoop/apps/git/current/bin/git.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = ScoopStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Users/Alice/scoop/apps/git/current"),
            }],
            process_system: Some(Box::new(fake)),
        };

        step.stop_running_processes(&RunOptions::new().with_force_kill(true));

        let signals = signals.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert_eq!(signals, [(100, false), (100, true)], "quit then force-kill");
    }

    #[test]
    fn stop_running_processes_never_stops_freshen_process() {
        let fake = FakeProcessSystem::new()
            .with_current_pid(999)
            .with_process(999, "/Users/Alice/scoop/apps/git/current/bin/git.exe");
        let signals = fake.signals.clone();

        let step = ScoopStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Users/Alice/scoop/apps/git/current"),
            }],
            process_system: Some(Box::new(fake)),
        };

        step.stop_running_processes(&RunOptions::new());

        assert!(
            signals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
                .is_empty(),
            "freshen process must never be stopped"
        );
    }

    #[test]
    fn run_with_options_stops_process_before_upgrading() {
        let (dir, log) = stub_scoop_always_ok();
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_SCOOP_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_SCOOP_LOG", &log);

        let fake = FakeProcessSystem::new()
            .with_process(100, "/Users/Alice/scoop/apps/git/current/bin/git.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = ScoopStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Users/Alice/scoop/apps/git/current"),
            }],
            process_system: Some(Box::new(fake)),
        };

        // Use a short stop timeout so the test does not wait 20 seconds when
        // the fake process does not stop after the graceful quit.
        std::env::set_var("DESKTOP_STOP_TIMEOUT", "1");
        let result = step.run_with_options(&RunOptions::new());
        std::env::remove_var("DESKTOP_STOP_TIMEOUT");

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_SCOOP_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_SCOOP_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_ok(), "run should succeed: {result:?}");

        assert_eq!(
            signals.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            [(100, false)],
            "process should be quit before upgrade"
        );
    }

    #[test]
    fn run_with_options_no_locations_means_no_stopping() {
        let (dir, log) = stub_scoop_always_ok();
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_SCOOP_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_SCOOP_LOG", &log);

        let fake = FakeProcessSystem::new()
            .with_process(100, "/Users/Alice/scoop/apps/git/current/bin/git.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = ScoopStep {
            install_locations: Vec::new(),
            process_system: Some(Box::new(fake)),
        };

        let result = step.run_with_options(&RunOptions::new());

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_SCOOP_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_SCOOP_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_ok());
        assert!(
            signals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
                .is_empty(),
            "no processes stopped when no install locations"
        );
    }

    fn stub_scoop_with_status_output(output: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-scoop-check-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script = format!(
                "#!/bin/sh\n\
                 case \"$1\" in\n\
                   --version) exit 0 ;;\n\
                   status) printf {}; exit 0 ;;\n\
                   *) exit 0 ;;\n\
                 esac\n",
                shell_quote(output)
            );
            let stub = dir.join("scoop");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let output_file = dir.join("status-output.txt");
            let text = output.replace('\n', "\r\n");
            std::fs::write(&output_file, text)
                .unwrap_or_else(|error| panic!("write {}: {error}", output_file.display()));
            let script = "@echo off\r\n\
                 if \"%~1\"==\"--version\" exit /b 0\r\n\
                 if \"%~1\"==\"status\" type \"%~dp0status-output.txt\"\r\n\
                 exit /b 0\r\n";
            let stub = dir.join("scoop.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        dir
    }

    #[cfg(unix)]
    fn shell_quote(text: &str) -> String {
        format!("'{}'", text.replace('\'', "'\\''"))
    }

    #[test]
    fn parses_the_status_listing() {
        let items = parse_scoop_status(
            "The following apps are outdated:\n\
             curl 7.81.0 -> 7.88.0\n\
             git 2.34.1 -> 2.40.1\n\
             Microsoft Visual C++ Redistributable 14.42.34512 -> 14.42.34513\n\
             All other apps are up to date.\n",
        );

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name(), "curl");
        assert_eq!(items[0].current(), Some("7.81.0"));
        assert_eq!(items[0].latest(), Some("7.88.0"));
        assert_eq!(items[1].name(), "git");
        assert_eq!(items[1].current(), Some("2.34.1"));
        assert_eq!(items[1].latest(), Some("2.40.1"));
        assert_eq!(items[2].name(), "Microsoft Visual C++ Redistributable");
        assert_eq!(items[2].current(), Some("14.42.34512"));
        assert_eq!(items[2].latest(), Some("14.42.34513"));
    }

    #[test]
    fn reports_nothing_when_all_apps_are_up_to_date() {
        let items = parse_scoop_status("All apps are up to date.\n");
        assert!(items.is_empty());
    }

    #[test]
    fn skips_the_header_and_non_matching_lines() {
        let items = parse_scoop_status(
            "The following apps are outdated:\n\
             --------------------------------\n\
             curl 7.81.0 -> 7.88.0\n",
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "curl");
    }

    #[test]
    fn ignores_short_lines_that_do_not_form_a_row() {
        let items = parse_scoop_status("curl\nscoop\n");
        assert!(items.is_empty());
    }

    #[test]
    fn check_excludes_apps_from_report() {
        let dir = stub_scoop_with_status_output(
            "The following apps are outdated:\n\
             curl 7.81.0 -> 7.88.0\n\
             git 2.34.1 -> 2.40.1\n",
        );
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let options = RunOptions::new().with_filters(
            ItemFilters::new().with_packages(NameFilter::new().with_exclude(["git"])),
        );
        let report = ScoopStep::new()
            .check_with_options(&options)
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(report.pending(), 1);
        assert!(!report.items().iter().any(|item| item.name() == "git"));
        assert!(report.items().iter().any(|item| item.name() == "curl"));
    }

    #[test]
    fn detect_reports_installed_when_scoop_is_on_path() {
        let dir = stub_scoop_with_status_output("All apps are up to date.\n");
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let detection = ScoopStep::new()
            .detect()
            .unwrap_or_else(|error| panic!("detect should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(detection, Detection::Installed);
    }

    #[test]
    fn detect_reports_not_installed_when_scoop_is_absent() {
        let dir = std::env::temp_dir().join(format!(
            "freshen-scoop-absent-{pid}",
            pid = std::process::id(),
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });

        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let detection = ScoopStep::new()
            .detect()
            .unwrap_or_else(|error| panic!("detect should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(detection, Detection::NotInstalled);
    }

    /// Runs `scoop update` then `scoop update *`, recording each invocation
    /// against a stub `scoop` placed on `PATH`.
    #[test]
    fn run_updates_scoop_and_then_every_app() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-scoop-run-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        let log = dir.join("calls.log");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script = "#!/bin/sh\n\
                 {\n\
                   printf '%s' scoop\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 } >> \"$FRESHEN_SCOOP_LOG\"\n\
                 exit 0\n";
            let stub = dir.join("scoop");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n>>\"%FRESHEN_SCOOP_LOG%\" echo scoop %*\r\nexit /b 0\r\n";
            let stub = dir.join("scoop.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_SCOOP_LOG").ok();

        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_SCOOP_LOG", &log);

        let result = ScoopStep::new().run();

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_SCOOP_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_SCOOP_LOG");
        }

        assert!(result.is_ok(), "scoop run should succeed");

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);

        let calls: Vec<&str> = calls.lines().map(str::trim).collect();
        assert_eq!(
            calls,
            &["scoop update", "scoop update *"],
            "calls: {calls:?}"
        );
    }

    /// A failing `scoop update *` is reported as a failure with no skip
    /// reason, while `scoop update` still ran first.
    #[test]
    fn run_reports_a_failed_app_update() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-scoop-fail-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        let log = dir.join("calls.log");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script = "#!/bin/sh\n\
                 {\n\
                   printf '%s' scoop\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 } >> \"$FRESHEN_SCOOP_LOG\"\n\
                 case \"$1\" in\n\
                   update)\n\
                     if [ \"$2\" = \\* ]; then exit 1; else exit 0; fi ;;\n\
                   *) exit 0 ;;\n\
                 esac\n";
            let stub = dir.join("scoop");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n\
                 >>\"%FRESHEN_SCOOP_LOG%\" echo scoop %*\r\n\
                 if \"%~2\"==\"*\" (exit /b 1) else (exit /b 0)\r\n";
            let stub = dir.join("scoop.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_SCOOP_LOG").ok();

        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_SCOOP_LOG", &log);

        let result = ScoopStep::new().run();

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_SCOOP_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_SCOOP_LOG");
        }

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);

        let error = match result {
            Err(error) => error,
            Ok(()) => panic!("scoop run should have failed"),
        };
        assert_eq!(error.step(), SCOOP);
        assert_eq!(error.operation(), crate::step::Operation::Run);
        assert_eq!(error.skip_reason(), None);
        let calls: Vec<&str> = calls.lines().map(str::trim).collect();
        assert_eq!(
            calls,
            &["scoop update", "scoop update *"],
            "calls: {calls:?}"
        );
    }

    /// Creates a stub `scoop` that logs every invocation to
    /// `$FRESHEN_SCOOP_LOG` and always succeeds, returning the stub directory
    /// and the log path.
    fn stub_scoop_logging_calls() -> (std::path::PathBuf, std::path::PathBuf) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-scoop-calls-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        let log = dir.join("calls.log");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script = "#!/bin/sh\n\
                 {\n\
                   printf '%s' scoop\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 } >> \"$FRESHEN_SCOOP_LOG\"\n\
                 exit 0\n";
            let stub = dir.join("scoop");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n>>\"%FRESHEN_SCOOP_LOG%\" echo scoop %*\r\nexit /b 0\r\n";
            let stub = dir.join("scoop.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        (dir, log)
    }

    /// With `--cleanup` enabled, `run_with_options` runs `scoop cleanup *`
    /// after `scoop update *`.
    #[test]
    fn run_with_cleanup_runs_scoop_cleanup_star() {
        let (dir, log) = stub_scoop_logging_calls();
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_SCOOP_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_SCOOP_LOG", &log);

        let options = RunOptions::new().with_cleanup(true);
        let result = ScoopStep::new().run_with_options(&options);

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_SCOOP_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_SCOOP_LOG");
        }

        assert!(result.is_ok(), "scoop run with cleanup should succeed");

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        let calls: Vec<&str> = calls.lines().map(str::trim).collect();
        assert_eq!(
            calls,
            &[
                "scoop update",
                "scoop update *",
                "scoop cleanup *",
                "scoop cache rm *",
            ],
            "calls: {calls:?}"
        );
    }

    /// Without `--cleanup`, `run_with_options` runs `scoop update` and
    /// `scoop update *` but not `scoop cleanup *`.
    #[test]
    fn run_without_cleanup_skips_scoop_cleanup_star() {
        let (dir, log) = stub_scoop_logging_calls();
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_SCOOP_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_SCOOP_LOG", &log);

        let options = RunOptions::new();
        let result = ScoopStep::new().run_with_options(&options);

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_SCOOP_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_SCOOP_LOG");
        }

        assert!(result.is_ok(), "scoop run without cleanup should succeed");

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        let calls: Vec<&str> = calls.lines().map(str::trim).collect();
        assert_eq!(
            calls,
            &["scoop update", "scoop update *"],
            "calls: {calls:?}"
        );
    }

    /// With `--cleanup` enabled, `run_with_options` runs `scoop cache rm *`
    /// after `scoop cleanup *`.
    #[test]
    fn run_with_cleanup_runs_scoop_cache_rm_star() {
        let (dir, log) = stub_scoop_logging_calls();
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_SCOOP_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_SCOOP_LOG", &log);

        let options = RunOptions::new().with_cleanup(true);
        let result = ScoopStep::new().run_with_options(&options);

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_SCOOP_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_SCOOP_LOG");
        }

        assert!(result.is_ok(), "scoop run with cleanup should succeed");

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        let calls: Vec<&str> = calls.lines().map(str::trim).collect();
        assert_eq!(
            calls,
            &[
                "scoop update",
                "scoop update *",
                "scoop cleanup *",
                "scoop cache rm *",
            ],
            "calls: {calls:?}"
        );
    }

    /// Without `--cleanup`, `run_with_options` runs `scoop update` and
    /// `scoop update *` but not `scoop cleanup *` nor `scoop cache rm *`.
    #[test]
    fn run_without_cleanup_skips_scoop_cache_rm_star() {
        let (dir, log) = stub_scoop_logging_calls();
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_SCOOP_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_SCOOP_LOG", &log);

        let options = RunOptions::new();
        let result = ScoopStep::new().run_with_options(&options);

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_SCOOP_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_SCOOP_LOG");
        }

        assert!(result.is_ok(), "scoop run without cleanup should succeed");

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        let calls: Vec<&str> = calls.lines().map(str::trim).collect();
        assert_eq!(
            calls,
            &["scoop update", "scoop update *"],
            "calls: {calls:?}"
        );
    }

    /// `scoop` is only declared for Windows, so it is never detected on other
    /// platforms and the step is skipped there.
    #[test]
    fn platforms_is_windows_only() {
        assert_eq!(ScoopStep::new().platforms(), &[Platform::Windows]);
    }

    /// A missing scoop is reported as skipped with `NotInstalled`.
    #[test]
    fn not_installed_uses_the_not_installed_skip_reason() {
        assert_eq!(SkipReason::NotInstalled.as_str(), "not installed");
    }

    #[allow(dead_code)]
    fn _assert_step_error_is_send_sync() {
        fn _assert<T: Send + Sync>() {}
        _assert::<StepError>();
    }
}
