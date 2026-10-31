//! Parity tests for the AUR helpers step.
//!
//! Every test runs the built binary against stub `yay`/`paru` executables and
//! asserts the exact command transcript of the AUR helpers upgrade step. No
//! real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "aur"])
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
fn aur_upgrades_packages_with_yay() {
    let tools = StubTools::new(&["yay"]);
    tools.set_responses(
        "yay",
        &[StubOutput::new()], // yay -Syu --noconfirm
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::aur_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn aur_falls_back_to_paru_when_yay_is_missing() {
    let tools = StubTools::new(&["paru"]);
    tools.set_responses(
        "paru",
        &[StubOutput::new()], // paru -Syu --noconfirm
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::aur_paru_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn aur_prefers_you_over_paru_when_both_are_installed() {
    let tools = StubTools::new(&["yay", "paru"]);
    tools.set_responses(
        "yay",
        &[StubOutput::new()], // yay -Syu --noconfirm
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::aur_transcript());
    assert!(
        !stdout(&output).contains("paru"),
        "paru must not run when yay is installed: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_aur_upgrade_fails_the_step() {
    let tools = StubTools::new(&["yay"]);
    tools.set_responses(
        "yay",
        &[StubOutput::new().with_status(1)], // yay -Syu --noconfirm fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::aur_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_aur_helper_is_skipped_without_calling_anything() {
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
fn check_reports_aur_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["yay"]);

    let output = tools
        .command()
        .args(["--check", "--only", "aur"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
