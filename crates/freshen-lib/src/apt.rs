//! The apt step: `apt-get update` followed by `apt-get upgrade -y`, and
//! `apt-get autoremove -y` plus `apt-get autoclean` when `--cleanup` is set.
//!
//! The zsh implementation runs `apt-get update` and `apt-get upgrade -y`
//! whenever `apt-get` is available. The step is skipped entirely when apt is
//! absent, so it is safe to register on every machine; on macOS and Windows
//! it is never detected.

use crate::cleanup;
use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::restart_needed;
use crate::services_needing_restart;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};
use std::path::Path;

/// The apt package manager step.
pub const APT: StepId = "apt";

/// apt only runs on Linux.
const APT_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the apt step, after the Homebrew steps.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(AptStep)
}

struct AptStep;

impl AptStep {
    /// Runs `apt-get update` and then `apt-get upgrade -y`, collecting the
    /// first failure so both commands are attempted.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("apt-get update");
        if let Err(error) = CommandSpec::new("apt-get")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(APT, &error));
        }

        human!("apt-get upgrade -y");
        if let Err(error) = CommandSpec::new("apt-get")
            .args(["upgrade", "-y"])
            .retry_transient_network(NETWORK_RETRIES)
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
}

impl Step for AptStep {
    fn id(&self) -> StepId {
        APT
    }

    fn display_name(&self) -> &str {
        "apt"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_apt())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("apt-get")
            .args(["-s", "upgrade"])
            .neutral_locale()
            .run()
            .map_err(|error| StepError::check_from_process(APT, &error))?;
        Ok(CheckReport::from_items(parse_apt_simulated_upgrade(
            output.stdout(),
        )))
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);

        if let Err(error) = cleanup::run(
            APT,
            "apt: autoremove",
            CommandSpec::new("apt-get")
                .args(["autoremove", "-y"])
                .retry_transient_network(NETWORK_RETRIES),
            options,
        ) {
            failures.push(error);
        }

        if let Err(error) = cleanup::run(
            APT,
            "apt: autoclean",
            CommandSpec::new("apt-get")
                .args(["autoclean"])
                .retry_transient_network(NETWORK_RETRIES),
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
fn detect_apt() -> Detection {
    if command_exists("apt-get") {
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
    use super::{parse_apt_simulated_upgrade, restart_needed_at, temp_marker_path};

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
