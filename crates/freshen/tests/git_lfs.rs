//! Parity tests for the `git lfs` step.
//!
//! Every test runs the built binary against a stub `git` executable and
//! asserts the exact command transcript of the `git lfs install --skip-repo`
//! step. No real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "git-lfs"])
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
fn git_lfs_installs_hooks() {
    let tools = StubTools::new(&["git"]);
    tools.set_responses(
        "git",
        &[
            StubOutput::new(), // git lfs version (detect)
            StubOutput::new(), // git lfs install --skip-repo (run)
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::git_lfs_transcript());
    assert!(
        stdout(&output).contains("1 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn git_lfs_skips_when_git_lfs_is_absent() {
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
fn check_reports_git_lfs_up_to_date() {
    let tools = StubTools::new(&["git"]);
    tools.set_responses("git", &[StubOutput::new()]); // git lfs version (detect)

    let output = tools
        .command()
        .args(["--check", "--only", "git-lfs"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::git_lfs_version());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
