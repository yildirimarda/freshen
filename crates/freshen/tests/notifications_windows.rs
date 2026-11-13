//! End-to-end checks for Windows toast notifications.
//!
//! The toast is posted through PowerShell, so the test stubs `pwsh` and
//! asserts the recorded invocation carries the result summary of the run. No
//! real toast notification is shown.

#![cfg(all(feature = "test-harness", target_os = "windows"))]

mod support;

use std::{
    thread,
    time::{Duration, Instant},
};

use support::StubTools;

fn is_toast(call: &str) -> bool {
    call.starts_with("pwsh ")
        && call.contains("ToastNotificationManager")
        && call.contains("run finished: 1 ok")
}

#[test]
fn the_end_of_a_run_posts_a_toast_notification_with_the_result_summary() {
    let tools = StubTools::new(&["pwsh", "probe"]);
    tools.set_responses("pwsh", &[support::StubOutput::new()]);

    let output = tools
        .command()
        .env("FRESHEN_TEST_STEPS", "probe")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let notified = loop {
        if tools.calls().iter().any(|call| is_toast(call)) {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(25));
    };
    assert!(notified, "no toast call in {:?}", tools.calls());

    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("1 ok"), "summary: {text}");
}
