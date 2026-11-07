//! The pip step: a report-only `pip list --user --outdated` check.
//!
//! The step reports which user-installed Python packages are outdated. It
//! never installs or upgrades anything: [`Step::run`] is a no-op, so the step
//! is safe to register on every machine and a run of it never changes the
//! system. pip manages user-level packages, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The pip package manager step.
pub const PIP: StepId = "pip";

/// pip runs on every platform freshen supports.
const PIP_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the pip step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(PipStep)
}

struct PipStep;

impl PipStep {
    /// Runs `pip list --user --outdated` and reports what it found.
    ///
    /// The command is read-only: it never installs or upgrades a package.
    fn check_packages(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("pip")
            .args(["list", "--user", "--outdated"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(PIP, &error))?;
        Ok(CheckReport::from_items(parse_pip_outdated(output.stdout())))
    }

    /// Report-only: never installs or upgrades any package.
    fn do_nothing(&self) {
        human!("pip: report only, nothing to upgrade");
    }
}

impl Step for PipStep {
    fn id(&self) -> StepId {
        PIP
    }

    fn display_name(&self) -> &str {
        "pip"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_pip())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_packages()
    }

    fn run(&self) -> Result<(), StepError> {
        self.do_nothing();
        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &PIP_PLATFORMS
    }

    fn is_user_level(&self) -> bool {
        true
    }
}

/// Whether the `pip` binary is discoverable on `PATH`.
fn detect_pip() -> Detection {
    if crate::executable::command_exists("pip") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses `pip list --user --outdated` output.
///
/// Every outdated package is one line: `NAME VERSION LATEST [TYPE]`. Header
/// and separator lines are ignored, so the parser also works when pip omits
/// the optional `Type` column.
fn parse_pip_outdated(output: &str) -> Vec<OutdatedItem> {
    output.lines().filter_map(parse_pip_outdated_line).collect()
}

fn parse_pip_outdated_line(line: &str) -> Option<OutdatedItem> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("Package") || line.starts_with('-') {
        return None;
    }
    let mut parts = line.split_whitespace();
    let name = parts.next()?;
    let current = parts.next()?;
    let latest = parts.next()?;
    Some(
        OutdatedItem::new(name)
            .with_current(current)
            .with_latest(latest),
    )
}

#[cfg(test)]
mod tests {
    use super::parse_pip_outdated;

    #[test]
    fn parses_outdated_packages_with_their_versions() {
        let items = parse_pip_outdated(
            "Package    Version Latest   Type\n\
             ---------- ------- ------ -----\n\
             black      23.1.0  26.10.1  wheel\n\
             ruff       0.6.0   0.7.0    wheel\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "black");
        assert_eq!(items[0].current(), Some("23.1.0"));
        assert_eq!(items[0].latest(), Some("26.10.1"));
        assert_eq!(items[1].name(), "ruff");
        assert_eq!(items[1].current(), Some("0.6.0"));
        assert_eq!(items[1].latest(), Some("0.7.0"));
    }

    #[test]
    fn parses_outdated_packages_without_the_type_column() {
        let items = parse_pip_outdated(
            "Package Version Latest\n\
             ------- ------- ------\n\
             black 23.1.0 26.10.1\n",
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "black");
        assert_eq!(items[0].current(), Some("23.1.0"));
        assert_eq!(items[0].latest(), Some("26.10.1"));
    }

    #[test]
    fn ignores_output_without_outdated_packages() {
        assert!(parse_pip_outdated(
            "Package    Version Latest   Type\n---------- ------- ------ -----\n"
        )
        .is_empty());
        assert!(parse_pip_outdated("").is_empty());
        assert!(parse_pip_outdated("   \n").is_empty());
    }
}
