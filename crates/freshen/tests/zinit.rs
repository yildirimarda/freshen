//! Parity tests for the zinit step.
//!
//! Every test runs the built binary against a stub `zinit` executable and
//! asserts the exact command transcript of the `zinit self-update` + `zinit
//! update --all` block. No real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "zinit"])
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
fn zinit_selfupdates_and_updates_all_plugins() {
    let tools = StubTools::new(&["zinit"]);
    tools.set_responses(
        "zinit",
        &[
            StubOutput::new(), // zinit self-update
            StubOutput::new(), // zinit update --all
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::zinit_transcript());
}

#[test]
fn zinit_skips_when_zinit_is_absent() {
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
fn a_failing_zinit_selfupdate_still_attempts_the_update_and_fails_the_step() {
    let tools = StubTools::new(&["zinit"]);
    tools.set_responses(
        "zinit",
        &[
            StubOutput::new().with_status(1), // zinit self-update fails
            StubOutput::new(),                // zinit update --all still runs
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::zinit_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_zinit_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["zinit"]);

    let output = tools
        .command()
        .args(["--check", "--only", "zinit"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
