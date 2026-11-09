//! Parity tests for the kubectl plugins step.
//!
//! Every test runs the built binary against a stub `kubectl` executable and
//! asserts the exact command transcript of the `kubectl krew upgrade` block.
//! No real kubectl or krew is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "kubectl"])
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
fn kubectl_upgrades_plugins() {
    let tools = StubTools::new(&["kubectl"]);
    tools.set_responses("kubectl", &[StubOutput::new()]); // kubectl krew upgrade

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::kubectl_transcript());
}

#[test]
fn kubectl_skips_when_kubectl_is_absent() {
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
fn a_failing_kubectl_upgrade_fails_the_step() {
    let tools = StubTools::new(&["kubectl"]);
    tools.set_responses(
        "kubectl",
        &[StubOutput::new().with_status(1)], // kubectl krew upgrade fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::kubectl_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_kubectl_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["kubectl"]);

    let output = tools
        .command()
        .args(["--check", "--only", "kubectl"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
