//! The apt step: `apt-get update` followed by `apt-get upgrade -y`, and
//! `apt-get autoremove -y` plus `apt-get autoclean` when `--cleanup` is set.
//!
//! The zsh implementation runs `apt-get update` and `apt-get upgrade -y`
//! whenever `apt-get` is available. The step is skipped entirely when apt is
//! absent, so it is safe to register on every machine; on macOS and Windows
//! it is never detected.

use crate::cleanup;
#[cfg(target_os = "linux")]
use crate::containers::{ContainerRuntime, NativeDockerRuntime};
#[cfg(target_os = "linux")]
use crate::desktop_apps::{self, HostDesktopAppSystem, StopPolicy};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::restart_needed;
use crate::services_needing_restart;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};
use crate::wsl::inside_command;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::Mutex;

/// The apt package manager step.
pub const APT: StepId = "apt";

/// apt runs on Linux natively and, on Windows, inside a selected WSL
/// distribution via `wsl -d <distro> apt-get ...`.
const APT_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// Docker packages whose upgrade requires the Docker Engine to be restarted.
#[cfg(target_os = "linux")]
const DOCKER_PACKAGES: [&str; 2] = ["docker.io", "docker-ce"];

/// Registers the apt step, after the Homebrew steps.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(AptStep::new())
}

struct AptStep {
    /// Package names reported as outdated by the last `check()` call. Used to
    /// decide whether the Docker Engine needs to be stopped before the
    /// upgrade, without running a second `apt-get -s upgrade`.
    #[cfg(target_os = "linux")]
    last_check_packages: Mutex<Vec<String>>,
}

impl AptStep {
    fn new() -> Self {
        #[cfg(target_os = "linux")]
        let last_check_packages = Mutex::new(Vec::new());
        Self {
            #[cfg(target_os = "linux")]
            last_check_packages,
        }
    }

    /// Runs `apt-get update` and then `apt-get upgrade -y`, collecting the
    /// first failure so both commands are attempted. On Windows the commands
    /// run inside the selected WSL distribution via `wsl -d <distro>`.
    fn upgrade(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        human!("apt-get update");
        if let Err(error) = inside_command(
            options,
            CommandSpec::new("apt-get")
                .args(["update"])
                .neutral_locale()
                .retry_transient_network(NETWORK_RETRIES),
        )
        .run()
        {
            failures.push(StepError::run_from_process(APT, &error));
        }

        human!("apt-get upgrade -y");
        if let Err(error) = inside_command(
            options,
            CommandSpec::new("apt-get")
                .args(["upgrade", "-y"])
                .neutral_locale()
                .retry_transient_network(NETWORK_RETRIES),
        )
        .run()
        {
            failures.push(StepError::run_from_process(APT, &error));
        }
    }

    /// After a successful upgrade, records the services that need a restart
    /// (reported by `needrestart`) and whether a restart is required (reported
    /// by the reboot marker file or by `needrestart` listing services).
    fn services_needing_restart(&self) {
        let mut needed = restart_needed_at(Path::new(REBOOT_REQUIRED));
        human!("needrestart");
        if let Ok(output) = CommandSpec::new(NEEDRESTART).run() {
            let services = parse_needrestart_services(output.stdout());
            needed = needed || !services.is_empty();
            for service in services {
                services_needing_restart::record(service);
            }
        }
        if needed {
            restart_needed::record();
        }
    }

    /// Stops running desktop applications whose packages are about to be
    /// upgraded, so the package manager does not replace files an app still
    /// has mapped. Applications that do not quit gracefully are force-killed
    /// after the stop timeout when `--force-kill` is set.
    #[cfg(target_os = "linux")]
    fn stop_desktop_apps(&self, options: &RunOptions) {
        let system = HostDesktopAppSystem;
        let apps = desktop_apps::find_running_apps(&system);
        if apps.is_empty() {
            return;
        }

        let policy = StopPolicy::from_env(options.force_kill());
        let (stopped, still_running) = desktop_apps::stop_apps(&system, &apps, &policy);

        for app in &stopped {
            desktop_apps::record_stopped_app(app.clone());
            human!("Stopped desktop application: {}", app.name());
        }

        for app in &still_running {
            if policy.force_kill() {
                human!(
                    "warning: {} did not stop before the upgrade, even after force-killing",
                    app.name()
                );
            } else {
                human!(
                    "warning: {} did not stop before the upgrade (use --force-kill to force)",
                    app.name()
                );
            }
        }
    }
}

impl Step for AptStep {
    fn id(&self) -> StepId {
        APT
    }

    fn display_name(&self) -> &str {
        "apt"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        #[cfg(target_os = "linux")]
        return Ok(detect_apt());
        #[cfg(target_os = "windows")]
        return Ok(detect_wsl());
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        return Ok(Detection::NotInstalled);
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("apt-get")
            .args(["-s", "upgrade"])
            .neutral_locale()
            .run()
            .map_err(|error| StepError::check_from_process(APT, &error))?;
        let items = parse_apt_simulated_upgrade(output.stdout());
        #[cfg(target_os = "linux")]
        {
            let names: Vec<String> = items.iter().map(|item| item.name().to_owned()).collect();
            let mut guard = self
                .last_check_packages
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *guard = names;
        }
        Ok(CheckReport::from_items(items))
    }

    /// Reports what is outdated inside the selected WSL distribution on
    /// Windows. Without a selected distribution the step reports up to date.
    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        #[cfg(target_os = "windows")]
        {
            if crate::wsl::selected_distro(options).is_none() {
                return Ok(CheckReport::up_to_date());
            }
            let output = inside_command(
                options,
                CommandSpec::new("apt-get")
                    .args(["-s", "upgrade"])
                    .neutral_locale(),
            )
            .run()
            .map_err(|error| StepError::check_from_process(APT, &error))?;
            let items = parse_apt_simulated_upgrade(output.stdout());
            Ok(CheckReport::from_items(items))
        }
        #[cfg(not(target_os = "windows"))]
        {
            // `check()` is read-only and runs natively on Linux; the run
            // options only affect the WSL-wrapped path on Windows.
            let _ = options;
            self.check()
        }
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        #[cfg(target_os = "windows")]
        {
            let Some(_distro) = crate::wsl::selected_distro(options) else {
                human!("apt: no WSL distribution selected; pass --wsl-distro <distro>");
                return Ok(());
            };
        }

        #[cfg(target_os = "linux")]
        self.stop_desktop_apps(options);

        #[cfg(target_os = "linux")]
        let docker_upgrading = {
            let packages = self
                .last_check_packages
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let engine = NativeDockerRuntime::new();
            engine.detect().is_installed()
                && packages
                    .iter()
                    .any(|name| DOCKER_PACKAGES.contains(&name.as_str()))
                && engine.is_running()
        };

        #[cfg(target_os = "linux")]
        if docker_upgrading {
            if let Err(error) = NativeDockerRuntime::new().stop() {
                return Err(StepError::run_from_process(APT, &error));
            }
        }

        let mut failures = Vec::new();
        self.upgrade(options, &mut failures);

        #[cfg(target_os = "linux")]
        if docker_upgrading {
            if let Err(error) = NativeDockerRuntime::new().start() {
                failures.push(StepError::run_from_process(APT, &error));
            }
        }

        if let Err(error) = cleanup::run(
            APT,
            "apt: autoremove",
            inside_command(
                options,
                CommandSpec::new("apt-get")
                    .args(["autoremove", "-y"])
                    .neutral_locale()
                    .retry_transient_network(NETWORK_RETRIES),
            ),
            options,
        ) {
            failures.push(error);
        }

        if let Err(error) = cleanup::run(
            APT,
            "apt: autoclean",
            inside_command(
                options,
                CommandSpec::new("apt-get")
                    .args(["autoclean"])
                    .neutral_locale()
                    .retry_transient_network(NETWORK_RETRIES),
            ),
            options,
        ) {
            failures.push(error);
        }

        if failures.is_empty() {
            self.services_needing_restart();
        }
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &APT_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `apt-get` binary is discoverable on `PATH`.
#[cfg(target_os = "linux")]
fn detect_apt() -> Detection {
    if command_exists("apt-get") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Whether the `wsl` binary is discoverable on `PATH` (Windows only).
///
/// apt is considered available on Windows whenever WSL is present; the actual
/// upgrade then runs inside the distribution selected with `--wsl-distro`.
#[cfg(target_os = "windows")]
fn detect_wsl() -> Detection {
    if command_exists("wsl") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Marker file that Debian/Ubuntu creates when an upgrade requires a reboot.
const REBOOT_REQUIRED: &str = "/var/run/reboot-required";
/// `needrestart` reports which running services need a restart after an apt
/// upgrade.
const NEEDRESTART: &str = "needrestart";

/// Whether a restart is required, based on the reboot marker file alone.
fn restart_needed_at(reboot_required: &Path) -> bool {
    reboot_required.exists()
}

/// Parses the output of `needrestart` into the names of services that need a
/// restart. Each non-empty, non-header line is treated as a service name.
fn parse_needrestart_services(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.contains("PID") && !line.contains("Service"))
        .filter(|line| {
            !line
                .chars()
                .all(|c| c == '-' || c == '+' || c == '|' || c.is_whitespace())
        })
        .map(ToOwned::to_owned)
        .collect()
}

/// Parses the output of `apt-get -s upgrade`.
///
/// Each package the simulator would install is printed as
/// `Inst <name> [<current>] (<candidate> ...)`; only the name and the two
/// versions are reported. Lines that do not match that shape are ignored.
fn parse_apt_simulated_upgrade(output: &str) -> Vec<OutdatedItem> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("Inst ")?;
            let (name, rest) = rest.split_once(char::is_whitespace)?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            let mut item = OutdatedItem::new(name);
            if let Some(current) = bracketed(rest) {
                item = item.with_current(current);
            }
            if let Some(candidate) =
                parenthesized(rest).and_then(|inside| inside.split_whitespace().next())
            {
                item = item.with_latest(candidate);
            }
            Some(item)
        })
        .collect()
}

fn bracketed(text: &str) -> Option<&str> {
    let start = text.find('[')?;
    let end = text[start..].find(']')?;
    Some(&text[start + 1..start + end])
}

fn parenthesized(text: &str) -> Option<&str> {
    let start = text.find('(')?;
    let end = text[start..].find(')')?;
    Some(&text[start + 1..start + end])
}

#[cfg(test)]
/// A unique temp path for the reboot-required marker, so parallel tests do not
/// collide and never touch the real `/var/run/reboot-required`.
#[cfg(target_os = "linux")]
fn temp_marker_path() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "freshen-apt-restart-{pid}-{id}",
        pid = std::process::id(),
        id = id
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
    dir.join("reboot-required")
}

#[cfg(test)]
mod tests {
    use super::parse_apt_simulated_upgrade;
    #[cfg(target_os = "linux")]
    use super::{restart_needed_at, temp_marker_path};

    #[test]
    fn parses_simulated_upgrade_lines() {
        let items = parse_apt_simulated_upgrade(
            "Reading package lists... Done\n\
             Building dependency tree... Done\n\
             Reading state information... Done\n\
             The following packages will be upgraded:\n\
               curl git\n\
             Inst curl [7.81.0] (7.88.0 stable [amd64])\n\
             Inst git [2.34.1] (2.40.1 jammy [amd64])\n\
             2 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "curl");
        assert_eq!(items[0].current(), Some("7.81.0"));
        assert_eq!(items[0].latest(), Some("7.88.0"));
        assert_eq!(items[1].name(), "git");
        assert_eq!(items[1].current(), Some("2.34.1"));
        assert_eq!(items[1].latest(), Some("2.40.1"));
    }

    #[test]
    fn ignores_non_install_lines() {
        let items = parse_apt_simulated_upgrade(
            "Reading package lists... Done\n\
             The following packages will be upgraded:\n\
               curl\n\
             0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.\n",
        );
        assert!(items.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn restart_needed_at_reports_restart_when_marker_file_exists() {
        let marker = temp_marker_path();
        std::fs::write(&marker, "")
            .unwrap_or_else(|error| panic!("failed to write marker file: {error}"));

        assert!(restart_needed_at(&marker));

        if let Some(parent) = marker.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}
