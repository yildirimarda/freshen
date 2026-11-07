//! Parity tests for the pnpm step.
//!
//! Every test runs the built binary against a stub `pnpm` executable and
//! asserts the exact command transcript of the `pnpm update -g` step. No real
//! tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "pnpm"])
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
fn pnpm_updates_global_packages() {
    let tools = StubTools::new(&["pnpm"]);
    tools.set_responses("pnpm", &[StubOutput::new()]); // pnpm update -g

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::pnpm_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
    assert!(
        stdout(&output).contains("pnpm: update -g"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn pnpm_skips_when_pnpm_is_absent() {
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
fn a_failing_pnpm_update_fails_the_step() {
    let tools = StubTools::new(&["pnpm"]);
    tools.set_responses("pnpm", &[StubOutput::new().with_status(1)]); // pnpm update -g fails

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::pnpm_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_pnpm_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["pnpm"]);

    let output = tools
        .command()
        .args(["--check", "--only", "pnpm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
