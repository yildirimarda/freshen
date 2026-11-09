//! Parity tests for the oh-my-zsh step.
//!
//! Every test runs the built binary against a stub `omz` executable and
//! asserts the exact command transcript of the `omz update --unattended`
//! step. No real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "oh-my-zsh"])
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
fn omz_updates_shell_config() {
    let tools = StubTools::new(&["omz"]);
    tools.set_responses("omz", &[StubOutput::new()]); // omz update --unattended

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::omz_transcript());
}

#[test]
fn omz_skips_when_omz_is_absent() {
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
fn a_failing_omz_update_fails_the_step() {
    let tools = StubTools::new(&["omz"]);
    tools.set_responses(
        "omz",
        &[StubOutput::new().with_status(1)], // omz update --unattended fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::omz_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_oh_my_zsh_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["omz"]);

    let output = tools
        .command()
        .args(["--check", "--only", "oh-my-zsh"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
