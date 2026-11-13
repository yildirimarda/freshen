//! macOS desktop notifications through Notification Center.
//!
//! At the end of a run, freshen posts a notification with the result summary
//! so the outcome is visible without watching the terminal. Sending a
//! notification never fails a run: when `osascript` is unavailable or the
//! user has not granted notification permission, the error is ignored and the
//! run finishes as usual.

use crate::runner::RunSummary;

#[cfg(any(test, target_os = "macos"))]
use crate::runner::format_duration;

/// Title shown in the notification.
#[cfg(any(test, target_os = "macos"))]
pub(crate) const TITLE: &str = "freshen";

/// Sends a desktop notification with the result of `summary`.
///
/// On macOS this posts to Notification Center through `osascript`; on every
/// other platform it does nothing. A failure to send is ignored so a run is
/// never aborted because of a notification.
pub(crate) fn send(summary: &RunSummary) {
    send_macos(summary);
}

#[cfg(target_os = "macos")]
fn send_macos(summary: &RunSummary) {
    let body = body(summary);
    let script = format!(
        "display notification {body} with title {title}",
        body = quoted(&body),
        title = quoted(TITLE),
    );
    // Fire and forget: a notification must never block the run or delay its
    // exit. Errors (for example a denied notification permission) are ignored.
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(not(target_os = "macos"))]
fn send_macos(_summary: &RunSummary) {
    // Desktop notifications are a macOS feature; every other platform is
    // silent here (see the `notify-send` and toast items in the plan).
}

/// The body of the notification: the run result as a single line.
///
/// Kept platform-neutral so it can be tested on every operating system.
#[cfg(any(test, target_os = "macos"))]
#[must_use]
pub(crate) fn body(summary: &RunSummary) -> String {
    let mut text = format!(
        "run finished: {} ok, {} planned, {} skipped, {} failed, {} timed out, {} interrupted ({})",
        summary.ok_count(),
        summary.planned_count(),
        summary.skipped_count(),
        summary.failed_count(),
        summary.timed_out_count(),
        summary.interrupted_count(),
        format_duration(summary.total_duration()),
    );
    if let Some(failure) = summary.failure() {
        text.push_str(&format!(" - {failure}"));
    }
    for failure in summary.failures() {
        if let Some(error) = failure.error() {
            text.push_str(&format!(" - {}: {}", failure.name(), error.message()));
        }
    }
    text
}

/// Quotes `text` as an AppleScript string literal.
#[cfg(target_os = "macos")]
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::{body, TITLE};
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::runner::{RunSummary, Runner};
    use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

    /// A step that reports itself as installed and then runs (or fails) on cue.
    struct ProbeStep {
        fail: bool,
        message: String,
    }

    impl ProbeStep {
        fn ok() -> Self {
            Self {
                fail: false,
                message: String::new(),
            }
        }

        fn fail(message: impl Into<String>) -> Self {
            Self {
                fail: true,
                message: message.into(),
            }
        }
    }

    impl Step for ProbeStep {
        fn id(&self) -> StepId {
            "probe"
        }

        fn display_name(&self) -> &str {
            "Probe"
        }

        fn detect(&self) -> Result<Detection, StepError> {
            Ok(Detection::Installed)
        }

        fn check(&self) -> Result<CheckReport, StepError> {
            Ok(CheckReport::from_items([OutdatedItem::new("jq")
                .with_current("1.7.1")
                .with_latest("1.8.0")]))
        }

        fn run(&self) -> Result<(), StepError> {
            if self.fail {
                Err(StepError::run("probe", &self.message))
            } else {
                Ok(())
            }
        }

        fn platforms(&self) -> &'static [Platform] {
            &Platform::ALL
        }
    }

    fn summary(steps: &[&dyn Step]) -> RunSummary {
        Runner::with_options(steps.iter().copied(), RunOptions::new()).run()
    }

    #[test]
    fn the_title_is_freshen() {
        assert_eq!(TITLE, "freshen");
    }

    #[test]
    fn the_body_reports_the_counts_of_every_status() {
        let ok = ProbeStep::ok();
        let failed = ProbeStep::fail("upgrade failed");
        let summary = summary(&[&ok, &failed]);

        let text = body(&summary);
        assert!(text.contains("run finished:"));
        assert!(text.contains("1 ok"));
        assert!(text.contains("1 failed"));
        assert!(text.contains("0 timed out"));
        assert!(text.contains("0 interrupted"));
        assert!(text.contains("Probe: upgrade failed"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quoted_text_becomes_an_applescript_string_literal() {
        assert_eq!(
            super::quoted(r#"say "hi" \\ now"#),
            r#""say \"hi\" \\\\ now""#
        );
    }

    #[test]
    fn the_body_of_a_successful_run_has_no_failures() {
        let ok = ProbeStep::ok();
        let summary = summary(&[&ok]);

        let text = body(&summary);
        assert!(text.contains("1 ok"));
        assert!(!text.contains(" - "));
    }

    #[test]
    fn the_body_reports_the_raw_failure_message() {
        let failed = ProbeStep::fail(r#"weird "name" \ fail"#);
        let summary = summary(&[&failed]);

        let text = body(&summary);
        assert!(text.contains(r#"Probe: weird "name" \ fail"#));
    }
}
