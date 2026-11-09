//! Parity tests for the antidote step.
//!
//! Every test runs the built binary against a stub `antidote` executable and
//! asserts the exact command transcript of the `antidote update` block. No
//! real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "antidote"])
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
fn antidote_updates_plugins() {
    let tools = StubTools::new(&["antidote"]);
    tools.set_responses(
        "antidote",
        &[
            StubOutput::new(), // antidote update
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::antidote_transcript());
}

#[test]
fn antidote_skips_when_antidote_is_absent() {
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
fn a_failing_antidote_update_fails_the_step() {
    let tools = StubTools::new(&["antidote"]);
    tools.set_responses(
        "antidote",
        &[StubOutput::new().with_status(1)], // antidote update fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::antidote_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_antidote_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["antidote"]);

    let output = tools
        .command()
        .args(["--check", "--only", "antidote"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
