//! Parity tests for the pacman step.
//!
//! Every test runs the built binary against a stub `pacman` executable and
//! asserts the exact command transcript of the `pacman -Syu --noconfirm`
//! step. No real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "pacman"])
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
fn pacman_upgrades_packages() {
    let tools = StubTools::new(&["pacman"]);
    tools.set_responses(
        "pacman",
        &[StubOutput::new()], // pacman -Syu --noconfirm
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::pacman_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_pacman_upgrade_fails_the_step() {
    let tools = StubTools::new(&["pacman"]);
    tools.set_responses(
        "pacman",
        &[StubOutput::new().with_status(1)], // pacman -Syu --noconfirm fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::pacman_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_pacman_is_skipped_without_calling_anything() {
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
fn check_reports_pacman_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["pacman"]);

    let output = tools
        .command()
        .args(["--check", "--only", "pacman"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
