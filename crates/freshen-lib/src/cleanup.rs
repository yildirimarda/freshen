//! Gating and logging for destructive cleanup commands.
//!
//! Commands that delete cached downloads, unused tool versions or dangling
//! images are never run unless the user explicitly enables them with
//! [`ENABLE_FLAG`]. When they do run, every line the command prints is echoed
//! and the reclaimed space is reported when the tool tells us about it.

use crate::options::RunOptions;
use crate::process::CommandSpec;
use crate::step::{StepError, StepId};

/// Command-line flag that enables destructive cleanup.
pub const ENABLE_FLAG: &str = "--cleanup";

/// What one destructive cleanup command removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    removed: Vec<String>,
    reclaimed: Option<String>,
}

impl CleanupReport {
    /// Lines the cleanup command printed about what it removed.
    #[must_use]
    pub fn removed(&self) -> &[String] {
        &self.removed
    }

    /// Space the tool reported as reclaimed, e.g. `"1.2GB"`.
    #[must_use]
    pub fn reclaimed(&self) -> Option<&str> {
        self.reclaimed.as_deref()
    }
}

/// Runs a destructive cleanup command when cleanup is enabled.
///
/// With cleanup disabled this only prints why the command was skipped and
/// returns `None`. With cleanup enabled every output line is echoed, so the
/// run log records exactly what was removed, and the report records the
/// reclaimed space when the command mentions it.
///
/// # Errors
///
/// Returns a step error when the command cannot be run or exits unsuccessfully.
pub fn run(
    step: StepId,
    label: &str,
    spec: CommandSpec,
    options: &RunOptions,
) -> Result<Option<CleanupReport>, StepError> {
    if !options.cleanup_enabled() {
        println!("{label}: skipped destructive cleanup (pass {ENABLE_FLAG} to enable)");
        return Ok(None);
    }

    println!("{label}:");
    let mut removed = Vec::new();
    let output = spec
        .run_streaming(|_, line| {
            if !line.is_empty() {
                println!("  {line}");
                removed.push(line.to_owned());
            }
        })
        .map_err(|error| StepError::run_from_process(step, &error))?;

    let mut text = String::new();
    text.push_str(output.stdout());
    text.push('\n');
    text.push_str(output.stderr());

    let reclaimed = parse_reclaimed_space(&text);
    match &reclaimed {
        Some(space) => println!("{label}: reclaimed space: {space}"),
        None => println!("{label}: reclaimed space: not reported by the tool"),
    }

    Ok(Some(CleanupReport { removed, reclaimed }))
}

/// Extracts the reclaimed space from a cleanup command's output.
///
/// Understands the formats Homebrew, Docker and uv use today, plus a generic
/// `reclaimed ...` line. Returns `None` when the command does not report it.
#[must_use]
pub fn parse_reclaimed_space(output: &str) -> Option<String> {
    for line in output.lines().rev() {
        let lower = line.to_ascii_lowercase();

        for marker in [
            "total reclaimed space:",
            "reclaimed space:",
            "space reclaimed:",
        ] {
            if let Some(rest) = text_after(line, marker) {
                if let Some(size) = first_size(rest) {
                    return Some(size);
                }
            }
        }

        if let Some(rest) = text_after(line, "freed approximately ") {
            let end = rest
                .to_ascii_lowercase()
                .find(" of disk space")
                .unwrap_or(rest.len());
            if let Some(size) = first_size(&rest[..end]) {
                return Some(size);
            }
        }

        if lower.contains("removed ") {
            if let Some(size) = parenthesized_size(line) {
                return Some(size);
            }
        }

        if let Some(rest) = text_after(line, "reclaimed ") {
            if let Some(size) = first_size(rest) {
                return Some(size);
            }
        }
    }

    None
}

fn text_after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let lower = line.to_ascii_lowercase();
    let index = lower.find(marker)? + marker.len();
    Some(&line[index..])
}

fn first_size(text: &str) -> Option<String> {
    let token = text
        .trim_start_matches([' ', '\t', ':', '='])
        .split_whitespace()
        .next()?
        .trim_end_matches(['.', ',', ';']);
    let mut chars = token.chars();
    chars.next()?.is_ascii_digit().then(|| token.to_owned())
}

fn parenthesized_size(line: &str) -> Option<String> {
    let start = line.rfind('(')?;
    let end = line[start..].find(')')? + start;
    let inner = line[start + 1..end].trim();
    let mut chars = inner.chars();
    chars.next()?.is_ascii_digit().then(|| inner.to_owned())
}

#[cfg(test)]
mod tests {
    use super::parse_reclaimed_space;

    #[test]
    fn parses_docker_total_reclaimed_space() {
        let output = "Deleted Images:\nuntagged: example:latest\nTotal reclaimed space: 3.5GB\n";
        assert_eq!(parse_reclaimed_space(output).as_deref(), Some("3.5GB"));
    }

    #[test]
    fn parses_homebrew_freed_space() {
        let output = "==> This operation has freed approximately 1.2GB of disk space.\n";
        assert_eq!(parse_reclaimed_space(output).as_deref(), Some("1.2GB"));
    }

    #[test]
    fn parses_uv_removed_files() {
        let output = "Removed 3 files (1.5 MiB)\n";
        assert_eq!(parse_reclaimed_space(output).as_deref(), Some("1.5 MiB"));
    }

    #[test]
    fn returns_none_when_the_tool_reports_no_space() {
        assert_eq!(parse_reclaimed_space("pruned node@20.0.0\n"), None);
        assert_eq!(parse_reclaimed_space(""), None);
    }
}
