//! Parity tests for the SDKMAN step.
//!
//! Every test runs the built binary against a stub `sdk` executable and
//! asserts the exact command transcript of the `sdk selfupdate` + `sdk
//! upgrade` block. Both commands run non-interactively because freshen gives
//! every child a closed standard input. No real tool is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "sdkman"])
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
fn sdkman_selfupdates_and_upgrades() {
    let tools = StubTools::new(&["sdk"]);
    tools.set_responses(
        "sdk",
        &[
            StubOutput::new(), // sdk selfupdate
            StubOutput::new(), // sdk upgrade
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::sdkman_transcript());
}

#[test]
fn sdkman_skips_when_sdk_is_absent() {
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
fn a_failing_sdk_selfupdate_still_attempts_the_upgrade_and_fails_the_step() {
    let tools = StubTools::new(&["sdk"]);
    tools.set_responses(
        "sdk",
        &[
            StubOutput::new().with_status(1), // sdk selfupdate fails
            StubOutput::new(),                // sdk upgrade still runs
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::sdkman_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_sdkman_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["sdk"]);

    let output = tools
        .command()
        .args(["--check", "--only", "sdkman"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
