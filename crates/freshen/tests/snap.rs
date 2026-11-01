//! Parity tests for the Snap step.
//!
//! Every test runs the built binary against a stub `snap` executable and
//! asserts the exact command transcript of the `snap refresh` step. No real
//! package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "snap"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn run_with_cleanup(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "snap", "--cleanup"])
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
fn snap_refreshes_packages() {
    let tools = StubTools::new(&["snap"]);
    tools.set_responses(
        "snap",
        &[StubOutput::new()], // snap refresh
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::snap_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn snap_skips_cleanup_without_cleanup_flag() {
    let tools = StubTools::new(&["snap"]);
    tools.set_responses(
        "snap",
        &[StubOutput::new()], // snap refresh
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["snap refresh"]);
}

#[test]
#[cfg(target_os = "linux")]
fn snap_skips_cleanup_when_snap_is_missing() {
    let tools = StubTools::new(&[]);

    let output = run_with_cleanup(&tools);

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
fn a_failing_snap_refresh_fails_the_step() {
    let tools = StubTools::new(&["snap"]);
    tools.set_responses(
        "snap",
        &[StubOutput::new().with_status(1)], // snap refresh fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::snap_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_held_snap_refresh_window_is_reported_instead_of_a_failure() {
    let tools = StubTools::new(&["snap"]);
    tools.set_responses(
        "snap",
        &[StubOutput::new()
            .with_stderr("error: a snap refresh is held until 2026-03-01T00:00:00+01:00")
            .with_status(1)],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::snap_transcript());
    let out = stdout(&output);
    assert!(out.contains("held"), "stdout: {}", out);
    let snap_line = out
        .lines()
        .find(|line| line.starts_with("  ") && line.contains("snap"))
        .unwrap_or_default();
    assert!(
        snap_line.starts_with("  ok"),
        "snap step line: {snap_line}\nstdout: {out}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_snap_is_skipped_without_calling_anything() {
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
fn check_reports_snap_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["snap"]);

    let output = tools
        .command()
        .args(["--check", "--only", "snap"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
