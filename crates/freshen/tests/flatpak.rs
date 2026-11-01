//! Parity tests for the Flatpak step.
//!
//! Every test runs the built binary against a stub `flatpak` executable and
//! asserts the exact command transcript of the `flatpak update -y` step. No
//! real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "flatpak"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn run_with_cleanup(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "flatpak", "--cleanup"])
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
fn flatpak_updates_packages() {
    let tools = StubTools::new(&["flatpak"]);
    tools.set_responses(
        "flatpak",
        &[StubOutput::new()], // flatpak update -y
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::flatpak_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn flatpak_runs_uninstall_unused_with_cleanup() {
    let tools = StubTools::new(&["flatpak"]);
    tools.set_responses(
        "flatpak",
        &[
            StubOutput::new(), // flatpak update -y
            StubOutput::new(), // flatpak uninstall --unused
        ],
    );

    let output = run_with_cleanup(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::flatpak_cleanup_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn flatpak_skips_cleanup_without_cleanup_flag() {
    let tools = StubTools::new(&["flatpak"]);
    tools.set_responses(
        "flatpak",
        &[StubOutput::new()], // flatpak update -y
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["flatpak update -y"]);
    assert!(
        stdout(&output).contains("skipped destructive cleanup"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn flatpak_skips_cleanup_when_flatpak_is_missing() {
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
fn a_failing_flatpak_update_fails_the_step() {
    let tools = StubTools::new(&["flatpak"]);
    tools.set_responses(
        "flatpak",
        &[StubOutput::new().with_status(1)], // flatpak update -y fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::flatpak_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_flatpak_is_skipped_without_calling_anything() {
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
fn check_reports_flatpak_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["flatpak"]);

    let output = tools
        .command()
        .args(["--check", "--only", "flatpak"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
