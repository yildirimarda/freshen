//! Parity tests for the pip step.
//!
//! Every test runs the built binary against a stub `pip` executable and
//! asserts the exact command transcript of the report-only check. No real
//! package manager is touched, and the step never upgrades anything.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "pip"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn outdated_transcript() -> &'static [&'static str] {
    &[
        "Package    Version Latest   Type",
        "---------- ------- ------ -----",
        "black      23.1.0  26.10.1  wheel",
        "ruff       0.6.0   0.7.0    wheel",
    ]
}

#[test]
fn pip_skips_when_pip_is_absent() {
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
fn check_reports_outdated_packages_and_exits_2() {
    let tools = StubTools::new(&["pip"]);
    tools.set_responses(
        "pip",
        &[StubOutput::stdout(outdated_transcript().join("\n"))],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "pip"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::pip_transcript(),
        "calls: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(text.contains("pip:"), "output: {text}");
    assert!(text.contains("black 23.1.0 -> 26.10.1"), "output: {text}");
    assert!(text.contains("ruff 0.6.0 -> 0.7.0"), "output: {text}");
    assert!(
        text.contains("2 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
}

#[test]
fn check_reports_pip_up_to_date() {
    let tools = StubTools::new(&["pip"]);
    tools.set_responses(
        "pip",
        &[StubOutput::stdout(
            "Package    Version Latest   Type\n---------- ------- ------ -----\n",
        )],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "pip"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::pip_transcript(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_reports_a_failed_pip_query_with_exit_1() {
    let tools = StubTools::new(&["pip"]);
    tools.set_responses("pip", &[StubOutput::new().with_status(1)]);

    let output = tools
        .command()
        .args(["--check", "--only", "pip"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::pip_transcript(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("failed during check"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn run_never_upgrades_anything() {
    let tools = StubTools::new(&["pip"]);
    tools.set_responses(
        "pip",
        &[StubOutput::stdout(outdated_transcript().join("\n"))],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // Report-only: no `pip install` or `pip upgrade` may ever run.
    assert!(
        !tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("pip install")),
        "calls: {:?}",
        tools.step_calls()
    );
}
