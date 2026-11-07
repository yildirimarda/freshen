//! Parity tests for the cargo-update step.
//!
//! Every test runs the built binary against stub `cargo` and
//! `cargo-install-update` executables and asserts the exact command transcript
//! of the `cargo install-update -a` step. No real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "cargo-update"])
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
fn cargo_update_installs_all_crates() {
    let tools = StubTools::new(&["cargo", "cargo-install-update"]);
    tools.set_responses("cargo", &[StubOutput::new()]); // cargo install-update -a

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::cargo_update_transcript());
}

#[test]
fn cargo_update_skips_when_cargo_update_is_absent() {
    let tools = StubTools::new(&["cargo"]);

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
fn a_failing_cargo_install_update_fails_the_step() {
    let tools = StubTools::new(&["cargo", "cargo-install-update"]);
    tools.set_responses(
        "cargo",
        &[StubOutput::new().with_status(1)], // cargo install-update -a fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::cargo_update_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_cargo_update_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["cargo", "cargo-install-update"]);

    let output = tools
        .command()
        .args(["--check", "--only", "cargo-update"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
