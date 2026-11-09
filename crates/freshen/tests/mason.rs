//! Parity tests for the Mason step.
//!
//! Every test runs the built binary against a stub `nvim` executable and
//! asserts the exact command transcript of the `nvim --headless "+MasonUpdate"
//! +qa` step. No real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "nvim-mason", "--install-nvim-mason"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[cfg(unix)]
#[test]
fn nvim_mason_updates_packages() {
    let tools = StubTools::new(&["nvim"]);
    tools.set_responses("nvim", &[StubOutput::new()]);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::nvim_mason_transcript());
}

#[test]
fn nvim_mason_skips_when_nvim_is_absent() {
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

#[cfg(unix)]
#[test]
fn a_failing_nvim_mason_fails_the_step() {
    let tools = StubTools::new(&["nvim"]);
    tools.set_responses("nvim", &[StubOutput::new().with_status(1)]);

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::nvim_mason_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_nvim_mason_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["nvim"]);

    let output = tools
        .command()
        .args(["--check", "--only", "nvim-mason"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn nvim_mason_does_not_run_without_the_opt_in_flag() {
    let tools = StubTools::new(&["nvim"]);
    tools.set_responses("nvim", &[StubOutput::new()]);

    let output = tools
        .command()
        .args(["--only", "nvim-mason"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools.step_calls().is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("nothing updated"),
        "summary: {}",
        stdout(&output)
    );
}
