//! The asdf step: `asdf plugin update --all` plus a read-only report of the
//! tools pinned in `.tool-versions` that have newer versions available.
//!
//! The step runs `asdf plugin update --all` whenever `asdf` is available. It
//! is skipped entirely when asdf is absent, so it is safe to register on every
//! machine. asdf manages user-level tools, so it must never run as root.
//!
//! The read-only `check()` parses `.tool-versions` (searched in the current
//! directory and its parents, then `~/.tool-versions`) and queries `asdf
//! latest <tool>` for every pinned tool, reporting those whose latest version
//! differs from the pinned one. Nothing is ever changed: the versions pinned
//! in `.tool-versions` are left untouched.

use std::fs;
use std::path::{Path, PathBuf};

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The asdf plugin step.
pub const ASDF: StepId = "asdf";

/// asdf runs on every platform freshen supports.
const ASDF_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the asdf step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(AsdfStep)
}

struct AsdfStep;

impl AsdfStep {
    /// Runs `asdf plugin update --all`.
    fn update(&self, failures: &mut Vec<StepError>) {
        human!("asdf: plugin update --all");
        if let Err(error) = CommandSpec::new("asdf")
            .args(["plugin", "update", "--all"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ASDF, &error));
        }
    }
}

impl Step for AsdfStep {
    fn id(&self) -> StepId {
        ASDF
    }

    fn display_name(&self) -> &str {
        "asdf"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_asdf())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        let Some(pinned) = pinned_tool_versions() else {
            return Ok(CheckReport::up_to_date());
        };
        let mut items = Vec::new();
        for (tool, version) in &pinned {
            let Some(latest) = latest_available_version(tool) else {
                continue;
            };
            if latest.is_empty() || latest == *version {
                continue;
            }
            items.push(
                OutdatedItem::new(tool)
                    .with_current(version)
                    .with_latest(&latest)
                    .with_reason("newer version available"),
            );
        }
        let report = CheckReport::from_items(items);
        Ok(report.filtered(|name| options.filters().packages().includes(name)))
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &ASDF_PLATFORMS
    }

    fn is_user_level(&self) -> bool {
        true
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `asdf` binary is discoverable on `PATH`.
fn detect_asdf() -> Detection {
    if crate::executable::command_exists("asdf") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// The versions pinned in `.tool-versions`, in file order.
///
/// asdf looks for `.tool-versions` in the current directory and its parents,
/// then in `~/.tool-versions`. The first file found wins. Returns `None` when
/// no file exists, which the check treats as "nothing pinned, up to date".
fn pinned_tool_versions() -> Option<Vec<(String, String)>> {
    let path = find_tool_versions()?;
    let content = fs::read_to_string(&path).ok()?;
    Some(parse_tool_versions(&content))
}

/// Locates a `.tool-versions` file the way asdf does: the current directory
/// and its ancestors first, then `~/.tool-versions`.
fn find_tool_versions() -> Option<PathBuf> {
    if let Ok(dir) = std::env::current_dir() {
        let mut current = dir;
        loop {
            let candidate = current.join(".tool-versions");
            if candidate.is_file() {
                return Some(candidate);
            }
            if !current.pop() {
                break;
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let candidate = Path::new(&home).join(".tool-versions");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Parses the `.tool-versions` format: one `tool version` pair per line.
///
/// Blank lines and comment lines (starting with `#`) are ignored. Only the
/// first two whitespace-separated fields of each line are used.
fn parse_tool_versions(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut fields = line.split_whitespace();
            let tool = fields.next()?;
            let version = fields.next()?;
            (!tool.is_empty() && !version.is_empty())
                .then_some((tool.to_owned(), version.to_owned()))
        })
        .collect()
}

/// The latest version available for `tool` from `asdf latest <tool>`.
///
/// Best effort: a missing plugin, a network error or unparseable output
/// yields `None` and never fails the step.
fn latest_available_version(tool: &str) -> Option<String> {
    let Ok(output) = CommandSpec::new("asdf")
        .args(["latest", tool])
        .neutral_locale()
        .retry_transient_network(NETWORK_RETRIES)
        .run()
    else {
        return None;
    };
    let latest = output.stdout().trim().to_owned();
    (!latest.is_empty()).then_some(latest)
}

#[cfg(test)]
mod tests {
    use crate::step::Detection;

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }
}
