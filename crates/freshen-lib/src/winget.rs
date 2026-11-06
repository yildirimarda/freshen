//! The winget step: detection, a read-only check and the upgrade for Windows
//! package updates.
//!
//! `winget` is the Windows Package Manager. Detection runs `winget --version`;
//! `check()` runs `winget upgrade` and parses the table of packages with
//! updates available. `run()` refreshes the package index with `winget source
//! update` and then upgrades every package with
//! `winget upgrade --all --silent --accept-package-agreements
//! --accept-source-agreements`. Neither `detect` nor `check` changes anything,
//! so the step is safe in `--check` mode. The step is only available on
//! Windows.

use crate::options::RunOptions;
use crate::output::human;
use crate::package_processes::{
    self, HostProcessSystem, PackageLocation, ProcessSystem, StopPolicy,
};
use crate::platform::Platform;
use crate::process::{CommandSpec, ProcessError};
use crate::registry::Registry;
use crate::skipped_items;
use crate::step::{CheckReport, Detection, OutdatedItem, SkipReason, Step, StepError, StepId};

/// The winget package manager step.
pub const WINGET: StepId = "winget";

/// The reason shown for a winget package that is pinned or excluded and thus
/// held back instead of upgraded.
const HELD_BACK_REASON: &str = "held back (pinned)";

/// Exit code `winget upgrade` returns when no package upgrades are
/// applicable, so freshen reports the step as skipped instead of failed.
///
/// This is a freshen convention; confirm it against real `winget` output
/// before shipping on Windows.
pub const EXIT_UPDATE_NOT_APPLICABLE: i32 = 1;

/// winget only runs on Windows.
const WINGET_PLATFORMS: [Platform; 1] = [Platform::Windows];

/// Registers the winget step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(WingetStep::new())
}

struct WingetStep {
    /// Install locations of outdated packages, used to find running processes
    /// that must be stopped before upgrading. Populated by tests; empty in
    /// production until package-manager install-location probing is added.
    install_locations: Vec<PackageLocation>,
    /// Injected process system for tests; defaults to the host implementation.
    process_system: Option<Box<dyn ProcessSystem>>,
}

impl WingetStep {
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

    /// Reports the packages `winget upgrade` lists as outdated, honouring the
    /// package include/exclude list. Packages that are excluded (pinned) are
    /// kept in the report with a "held back (pinned)" reason instead of being
    /// reported as plainly outdated.
    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("winget")
            .args(["upgrade"])
            .neutral_locale()
            .run()
            .map_err(|error| StepError::check_from_process(WINGET, &error))?;
        let items = parse_winget_upgrade(output.stdout());
        let filter = options.filters().packages();
        let report = CheckReport::from_items(
            items
                .into_iter()
                .filter(|item| filter.includes(item.name())),
        );
        Ok(report)
    }
}

impl Step for WingetStep {
    fn id(&self) -> StepId {
        WINGET
    }

    fn display_name(&self) -> &str {
        "winget"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        if CommandSpec::new("winget").args(["--version"]).run().is_ok() {
            Ok(Detection::Installed)
        } else {
            Ok(Detection::NotInstalled)
        }
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        WingetStep::check_with_options(self, options)
    }

    /// Refreshes the package index with `winget source update` and then
    /// upgrades every package with `winget upgrade --all --silent
    /// --accept-package-agreements --accept-source-agreements`, honouring the
    /// package include/exclude list. When an include or exclude list is set,
    /// each non-held-back package is upgraded individually by id so that
    /// excluded (pinned) packages are never touched. A successful source
    /// update is required, so its failure is reported first. The upgrade's
    /// exit code is mapped as follows: `0` is ok, the update-not-applicable
    /// code is skipped (nothing to upgrade), and any other non-zero code is a
    /// failure.
    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        self.stop_running_processes(options);
        let mut failures = Vec::new();
        let mut not_applicable = false;

        human!("winget source update");
        if let Err(error) = CommandSpec::new("winget")
            .args(["source", "update"])
            .neutral_locale()
            .run()
        {
            failures.push(StepError::run_from_process(WINGET, &error));
        }

        if options.filters().packages().is_empty() {
            human!(
                "winget upgrade --all --silent --accept-package-agreements --accept-source-agreements"
            );
            let upgrade = CommandSpec::new("winget")
                .args([
                    "upgrade",
                    "--all",
                    "--silent",
                    "--accept-package-agreements",
                    "--accept-source-agreements",
                ])
                .neutral_locale()
                .run();
            if let Err(error) = &upgrade {
                if exit_code(error) == Some(EXIT_UPDATE_NOT_APPLICABLE) {
                    not_applicable = true;
                } else {
                    failures.push(StepError::run_from_process(WINGET, error));
                }
            }
        } else {
            self.upgrade_filtered(options, &mut failures, &mut not_applicable);
        }

        if let Some(error) = failures.into_iter().next() {
            return Err(error);
        }
        if not_applicable {
            return Err(StepError::run(WINGET, "no applicable package upgrades")
                .with_skip_reason(SkipReason::NothingToDo));
        }
        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &WINGET_PLATFORMS
    }
}

impl WingetStep {
    /// Upgrades every outdated package that is not held back, one by one with
    /// `winget upgrade --id <id>`, recording the held-back packages so they
    /// show up in the run summary.
    fn upgrade_filtered(
        &self,
        options: &RunOptions,
        failures: &mut Vec<StepError>,
        not_applicable: &mut bool,
    ) {
        let output = match CommandSpec::new("winget")
            .args(["upgrade"])
            .neutral_locale()
            .run()
        {
            Ok(output) => output,
            Err(error) => {
                failures.push(StepError::run_from_process(WINGET, &error));
                return;
            }
        };
        let packages = parse_winget_packages(output.stdout());
        let filter = options.filters().packages();

        let mut to_upgrade = Vec::new();
        for package in &packages {
            if filter.includes(&package.name) {
                to_upgrade.push(package.id.clone());
            } else {
                human!("{}: {HELD_BACK_REASON}", package.name);
                skipped_items::record(package.name.clone());
            }
        }

        if to_upgrade.is_empty() {
            human!("No packages to upgrade after applying the include/exclude lists.");
            *not_applicable = true;
            return;
        }

        for id in &to_upgrade {
            human!(
                "winget upgrade --id {id} --silent --accept-package-agreements --accept-source-agreements"
            );
            if let Err(error) = CommandSpec::new("winget")
                .args([
                    "upgrade",
                    "--id",
                    id,
                    "--silent",
                    "--accept-package-agreements",
                    "--accept-source-agreements",
                ])
                .neutral_locale()
                .run()
            {
                failures.push(StepError::run_from_process(WINGET, &error));
            }
        }
    }
}

/// Returns the exit code of a failed process, when one was captured.
fn exit_code(error: &ProcessError) -> Option<i32> {
    error
        .output()
        .and_then(|output| output.status())
        .and_then(|status| status.code())
}

/// A winget package as reported by `winget upgrade`, including its package id
/// so an individual package can be upgraded with `winget upgrade --id`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WingetPackage {
    name: String,
    id: String,
    version: String,
}

/// Parses the output of `winget upgrade` into packages with name, id and
/// version.
///
/// Each row of the upgrade table is `Name Id Version Source`, where `Name`
/// may contain spaces. The last three whitespace-separated tokens are
/// `Source`, `Version` and `Id`; everything before `Id` is the package name.
/// The announcement, the column titles and the separator line are skipped.
fn parse_winget_packages(output: &str) -> Vec<WingetPackage> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            // Skip the "Found the following packages available for upgrade:"
            // announcement and the "nothing to upgrade" message.
            if line.starts_with("Found the following packages")
                || line.starts_with("No applicable package upgrades found")
            {
                return None;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() < 4 {
                return None;
            }
            // Skip the column titles: `Name Id Version Source`.
            if tokens == ["Name", "Id", "Version", "Source"] {
                return None;
            }
            // Skip the separator line: runs of dashes separated by spaces.
            if tokens
                .iter()
                .all(|token| token.bytes().all(|byte| byte == b'-'))
            {
                return None;
            }
            let version = tokens.get(tokens.len() - 2).copied().unwrap_or("");
            let id = tokens.get(tokens.len() - 3).copied().unwrap_or("");
            let name = tokens[..tokens.len() - 3].join(" ");
            if name.is_empty() || id.is_empty() || version.is_empty() {
                return None;
            }
            Some(WingetPackage {
                name,
                id: id.to_owned(),
                version: version.to_owned(),
            })
        })
        .collect()
}

/// Parses the output of `winget upgrade` into outdated items.
///
/// `winget` does not report the currently installed version, so only the
/// available version is recorded.
fn parse_winget_upgrade(output: &str) -> Vec<OutdatedItem> {
    parse_winget_packages(output)
        .into_iter()
        .map(|package| OutdatedItem::new(package.name).with_latest(package.version))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        parse_winget_packages, parse_winget_upgrade, WingetStep, EXIT_UPDATE_NOT_APPLICABLE,
    };
    use crate::options::RunOptions;
    use crate::package_processes::{PackageLocation, Process, ProcessSystem};
    use crate::policy::{ItemFilters, NameFilter};
    use crate::skipped_items;
    use crate::step::{SkipReason, Step, StepError};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// A fake process system for winget tests.
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

    /// A stub `winget` that exits 0 for every command and logs invocations.
    fn stub_winget_always_ok() -> (std::path::PathBuf, std::path::PathBuf) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-winget-stop-{pid}-{id}",
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
                   printf '%s' winget\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 } >> \"$FRESHEN_WINGET_LOG\"\n\
                 exit 0\n";
            let stub = dir.join("winget");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n>>\"%FRESHEN_WINGET_LOG%\" echo winget %*\r\nexit /b 0\r\n";
            let stub = dir.join("winget.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        (dir, log)
    }

    #[test]
    fn stop_running_processes_stops_process_under_install_location() {
        let fake = FakeProcessSystem::new()
            .with_process(100, "/Program Files/Git/bin/git.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = WingetStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Program Files/Git"),
            }],
            process_system: Some(Box::new(fake)),
        };

        let options = RunOptions::new();
        step.stop_running_processes(&options);

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

        let step = WingetStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Program Files/Git"),
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
            .with_process(100, "/Program Files/Git/bin/git.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = WingetStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Program Files/Git"),
            }],
            process_system: Some(Box::new(fake)),
        };

        let options = RunOptions::new().with_force_kill(true);
        step.stop_running_processes(&options);

        let signals = signals.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert_eq!(signals, [(100, false), (100, true)], "quit then force-kill");
    }

    #[test]
    fn stop_running_processes_does_not_force_kill_without_flag() {
        let fake = FakeProcessSystem::new()
            .with_process(100, "/Program Files/Git/bin/git.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = WingetStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Program Files/Git"),
            }],
            process_system: Some(Box::new(fake)),
        };

        step.stop_running_processes(&RunOptions::new());

        assert_eq!(
            signals.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            [(100, false)],
            "only graceful quit, no force-kill"
        );
    }

    #[test]
    fn stop_running_processes_never_stops_frenhen_process() {
        let fake = FakeProcessSystem::new()
            .with_current_pid(999)
            .with_process(999, r"C:\Program Files\Git\bin\git.exe");
        let signals = fake.signals.clone();

        let step = WingetStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Program Files/Git"),
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
        let (dir, log) = stub_winget_always_ok();
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINGET_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINGET_LOG", &log);

        let fake = FakeProcessSystem::new()
            .with_process(100, "/Program Files/Git/bin/git.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = WingetStep {
            install_locations: vec![PackageLocation {
                name: "git".to_owned(),
                install_location: PathBuf::from("/Program Files/Git"),
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
            std::env::set_var("FRESHEN_WINGET_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_ok(), "run should succeed: {result:?}");

        // The process was stopped before the upgrade commands ran.
        assert_eq!(
            signals.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            [(100, false)],
            "process should be quit before upgrade"
        );
    }

    #[test]
    fn run_with_options_no_locations_means_no_stopping() {
        let (dir, log) = stub_winget_always_ok();
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINGET_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINGET_LOG", &log);

        let fake = FakeProcessSystem::new()
            .with_process(100, "/Program Files/Git/bin/git.exe")
            .with_current_pid(999);
        let signals = fake.signals.clone();

        let step = WingetStep {
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
            std::env::set_var("FRESHEN_WINGET_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_LOG");
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

    /// Creates a stub `winget` on a temp dir that prints `output` for
    /// `winget upgrade` and exits 0 for `winget --version`. Returns the stub
    /// directory; the caller must restore `PATH` and remove it.
    fn stub_winget_with_upgrade_output(output: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-winget-check-{pid}-{id}",
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
                 {{\n\
                    printf '%s' winget\n\
                    for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                    printf '\\n'\n\
                 }} >> \"$FRESHEN_WINGET_LOG\"\n\
                 case \"$1\" in\n\
                    --version) exit 0 ;;\n\
                 esac\n\
                 case \"$2\" in\n\
                    upgrade) printf {}; exit 0 ;;\n                   *) exit 0 ;;\n                 esac\n",
                shell_quote(output)
            );
            let stub = dir.join("winget");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            // The table is printed from a file: a multi-line `echo` would turn
            // every line after the first into a separate `cmd` command.
            let output_file = dir.join("upgrade-output.txt");
            let table = output.replace('\n', "\r\n");
            std::fs::write(&output_file, table)
                .unwrap_or_else(|error| panic!("write {}: {error}", output_file.display()));
            let script = "@echo off\r\n\
                 if \"%~2\"==\"upgrade\" type \"%~dp0upgrade-output.txt\"\r\n\
                 exit /b 0\r\n";
            let stub = dir.join("winget.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        dir
    }

    /// Quotes `text` as a single shell word.
    #[cfg(unix)]
    fn shell_quote(text: &str) -> String {
        format!("'{}'", text.replace('\'', "'\\''"))
    }

    #[test]
    fn parses_the_upgrade_table() {
        let items = parse_winget_upgrade(
            "Found the following packages available for upgrade:\n\
             Name                Id                          Version        Source\n\
             --------------------------------------------------------------------------------\n\
             7zip                7zip.7zip                   24.08        winget\n\
             Git                 Git.Git                     2.45.2       winget\n\
             Microsoft Visual C++ Redistributable  Microsoft.VC++.Redist  14.42.34512  winget\n",
        );

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name(), "7zip");
        assert_eq!(items[0].current(), None);
        assert_eq!(items[0].latest(), Some("24.08"));
        assert_eq!(items[1].name(), "Git");
        assert_eq!(items[1].latest(), Some("2.45.2"));
        assert_eq!(items[2].name(), "Microsoft Visual C++ Redistributable");
        assert_eq!(items[2].latest(), Some("14.42.34512"));
    }

    #[test]
    fn parses_packages_with_ids_and_versions() {
        let packages = parse_winget_packages(
            "Found the following packages available for upgrade:\n\
             Name Id Version Source\n\
             ----------------------\n\
             7zip 7zip.7zip 24.08 winget\n\
             Git Git.Git 2.45.2 winget\n",
        );

        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "7zip");
        assert_eq!(packages[0].id, "7zip.7zip");
        assert_eq!(packages[0].version, "24.08");
        assert_eq!(packages[1].name, "Git");
        assert_eq!(packages[1].id, "Git.Git");
        assert_eq!(packages[1].version, "2.45.2");
    }

    #[test]
    fn reports_nothing_when_no_upgrades_are_available() {
        let items = parse_winget_upgrade("No applicable package upgrades found.\n");
        assert!(items.is_empty());
    }

    #[test]
    fn skips_the_header_and_separator_lines() {
        let items = parse_winget_upgrade(
            "Found the following packages available for upgrade:\n\
             Name Id Version Source\n\
             ----------------------\n",
        );
        assert!(items.is_empty());
    }

    #[test]
    fn ignores_short_lines_that_do_not_form_a_row() {
        let items = parse_winget_upgrade("7zip\nwinget\n");
        assert!(items.is_empty());
    }

    #[test]
    fn check_excludes_packages_from_report() {
        let dir = stub_winget_with_upgrade_output(
            "Found the following packages available for upgrade:\n\
             Name Id Version Source\n\
             ----------------------\n\
             7zip 7zip.7zip 24.08 winget\n\
             Git Git.Git 2.45.2 winget\n\
             Microsoft Visual C++ Redistributable Microsoft.VC++.Redist 14.42.34512 winget\n",
        );
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINGET_LOG").ok();
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINGET_LOG", dir.join("calls.log"));

        let options = RunOptions::new().with_filters(
            ItemFilters::new().with_packages(NameFilter::new().with_exclude(["Git"])),
        );
        let report = WingetStep::new()
            .check_with_options(&options)
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINGET_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_LOG");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(report.pending(), 2);
        assert!(!report.items().iter().any(|item| item.name() == "Git"));
        assert!(report.items().iter().any(|item| item.name() == "7zip"));
    }

    #[test]
    fn check_holds_back_packages_outside_the_include_list() {
        let dir = stub_winget_with_upgrade_output(
            "Found the following packages available for upgrade:\n\
             Name Id Version Source\n\
             ----------------------\n\
             7zip 7zip.7zip 24.08 winget\n\
             Git Git.Git 2.45.2 winget\n",
        );
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let options = RunOptions::new().with_filters(
            ItemFilters::new().with_packages(NameFilter::new().with_include(["7zip"])),
        );
        let report = WingetStep::new()
            .check_with_options(&options)
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(report.pending(), 1);
        assert_eq!(report.items()[0].name(), "7zip");
        assert_eq!(report.items()[0].reason(), None);
    }

    #[test]
    fn exclude_list_wins_over_include_list() {
        let dir = stub_winget_with_upgrade_output(
            "Found the following packages available for upgrade:\n\
             Name Id Version Source\n\
             ----------------------\n\
             7zip 7zip.7zip 24.08 winget\n\
             Git Git.Git 2.45.2 winget\n",
        );
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", &dir);

        let options = RunOptions::new().with_filters(
            ItemFilters::new().with_packages(
                NameFilter::new()
                    .with_include(["7zip", "Git"])
                    .with_exclude(["Git"]),
            ),
        );
        let report = WingetStep::new()
            .check_with_options(&options)
            .unwrap_or_else(|error| panic!("check should succeed: {error}"));

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(report.pending(), 1);
        assert_eq!(report.items()[0].name(), "7zip");
    }

    /// Runs `winget source update` before `winget upgrade`, recording each
    /// invocation against a stub `winget` placed on `PATH`.
    #[test]
    fn run_updates_the_source_before_upgrading() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-winget-run-{pid}-{id}",
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
                    printf '%s' winget\n\
                    for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                    printf '\\n'\n\
                 } >> \"$FRESHEN_WINGET_LOG\"\n\
                 exit 0\n";
            let stub = dir.join("winget");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n>>\"%FRESHEN_WINGET_LOG%\" echo winget %*\r\nexit /b 0\r\n";
            let stub = dir.join("winget.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINGET_LOG").ok();

        // Serialize with the other tests that replace the global `PATH` (snap
        // replaces it entirely) so this test's stub `winget` is not shadowed
        // while `run()` spawns it twice.
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINGET_LOG", &log);

        let result = WingetStep::new().run();

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINGET_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_LOG");
        }

        assert!(result.is_ok(), "winget run should succeed");

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);

        let calls: Vec<&str> = calls.lines().map(str::trim).collect();
        assert_eq!(
            calls,
            vec![
                "winget --disable-interactivity source update",
                "winget --disable-interactivity upgrade --all --silent --accept-package-agreements --accept-source-agreements",
            ],
            "calls: {calls:?}"
        );
    }

    /// With an exclude list, each non-held-back package is upgraded
    /// individually by id and held-back packages are recorded.
    #[test]
    fn run_holds_back_excluded_packages() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-winget-filter-{pid}-{id}",
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
                    printf '%s' winget\n\
                    for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                    printf '\\n'\n\
                 } >> \"$FRESHEN_WINGET_LOG\"\n\
                 case \"$2\" in\n\
                    source) exit 0 ;;\n\
                    upgrade)\n\
                      if [ \"$3\" = --id ]; then\n\
                        exit 0\n\
                      else\n\
                        printf 'Name Id Version Source\\n'\n\
                        printf '------ -- -- ------- ------\\n'\n\
                        printf '7zip 7zip.7zip 24.08 winget\\n'\n\
                        printf 'Git Git.Git 2.45.2 winget\\n'\n\
                        exit 0\n\
                      fi ;;\n                   *) exit 0 ;;\n                 esac\n";
            let stub = dir.join("winget");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n\
                 >>\"%FRESHEN_WINGET_LOG%\" echo winget %*\r\n\
                 if \"%~2\"==\"source\" (exit /b 0) else (\n\
                    if \"%~3\"==\"--id\" (exit /b 0) else (\n\
                      echo Name Id Version Source\r\n\
                      echo 7zip 7zip.7zip 24.08 winget\r\n\
                      echo Git Git.Git 2.45.2 winget\r\n\
                      exit /b 0\n                   )\n                 )\r\n";
            let stub = dir.join("winget.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINGET_LOG").ok();

        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINGET_LOG", &log);

        let options = RunOptions::new().with_filters(
            ItemFilters::new().with_packages(NameFilter::new().with_exclude(["Git"])),
        );
        let result = WingetStep::new().run_with_options(&options);

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINGET_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_LOG");
        }

        assert!(result.is_ok(), "filtered run should succeed");

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);

        let calls: Vec<&str> = calls.lines().map(str::trim).collect();
        assert!(
            calls.contains(&"winget --disable-interactivity source update"),
            "source update missing: {calls:?}"
        );
        assert!(
            calls.contains(&"winget --disable-interactivity upgrade --id 7zip.7zip --silent --accept-package-agreements --accept-source-agreements"),
            "7zip should be upgraded by id: {calls:?}"
        );
        assert!(
            !calls.iter().any(|call| call.contains("Git.Git")),
            "Git should not be upgraded: {calls:?}"
        );

        let held_back = skipped_items::take();
        assert_eq!(held_back, vec!["Git"], "held back packages: {held_back:?}");
    }

    /// Runs `winget source update` (always exit 0) followed by `winget
    /// upgrade --all ...` which exits with `exit_code`, recording both
    /// invocations against a stub `winget` on `PATH`.
    fn run_winget_with_upgrade_exit(exit_code: i32) -> (Result<(), StepError>, Vec<String>) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-winget-exit-{pid}-{id}",
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
                    printf '%s' winget\n\
                    for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                    printf '\\n'\n\
                 } >> \"$FRESHEN_WINGET_LOG\"\n\
                 case \"$2\" in\n\
                    source) exit 0 ;;\n\
                    *) exit \"${FRESHEN_WINGET_EXIT_CODE:-0}\" ;;\n\
                 esac\n";
            let stub = dir.join("winget");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n\
                 >>\"%FRESHEN_WINGET_LOG%\" echo winget %*\r\n\
                 set \"code=%FRESHEN_WINGET_EXIT_CODE%\r\n\
                 if not defined code set code=0\r\n\
                 if \"%~2\"==\"source\" (\r\n\
                    exit /b 0\r\n\
                 ) else (\r\n\
                    exit /b %code%\r\n\
                 )\r\n";
            let stub = dir.join("winget.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINGET_LOG").ok();
        let previous_exit_code = std::env::var("FRESHEN_WINGET_EXIT_CODE").ok();

        // Serialize with the other tests that replace the global `PATH`
        // while `run()` spawns the stub twice.
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINGET_LOG", &log);
        std::env::set_var("FRESHEN_WINGET_EXIT_CODE", exit_code.to_string());

        let result = WingetStep::new().run();

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINGET_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_LOG");
        }
        if let Some(previous) = previous_exit_code {
            std::env::set_var("FRESHEN_WINGET_EXIT_CODE", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_EXIT_CODE");
        }

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);

        let calls: Vec<String> = calls.lines().map(str::trim).map(String::from).collect();
        (result, calls)
    }

    /// The upgrade exit code maps to ok / skipped / failed: exit 0 is ok, the
    /// update-not-applicable code is skipped (nothing to upgrade) and any
    /// other non-zero code is a failure.
    #[test]
    fn run_maps_upgrade_exit_codes_to_ok_skipped_and_failed() {
        for (exit_code, expect_ok, expect_skipped) in [
            (0, true, false),
            (EXIT_UPDATE_NOT_APPLICABLE, false, true),
            (42, false, false),
        ] {
            let (result, calls) = run_winget_with_upgrade_exit(exit_code);

            if expect_ok {
                assert!(result.is_ok(), "exit {exit_code} should be ok");
            } else {
                let error = match result {
                    Err(error) => error,
                    Ok(()) => panic!("exit {exit_code} should have failed, got Ok"),
                };
                let expected_skip = if expect_skipped {
                    Some(SkipReason::NothingToDo)
                } else {
                    None
                };
                assert_eq!(
                    error.skip_reason(),
                    expected_skip,
                    "exit {exit_code} skip reason"
                );
            }

            assert_eq!(
                calls,
                &[
                    "winget --disable-interactivity source update",
                    "winget --disable-interactivity upgrade --all --silent --accept-package-agreements --accept-source-agreements",
                ],
                "calls for exit {exit_code}: {calls:?}"
            );
        }
    }
}
