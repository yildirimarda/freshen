//! Parity tests for the bootc step.
//!
//! Every test runs the built binary against a stub `bootc` executable and
//! asserts the exact command transcript of the staged `bootc upgrade`. No real
//! package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "bootc"])
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
fn bootc_upgrades() {
    let tools = StubTools::new(&["bootc"]);
    tools.set_responses(
        "bootc",
        &[StubOutput::new()], // bootc upgrade
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::bootc_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_bootc_upgrade_fails_the_step() {
    let tools = StubTools::new(&["bootc"]);
    tools.set_responses(
        "bootc",
        &[StubOutput::new().with_status(1)], // bootc upgrade fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::bootc_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_bootc_is_skipped_without_calling_anything() {
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
fn bootc_announces_a_pending_reboot() {
    let tools = StubTools::new(&["bootc"]);
    tools.set_responses(
        "bootc",
        &[StubOutput::new()], // bootc upgrade
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::bootc_transcript());
    assert!(
        stdout(&output).contains("restart needed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn bootc_reports_restart_needed_in_the_json_report() {
    let tools = StubTools::new(&["bootc"]);
    tools.set_responses(
        "bootc",
        &[StubOutput::new()], // bootc upgrade
    );

    let output = tools
        .command()
        .args(["--only", "bootc", "--json"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let report: serde_json::Value = match serde_json::from_str(&stdout(&output)) {
        Ok(report) => report,
        Err(error) => panic!("invalid JSON report: {error}"),
    };
    let step = &report["steps"][0];
    assert_eq!(step["id"], "bootc");
    assert!(step["restart_needed"].as_bool().unwrap_or(false));
}

#[test]
#[cfg(target_os = "linux")]
fn check_reports_bootc_up_to_date_without_calling_anything_else() {
    let tools = StubTools::new(&["bootc"]);

    let output = tools
        .command()
        .args(["--check", "--only", "bootc"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
