//! Parity tests for the apk step.
//!
//! Every test runs the built binary against a stub `apk` executable and
//! asserts the exact command transcript of the `apk update` + `apk upgrade`
//! block. No real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "apk"])
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
fn apk_updates_and_upgrades() {
    let tools = StubTools::new(&["apk"]);
    tools.set_responses(
        "apk",
        &[
            StubOutput::new(), // apk update
            StubOutput::new(), // apk upgrade
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::apk_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_apk_update_still_attempts_the_upgrade_and_fails_the_step() {
    let tools = StubTools::new(&["apk"]);
    tools.set_responses(
        "apk",
        &[
            StubOutput::new().with_status(1), // apk update fails
            StubOutput::new(),                // apk upgrade still runs
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::apk_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_apk_is_skipped_without_calling_anything() {
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
fn check_reports_apk_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["apk"]);

    let output = tools
        .command()
        .args(["--check", "--only", "apk"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
