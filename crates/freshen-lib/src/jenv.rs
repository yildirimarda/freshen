//! The jenv step: reports newer Java versions available through jenv without
//! switching to them.
//!
//! jenv is a report-only step: `check()` lists the Java versions jenv knows
//! about and reports any installed version for which a newer version is
//! available to install. `run()` does nothing — jenv is never switched
//! automatically.

use crate::executable::command_exists;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The jenv Java version manager step.
pub const JENV: StepId = "jenv";

/// jenv runs on macOS and Linux, where the `jenv` command is available.
const JENV_PLATFORMS: [Platform; 2] = [Platform::Macos, Platform::Linux];

/// Registers the jenv step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(JenvStep)
}

struct JenvStep;

impl Step for JenvStep {
    fn id(&self) -> StepId {
        JENV
    }

    fn display_name(&self) -> &str {
        "jenv"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_jenv())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let installed = installed_java_versions()?;
        let available = available_java_versions()?;
        let newest = newest_available(&available);
        let mut items = Vec::new();
        for version in &installed {
            if let Some(latest) = &newest {
                if version_newer(latest, version) {
                    items.push(
                        OutdatedItem::new("java")
                            .with_current(version.as_str())
                            .with_latest(latest.as_str())
                            .with_reason("newer Java version available"),
                    );
                }
            }
        }
        Ok(CheckReport::from_items(items))
    }

    fn run(&self) -> Result<(), StepError> {
        // jenv is report-only: it never switches Java versions automatically.
        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &JENV_PLATFORMS
    }

    fn is_user_level(&self) -> bool {
        true
    }
}

fn detect_jenv() -> Detection {
    if command_exists("jenv") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// The Java versions jenv has installed, from `jenv versions`.
///
/// The active version is marked with a leading `* ` and may carry a
/// `(set by ...)` suffix. The `system` entry is not a jenv-managed version
/// and is skipped.
fn installed_java_versions() -> Result<Vec<String>, StepError> {
    let output = CommandSpec::new("jenv")
        .arg("versions")
        .neutral_locale()
        .run()
        .map_err(|error| StepError::check_from_process(JENV, &error))?;
    Ok(parse_jenv_versions(output.stdout()))
}

/// The Java versions available to install, from `jenv install -l`.
fn available_java_versions() -> Result<Vec<String>, StepError> {
    let output = CommandSpec::new("jenv")
        .args(["install", "-l"])
        .neutral_locale()
        .run()
        .map_err(|error| StepError::check_from_process(JENV, &error))?;
    Ok(parse_jenv_available(output.stdout()))
}

fn parse_jenv_versions(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line == "system" {
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

fn parse_jenv_available(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            // `Vendor:VERSION` or just `VERSION`.
            let version = line.rsplit(':').next().unwrap_or(line).trim();
            if version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                Some(version.to_owned())
            } else {
                None
            }
        })
        .collect()
}

/// The newest available Java version, or `None` when none could be parsed.
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

/// A sortable key for a version string like `11.0.2` or `1.8`.
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
    fn parse_jenv_versions_skips_system_and_marks_active() {
        let output = "  system\n  11.0.2\n* 21.0.2 (set by /Users/user/.java-version)\n";
        assert_eq!(
            parse_jenv_versions(output),
            vec!["11.0.2".to_owned(), "21.0.2".to_owned()]
        );
    }

    #[test]
    fn parse_jenv_available_handles_vendor_prefixes() {
        let output = "  Adoptium:11.0.22\n  Adoptium:17.0.12\n  21.0.3\n";
        assert_eq!(
            parse_jenv_available(output),
            vec![
                "11.0.22".to_owned(),
                "17.0.12".to_owned(),
                "21.0.3".to_owned()
            ]
        );
    }

    #[test]
    fn version_key_parses_major_minor_patch() {
        assert_eq!(version_key("11.0.2"), vec![11, 0, 2]);
        assert_eq!(version_key("1.8"), vec![1, 8]);
        assert_eq!(version_key("not-a-version"), Vec::<u64>::new());
    }

    #[test]
    fn version_newer_compare_major_minor_patch() {
        assert!(version_newer("22.0.1", "21.0.2"));
        assert!(version_newer("17.0.12", "17.0.2"));
        assert!(!version_newer("21.0.2", "22.0.1"));
        assert!(!version_newer("11.0.2", "11.0.2"));
    }

    #[test]
    fn newest_available_picks_the_highest() {
        assert_eq!(
            newest_available(&[
                "11.0.22".to_owned(),
                "17.0.12".to_owned(),
                "21.0.3".to_owned(),
            ]),
            Some("21.0.3".to_owned())
        );
        assert_eq!(newest_available(&[]), None);
    }

    #[test]
    fn detect_jenv_not_installed_when_absent() {
        assert!(!detect_jenv().is_installed());
    }
}
