//! Parity tests for the pipx step.
//!
//! Every test runs the built binary against a stub `pipx` executable and
//! asserts the exact command transcript of the `pipx upgrade-all` step. No
//! real package manager is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "pipx"])
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
fn pipx_upgrades_all_packages() {
    let tools = StubTools::new(&["pipx"]);
    tools.set_responses("pipx", &[StubOutput::new()]); // pipx upgrade-all

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::pipx_transcript());
}

#[test]
fn pipx_skips_when_pipx_is_absent() {
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
fn a_failing_pipx_upgrade_fails_the_step() {
    let tools = StubTools::new(&["pipx"]);
    tools.set_responses(
        "pipx",
        &[StubOutput::new().with_status(1)], // pipx upgrade-all fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::pipx_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_pipx_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["pipx"]);

    let output = tools
        .command()
        .args(["--check", "--only", "pipx"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
