//! The AWS CLI v2 step: reports newer AWS CLI v2 versions for installs that are
//! not managed by Homebrew.
//!
//! `aws` is the command line tool of the AWS CLI. Homebrew-managed
//! installations are handled by the `brew` step, so this step only reports on
//! installations outside of Homebrew (for example the official installer, the
//! ZIP bundle or a manual download). It is report-only: it never upgrades the
//! AWS CLI itself.
//!
//! The step runs `aws --version` to learn the installed version and reports it
//! when a newer AWS CLI v2 version is known. AWS CLI v2 does not ship an update
//! check command, so freshen compares `aws --version` against `LATEST_VERSION`;
//! bump that constant when a new AWS CLI v2 release is published.

use std::path::Path;

use crate::homebrew::brew_installs;
use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The AWS CLI v2 step.
pub const AWS_CLI: StepId = "aws";

/// The latest AWS CLI v2 version known to freshen.
///
/// Bump this when a new AWS CLI v2 release is published so the check step can
/// report installations that are behind.
const LATEST_VERSION: &str = "2.16.0";

/// AWS CLI v2 runs on every platform freshen supports.
const AWS_CLI_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the AWS CLI v2 step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(AwsCliStep)
}

struct AwsCliStep;

impl AwsCliStep {
    /// Runs `aws --version` and returns its stdout.
    fn version(&self) -> Result<String, StepError> {
        CommandSpec::new("aws")
            .args(["--version"])
            .neutral_locale()
            .run()
            .map(|output| output.stdout().to_owned())
            .map_err(|error| StepError::check_from_process(AWS_CLI, &error))
    }

    /// Parses the AWS CLI v2 version from `aws --version` output.
    ///
    /// Returns `None` when the output is not AWS CLI v2 (for example AWS CLI
    /// v1, which reports `aws-cli/1.x`, or an error message).
    fn parse_version(output: &str) -> Option<String> {
        output
            .split_whitespace()
            .find(|word| word.starts_with("aws-cli/2."))
            .and_then(|word| word.strip_prefix("aws-cli/"))
            .map(ToOwned::to_owned)
    }

    /// Returns `true` when `aws_path` is not located inside a Homebrew
    /// installation, i.e. the AWS CLI is managed outside of `brew`.
    fn is_non_brew_install(aws_path: &Path) -> bool {
        !brew_installs()
            .iter()
            .any(|install| aws_path.starts_with(&install.prefix))
    }

    /// Reports a newer AWS CLI v2 version without changing anything.
    fn report(&self) -> Result<CheckReport, StepError> {
        let Ok(output) = self.version() else {
            return Ok(CheckReport::up_to_date());
        };
        let Some(current) = Self::parse_version(&output) else {
            return Ok(CheckReport::up_to_date());
        };
        let mut items = Vec::new();
        if version_newer(LATEST_VERSION, &current) {
            items.push(
                OutdatedItem::new("AWS CLI")
                    .with_current(&current)
                    .with_latest(LATEST_VERSION)
                    .with_reason("newer AWS CLI v2 version available"),
            );
        }
        Ok(CheckReport::from_items(items))
    }

    /// Report-only: never upgrades the AWS CLI.
    fn do_nothing(&self) {
        human!("aws: report only, nothing to upgrade");
    }
}

impl Step for AwsCliStep {
    fn id(&self) -> StepId {
        AWS_CLI
    }

    fn display_name(&self) -> &str {
        "AWS CLI"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        let Some(aws_path) = crate::executable::resolve("aws") else {
            return Ok(Detection::NotInstalled);
        };
        if !Self::is_non_brew_install(&aws_path) {
            return Ok(Detection::NotInstalled);
        }
        match self.version() {
            Ok(output) if Self::parse_version(&output).is_some() => Ok(Detection::Installed),
            _ => Ok(Detection::NotInstalled),
        }
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.report()
    }

    fn run(&self) -> Result<(), StepError> {
        self.do_nothing();
        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &AWS_CLI_PLATFORMS
    }
}

/// Whether `candidate` is a newer version than `current`.
fn version_newer(candidate: &str, current: &str) -> bool {
    version_key(candidate) > version_key(current)
}

/// A sortable key for a version string like `2.16.0`.
fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{version_newer, AwsCliStep};

    #[test]
    fn parse_version_reads_aws_cli_v2() {
        assert_eq!(
            AwsCliStep::parse_version(
                "aws-cli/2.15.34 Python/3.11.9 Darwin/23.4.0 source/x86_64 prompt/off"
            ),
            Some("2.15.34".to_owned())
        );
    }

    #[test]
    fn parse_version_ignores_aws_cli_v1() {
        assert_eq!(
            AwsCliStep::parse_version(
                "aws-cli/1.29.34 Python/3.11.9 Linux/5.15.0 source/x86_64 prompt/off"
            ),
            None
        );
    }

    #[test]
    fn parse_version_ignores_garbage() {
        assert_eq!(AwsCliStep::parse_version("command not found"), None);
        assert_eq!(AwsCliStep::parse_version(""), None);
    }

    #[test]
    fn version_newer_compare_major_minor_patch() {
        assert!(version_newer("2.16.0", "2.15.34"));
        assert!(!version_newer("2.16.0", "2.16.0"));
        assert!(!version_newer("2.15.34", "2.16.0"));
    }
}
