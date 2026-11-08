//! The tfenv step: reports newer Terraform versions available through tfenv
//! without switching to them.
//!
//! tfenv is a report-only step: `check()` lists the Terraform versions tfenv
//! knows about and reports any installed version for which a newer version is
//! available to install. `run()` does nothing — tfenv is never switched
//! automatically.

use crate::executable::command_exists;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The tfenv Terraform version manager step.
pub const TFENV: StepId = "tfenv";

/// tfenv runs on macOS and Linux, where the `tfenv` command is available.
const TFENV_PLATFORMS: [Platform; 2] = [Platform::Macos, Platform::Linux];

/// Registers the tfenv step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(TfenvStep)
}

struct TfenvStep;

impl Step for TfenvStep {
    fn id(&self) -> StepId {
        TFENV
    }

    fn display_name(&self) -> &str {
        "tfenv"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_tfenv())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let installed = installed_terraform_versions()?;
        let available = available_terraform_versions()?;
        let newest = newest_available(&available);
        let mut items = Vec::new();
        for version in &installed {
            if let Some(latest) = &newest {
                if version_newer(latest, version) {
                    items.push(
                        OutdatedItem::new("terraform")
                            .with_current(version.as_str())
                            .with_latest(latest.as_str())
                            .with_reason("newer Terraform version available"),
                    );
                }
            }
        }
        Ok(CheckReport::from_items(items))
    }

    fn run(&self) -> Result<(), StepError> {
        // tfenv is report-only: it never switches Terraform versions automatically.
        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &TFENV_PLATFORMS
    }

    fn is_user_level(&self) -> bool {
        true
    }
}

fn detect_tfenv() -> Detection {
    if command_exists("tfenv") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// The Terraform versions tfenv has installed, from `tfenv list`.
///
/// The active version is marked with a leading `* ` and may carry a
/// `(set by ...)` suffix.
fn installed_terraform_versions() -> Result<Vec<String>, StepError> {
    let output = CommandSpec::new("tfenv")
        .arg("list")
        .neutral_locale()
        .run()
        .map_err(|error| StepError::check_from_process(TFENV, &error))?;
    Ok(parse_tfenv_versions(output.stdout()))
}

/// The Terraform versions available to install, from `tfenv list-remote`.
fn available_terraform_versions() -> Result<Vec<String>, StepError> {
    let output = CommandSpec::new("tfenv")
        .args(["list-remote"])
        .neutral_locale()
        .run()
        .map_err(|error| StepError::check_from_process(TFENV, &error))?;
    Ok(parse_tfenv_available(output.stdout()))
}

fn parse_tfenv_versions(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let line = line.strip_prefix("* ").unwrap_or(line);
            let version = line.split_whitespace().next()?;
            let version = version.split(" (").next().unwrap_or(version);
            if version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                Some(version.to_owned())
            } else {
                None
            }
        })
        .collect()
}

fn parse_tfenv_available(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            if line.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                Some(line.to_owned())
            } else {
                None
            }
        })
        .collect()
}

/// The newest available Terraform version, or `None` when none could be parsed.
fn newest_available(available: &[String]) -> Option<String> {
    available
        .iter()
        .max_by_key(|version| version_key(version))
        .cloned()
}

/// Whether `candidate` is a newer version than `current`.
fn version_newer(candidate: &str, current: &str) -> bool {
    version_key(candidate) > version_key(current)
}

/// A sortable key for a version string like `1.5.7`.
fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tfenv_versions_marks_active() {
        let output = "  1.5.7\n* 1.6.1 (set by /Users/user/.terraform-version)\n";
        assert_eq!(
            parse_tfenv_versions(output),
            vec!["1.5.7".to_owned(), "1.6.1".to_owned()]
        );
    }

    #[test]
    fn parse_tfenv_available_handles_remote_versions() {
        let output = "1.5.7\n1.6.1\n1.7.0\n";
        assert_eq!(
            parse_tfenv_available(output),
            vec!["1.5.7".to_owned(), "1.6.1".to_owned(), "1.7.0".to_owned()]
        );
    }

    #[test]
    fn version_key_parses_major_minor_patch() {
        assert_eq!(version_key("1.5.7"), vec![1, 5, 7]);
        assert_eq!(version_key("1.0"), vec![1, 0]);
        assert_eq!(version_key("not-a-version"), Vec::<u64>::new());
    }

    #[test]
    fn version_newer_compare_major_minor_patch() {
        assert!(version_newer("1.7.0", "1.6.1"));
        assert!(version_newer("1.6.2", "1.6.1"));
        assert!(!version_newer("1.6.1", "1.7.0"));
        assert!(!version_newer("1.6.1", "1.6.1"));
    }

    #[test]
    fn newest_available_picks_the_highest() {
        assert_eq!(
            newest_available(&["1.5.7".to_owned(), "1.6.1".to_owned(), "1.7.0".to_owned(),]),
            Some("1.7.0".to_owned())
        );
        assert_eq!(newest_available(&[]), None);
    }

    #[test]
    fn detect_tfenv_not_installed_when_absent() {
        assert!(!detect_tfenv().is_installed());
    }
}
