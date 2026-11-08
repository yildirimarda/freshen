//! Parity tests for the Composer step.
//!
//! Every test runs the built binary against a stub `composer` executable and
//! asserts the exact command transcript of the `composer self-update` +
//! `composer global update` block. No real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "composer", "--cleanup"])
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
fn composer_self_updates_and_updates_global_packages() {
    let tools = StubTools::new(&["composer"]);
    tools.set_responses(
        "composer",
        &[
            StubOutput::new(), // composer self-update
            StubOutput::new(), // composer global update
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::composer_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
}

#[test]
fn composer_skips_when_composer_is_absent() {
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
fn a_failing_composer_global_update_fails_the_step() {
    let tools = StubTools::new(&["composer"]);
    tools.set_responses(
        "composer",
        &[
            StubOutput::new(),                // composer self-update
            StubOutput::new().with_status(1), // composer global update fails
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::composer_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn a_failing_composer_self_update_is_only_a_warning() {
    let tools = StubTools::new(&["composer"]);
    tools.set_responses(
        "composer",
        &[
            StubOutput::new().with_status(1), // composer self-update fails
            StubOutput::new(),                // composer global update still runs
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::composer_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
    assert!(
        stdout(&output).contains("warning: composer self-update skipped/failed"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_composer_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["composer"]);

    let output = tools
        .command()
        .args(["--check", "--only", "composer"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
