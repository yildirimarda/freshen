//! Parity tests for the asdf step.
//!
//! Every test runs the built binary against a stub `asdf` executable and
//! asserts the exact command transcript of the `asdf plugin update --all`
//! step. No real package manager is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "asdf"])
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
fn asdf_updates_all_plugins() {
    let tools = StubTools::new(&["asdf"]);
    tools.set_responses("asdf", &[StubOutput::new()]); // asdf plugin update --all

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::asdf_transcript());
}

#[test]
fn asdf_skips_when_asdf_is_absent() {
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
fn a_failing_asdf_plugin_update_fails_the_step() {
    let tools = StubTools::new(&["asdf"]);
    tools.set_responses(
        "asdf",
        &[StubOutput::new().with_status(1)], // asdf plugin update --all fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::asdf_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_asdf_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["asdf"]);

    let output = tools
        .command()
        .args(["--check", "--only", "asdf"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
