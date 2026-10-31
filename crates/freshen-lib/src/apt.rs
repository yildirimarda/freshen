//! The apt step: `apt-get update` followed by `apt-get upgrade -y`.
//!
//! The zsh implementation runs `apt-get update` and `apt-get upgrade -y`
//! whenever `apt-get` is available. The step is skipped entirely when apt is
//! absent, so it is safe to register on every machine; on macOS and Windows
//! it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

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

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
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
mod tests {
    use super::parse_apt_simulated_upgrade;

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
}
