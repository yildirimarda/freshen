//! Gating and logging for destructive cleanup commands.
//!
//! Commands that delete cached downloads, unused tool versions or dangling
//! images are never run unless the user explicitly enables them with
//! [`ENABLE_FLAG`]. When they do run, every line the command prints is echoed
//! and the reclaimed space is reported when the tool tells us about it.

use std::cell::RefCell;

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

thread_local! {
    static RECORDED: RefCell<Option<Vec<CleanupReport>>> = const { RefCell::new(None) };
}

/// Collects the cleanup reports produced on the current thread until it is
/// finished.
///
/// The runner installs a recorder around every step so the reports can be
/// attached to the step result. Outside a recorder, [`run`] still prints its
/// output but does not store anything. Keeping the recorder in a thread-local
/// means parallel runners (and parallel tests) never share one.
pub(crate) struct Recorder {
    previous: Option<Vec<CleanupReport>>,
}

impl Recorder {
    /// Starts recording cleanup reports on the current thread.
    pub(crate) fn start() -> Self {
        let previous = RECORDED.with(|recorded| recorded.replace(Some(Vec::new())));
        Self { previous }
    }

    /// Stops recording and returns the reports collected since [`start`].
    ///
    /// [`start`]: Recorder::start
    pub(crate) fn finish(self) -> Vec<CleanupReport> {
        RECORDED
            .with(|recorded| recorded.replace(self.previous))
            .unwrap_or_default()
    }
}

fn record(report: &CleanupReport) {
    RECORDED.with(|recorded| {
        if let Some(reports) = recorded.borrow_mut().as_mut() {
            reports.push(report.clone());
        }
    });
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
        return skipped(label);
    }

    run_enabled(step, label, spec)
}

/// Runs a destructive cleanup command only when its dry-run listing reports
/// something to remove.
///
/// The listing is the read-only form of the command (for example
/// `brew cleanup --dry-run --prune=all`). A listing that prints no removal
/// lines leaves the mutating command untouched and reports that there was
/// nothing to remove. With cleanup disabled the listing is not run either.
///
/// # Errors
///
/// Returns a step error when the listing or the command cannot be run or exits
/// unsuccessfully.
pub fn run_guarded(
    step: StepId,
    label: &str,
    listing: CommandSpec,
    spec: CommandSpec,
    options: &RunOptions,
) -> Result<Option<CleanupReport>, StepError> {
    if !options.cleanup_enabled() {
        return skipped(label);
    }

    let listing = listing
        .run()
        .map_err(|error| StepError::run_from_process(step, &error))?;
    if !listing_reports_removals(listing.stdout()) {
        println!("{label}: nothing to remove");
        return Ok(None);
    }

    run_enabled(step, label, spec)
}

fn skipped(label: &str) -> Result<Option<CleanupReport>, StepError> {
    println!("{label}: skipped destructive cleanup (pass {ENABLE_FLAG} to enable)");
    Ok(None)
}

fn run_enabled(
    step: StepId,
    label: &str,
    spec: CommandSpec,
) -> Result<Option<CleanupReport>, StepError> {
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

    let report = CleanupReport { removed, reclaimed };
    record(&report);

    Ok(Some(report))
}

/// Returns whether a dry-run cleanup listing shows work to do.
///
/// Homebrew uses `Would autoremove`, `Would remove` and `Would prune` for the
/// lines that describe removals. Anything else (including an empty listing) is
/// treated as nothing to remove, so a warning cannot trigger a mutation.
fn listing_reports_removals(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        let line = line.trim();
        line.contains("Would autoremove")
            || line.contains("Would remove")
            || line.contains("Would prune")
    })
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
    use super::{listing_reports_removals, parse_reclaimed_space, record, CleanupReport, Recorder};

    #[test]
    fn the_recorder_collects_reports_until_it_finishes() {
        let recorder = Recorder::start();
        record(&CleanupReport {
            removed: vec!["Removing: /tmp/foo (2.5MB)".to_owned()],
            reclaimed: Some("2.5MB".to_owned()),
        });

        let reports = recorder.finish();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].removed(), ["Removing: /tmp/foo (2.5MB)"]);
        assert_eq!(reports[0].reclaimed(), Some("2.5MB"));
    }

    #[test]
    fn recording_outside_a_recorder_is_harmless() {
        record(&CleanupReport::default());

        let recorder = Recorder::start();
        assert!(recorder.finish().is_empty());
    }

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

    #[test]
    fn recognises_homebrew_dry_run_removal_lines() {
        assert!(listing_reports_removals(
            "==> Would autoremove 1 unneeded formula:\nold-dependency\n"
        ));
        assert!(listing_reports_removals("Would remove: /tmp/old (1.2MB)\n"));
        assert!(listing_reports_removals("Would prune 2 files from: /tmp\n"));
    }

    #[test]
    fn warnings_and_empty_listings_do_not_count_as_work() {
        assert!(!listing_reports_removals(""));
        assert!(!listing_reports_removals("Warning: nothing to do\n"));
    }
}
