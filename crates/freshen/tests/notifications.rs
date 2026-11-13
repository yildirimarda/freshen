//! End-to-end checks for macOS desktop notifications.
//!
//! The notification is posted to Notification Center through `osascript`, so
//! the test stubs `osascript` and asserts the recorded invocation carries the
//! result summary of the run.

#![cfg(feature = "test-harness")]
#![cfg(target_os = "macos")]

mod support;

use std::process::Output;

use support::StubTools;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_end_of_a_run_posts_a_notification_with_the_result_summary() {
    let tools = StubTools::new(&["osascript", "probe"]);
    tools.set_responses("osascript", &[support::StubOutput::new()]);

    let output = tools
        .command()
        .env("FRESHEN_TEST_STEPS", "probe")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let calls = tools.calls();
    assert!(
        calls.iter().any(|call| {
            call.starts_with("osascript -e ")
                && call.contains("display notification")
                && call.contains("with title \"freshen\"")
                && call.contains("run finished: 1 ok")
        }),
        "no notification call in {calls:?}"
    );

    let text = stdout(&output);
    assert!(text.contains("1 ok"), "summary: {text}");
}
