//! Parity tests for the zypper step.
//!
//! Every test runs the built binary against a stub `zypper` executable and
//! asserts the exact command transcript of the `zypper refresh` + `zypper
//! update -y` block. No real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "zypper"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
#[cfg(target_os = "linux")]
fn zypper_refreshes_and_updates() {
    let tools = StubTools::new(&["zypper"]);
    tools.set_responses(
        "zypper",
        &[
            StubOutput::new(), // zypper refresh
            StubOutput::new(), // zypper update -y
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::zypper_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_zypper_refresh_still_attempts_the_update_and_fails_the_step() {
    let tools = StubTools::new(&["zypper"]);
    tools.set_responses(
        "zypper",
        &[
            StubOutput::new().with_status(1), // zypper refresh fails
            StubOutput::new(),                // zypper update -y still runs
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::zypper_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_zypper_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn check_reports_zypper_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["zypper"]);

    let output = tools
        .command()
        .args(["--check", "--only", "zypper"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
