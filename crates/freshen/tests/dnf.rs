//! Parity tests for the dnf step.
//!
//! Every test runs the built binary against a stub `dnf` executable and
//! asserts the exact command transcript of the `dnf upgrade -y` step. No real
//! package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "dnf"])
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
fn dnf_upgrades_packages() {
    let tools = StubTools::new(&["dnf"]);
    tools.set_responses(
        "dnf",
        &[StubOutput::new()], // dnf upgrade -y
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::dnf_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_dnf_upgrade_fails_the_step() {
    let tools = StubTools::new(&["dnf"]);
    tools.set_responses(
        "dnf",
        &[StubOutput::new().with_status(1)], // dnf upgrade -y fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::dnf_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_dnf_is_skipped_without_calling_anything() {
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
fn check_reports_dnf_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["dnf"]);

    let output = tools
        .command()
        .args(["--check", "--only", "dnf"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
