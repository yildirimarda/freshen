//! End-to-end tests for the WSL step.
//!
//! Every test runs the built binary against a stub `wsl` executable and
//! asserts that `wsl --update` runs only when `--install-wsl-updates` is
//! given, because the update shuts down every running WSL distribution. No
//! real WSL installation is touched.

#![cfg(all(feature = "test-harness", target_os = "windows"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools, opt_in: bool) -> Output {
    tools.set_responses(
        "wsl",
        &[
            StubOutput::new(), // detect: wsl --version
            StubOutput::new(), // run: wsl --update (only when opted in)
        ],
    );
    let mut command = tools.command();
    command.args(["--only", "wsl"]);
    if opt_in {
        command.arg("--install-wsl-updates");
    }
    command
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

/// The commands the wsl step issues when opted in: detection then the update.
const WSL_UPDATE_CALLS: &[&str] = &["wsl --version", "wsl --update"];

/// The commands the wsl step issues without the opt-in: detection only.
const WSL_NO_UPDATE_CALLS: &[&str] = &["wsl --version"];

#[test]
fn wsl_updates_the_kernel_when_opted_in() {
    let tools = StubTools::new(&["wsl"]);
    let output = run(&tools, true);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WSL_UPDATE_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("1 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn wsl_does_not_update_without_the_opt_in() {
    let tools = StubTools::new(&["wsl"]);
    let output = run(&tools, false);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WSL_NO_UPDATE_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("1 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn wsl_reports_failed_when_the_update_fails() {
    let tools = StubTools::new(&["wsl"]);
    tools.set_responses(
        "wsl",
        &[
            StubOutput::new(),                 // detect: wsl --version
            StubOutput::new().with_status(42), // run: wsl --update
        ],
    );
    let output = tools
        .command()
        .args(["--only", "wsl", "--install-wsl-updates"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WSL_UPDATE_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn wsl_is_skipped_when_wsl_is_absent() {
    let tools = StubTools::new(&[]);
    let output = tools
        .command()
        .args(["--only", "wsl", "--install-wsl-updates"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools.step_calls().is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

/// `--list-steps` lists the wsl step with its detection state.
#[test]
fn wsl_list_steps_lists_wsl() {
    let tools = StubTools::new(&["wsl"]);
    tools.set_responses("wsl", &[StubOutput::new()]); // detect: wsl --version
    let output = tools
        .command()
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("wsl"), "listing: {text}");
    assert!(text.contains("installed"), "listing: {text}");
}
