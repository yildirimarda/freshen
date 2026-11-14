//! Desktop notifications at the end of a run.
//!
//! At the end of a run, freshen posts a notification with the result summary
//! so the outcome is visible without watching the terminal. On macOS this uses
//! `osascript` (Notification Center); on Linux it uses `notify-send`. Sending
//! a notification never fails a run: when the notifier is unavailable or the
//! user has not granted permission, the error is ignored and the run finishes
//! as usual.

use crate::check::{CheckStatus, CheckSummary};
use crate::runner::RunSummary;

#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
use crate::runner::format_duration;

/// Title shown in the notification.
#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) const TITLE: &str = "freshen";

/// Sends a desktop notification with the result of `summary`.
///
/// On macOS this posts to Notification Center through `osascript`; on Linux it
/// uses `notify-send`; on Windows it shows a toast through PowerShell; on every
/// other platform it does nothing. A failure to send is ignored so a run is
/// never aborted because of a notification.
pub(crate) fn send(summary: &RunSummary) {
    let body = body(summary);
    send_macos(&body);
    send_linux(&body);
    send_windows(&body);
}

/// Sends a desktop notification when a `--check` run found pending updates.
///
/// Does nothing when the check found no updates. A failure to send is ignored
/// so a run is never aborted because of a notification.
pub(crate) fn send_check(summary: &CheckSummary) {
    if !summary.has_updates() {
        return;
    }
    let body = body_for_check(summary);
    send_macos(&body);
    send_linux(&body);
    send_windows(&body);
}

#[cfg(target_os = "macos")]
fn send_macos(body: &str) {
    let script = format!(
        "display notification {body} with title {title}",
        body = quoted(body),
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

#[cfg(target_os = "linux")]
fn send_linux(body: &str) {
    // Fire and forget: a notification must never block the run or delay its
    // exit. Errors (for example `notify-send` not being installed or the
    // desktop session not supporting notifications) are ignored.
    let _ = std::process::Command::new("notify-send")
        .arg(TITLE)
        .arg(body)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(not(target_os = "macos"))]
fn send_macos(_body: &str) {
    // Desktop notifications are a macOS feature; every other platform is
    // silent here (see `send_linux` for Linux).
}

#[cfg(not(target_os = "linux"))]
fn send_linux(_body: &str) {
    // Desktop notifications are a Linux feature; every other platform is
    // silent here (see `send_macos` for macOS).
}

/// Shows a Windows toast notification with the result of `summary`.
///
/// The toast is posted through PowerShell using the Windows notification API.
/// Fire and forget: a notification must never block the run or delay its exit,
/// and a failure to send is ignored.
#[cfg(windows)]
fn send_windows(body: &str) {
    let script = windows::toast_script(TITLE, body);
    let Some(powershell) = crate::shell::windows::powershell() else {
        return;
    };
    let Some(path) = crate::executable::resolve(powershell.command()) else {
        return;
    };
    let _ = std::process::Command::new(path)
        .args(crate::shell::windows::invocation_args(&script, true))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(not(windows))]
fn send_windows(_body: &str) {
    // Windows toast notifications are a Windows feature; every other platform
    // is silent here.
}

/// Builds the PowerShell command that shows a Windows toast notification.
///
/// `title` and `body` are embedded in single-quoted PowerShell strings with a
/// literal `'` escaped as `''`, so no external quoting crate is needed.
#[cfg(windows)]
mod windows {
    #[must_use]
    pub(crate) fn toast_script(title: &str, body: &str) -> String {
        let title = title.replace('\'', "''");
        let body = body.replace('\'', "''");
        format!(
            r#"$t=[Windows.UI.Notifications.ToastTemplateType]::ToastText02;$x=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent($t);$e=$x.GetElementsByTagName("text");$e.Item(0).AppendChild($x.CreateTextNode('{title}'));$e.Item(1).AppendChild($x.CreateTextNode('{body}'));$n=[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{title}');$n.Show([Windows.UI.Notifications.ToastNotification]::new($x))"#,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::super::TITLE;
        use super::toast_script;

        #[test]
        fn toast_script_carries_the_title_and_summary_body() {
            let summary_body =
                "run finished: 1 ok, 0 planned, 0 skipped, 0 failed, 0 timed out, 0 interrupted (0s)";
            let script = toast_script(TITLE, summary_body);
            assert!(script.contains(TITLE));
            assert!(script.contains(summary_body));
            assert!(script.contains("ToastNotificationManager"));
            assert!(script.contains("CreateToastNotifier"));
        }
    }
}

/// The body of the notification: the run result as a single line.
///
/// Kept platform-neutral so it can be tested on every operating system.
#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
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

/// The body of a `--check` notification: the pending updates as a single line.
///
/// Lists each outdated step with how many items it reported, so a scheduled
/// run can tell the user what needs upgrading without opening a terminal.
/// Kept platform-neutral so it can be tested on every operating system.
#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
#[must_use]
pub(crate) fn body_for_check(summary: &CheckSummary) -> String {
    let mut parts: Vec<String> = Vec::new();
    for result in summary.steps() {
        if result.status() == CheckStatus::Outdated {
            let count = result.report().pending();
            parts.push(format!("{} ({} update(s))", result.name(), count));
        }
    }
    format!(
        "{} update(s) pending across {} step(s): {}",
        summary.pending(),
        summary.outdated_steps(),
        parts.join(", ")
    )
}

/// Quotes `text` as an AppleScript string literal.
#[cfg(target_os = "macos")]
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::{body, body_for_check, TITLE};
    use crate::check::{CheckResult, CheckSummary};
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::runner::{RunSummary, Runner};
    use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};
    use std::time::Duration;

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

    fn check_step(count: usize) -> CheckResult {
        CheckResult::outdated(
            &ProbeStep::ok(),
            CheckReport::from_items((0..count).map(|i| OutdatedItem::new(format!("pkg-{i}")))),
            Duration::ZERO,
        )
    }

    fn check_summary(results: Vec<CheckResult>) -> CheckSummary {
        CheckSummary::from_results(results)
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

    #[test]
    fn check_body_reports_pending_updates_across_outdated_steps() {
        let summary = check_summary(vec![check_step(2), check_step(1)]);

        let text = body_for_check(&summary);
        assert!(text.contains("3 update(s) pending across 2 step(s)"));
        assert!(text.contains("Probe (2 update(s))"));
        assert!(text.contains("Probe (1 update(s))"));
    }

    #[test]
    fn check_body_reports_zero_when_no_step_is_outdated() {
        let up_to_date = CheckResult::up_to_date(&ProbeStep::ok(), Duration::ZERO);
        let summary = check_summary(vec![up_to_date]);

        let text = body_for_check(&summary);
        assert_eq!(text, "0 update(s) pending across 0 step(s): ");
    }
}
