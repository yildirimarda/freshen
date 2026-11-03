//! End-to-end tests for the Scoop step.
//!
//! Every test runs the built binary against a stub `scoop` executable and
//! asserts that the upgrade exit codes map to ok, skipped and failed
//! respectively. No real package manager is touched.

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

fn run(tools: &StubTools, app_update_exit: i32) -> Output {
    tools.set_responses(
        "scoop",
        &[
            StubOutput::new(),                              // detect: scoop --version
            StubOutput::new(), // check: scoop --disable-interactivity status
            StubOutput::new(), // run: scoop update
            StubOutput::new().with_status(app_update_exit), // run: scoop update *
        ],
    );
    tools
        .command()
        .args(["--only", "scoop"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

/// The commands the scoop step issues, in order: detection, the read-only
/// check, the Scoop update itself and the app update.
const SCOOP_CALLS: &[&str] = &[
    "scoop --version",
    "scoop status",
    "scoop update",
    "scoop update *",
];

#[test]
fn scoop_reports_ok_when_the_app_update_succeeds() {
    let tools = StubTools::new(&["scoop"]);
    let output = run(&tools, 0);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        SCOOP_CALLS,
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
fn scoop_reports_failed_when_the_app_update_fails() {
    let tools = StubTools::new(&["scoop"]);
    let output = run(&tools, 42);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        SCOOP_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

/// With `--cleanup`, `scoop cleanup *` runs after `scoop update *`.
#[test]
fn scoop_runs_cleanup_star_with_cleanup_flag() {
    let tools = StubTools::new(&["scoop"]);
    tools.set_responses(
        "scoop",
        &[
            StubOutput::new(), // detect: scoop --version
            StubOutput::new(), // check: scoop status
            StubOutput::new(), // run: scoop update
            StubOutput::new(), // run: scoop update *
            StubOutput::new(), // cleanup: scoop cleanup *
            StubOutput::new(), // cleanup: scoop cache rm *
        ],
    );
    let output = tools
        .command()
        .args(["--only", "scoop", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        &[
            "scoop --version",
            "scoop status",
            "scoop update",
            "scoop update *",
            "scoop cleanup *",
            "scoop cache rm *",
        ],
        "calls: {:?}",
        tools.step_calls()
    );
}

/// Without `--cleanup`, `scoop cleanup *` is not invoked and the run reports
/// that destructive cleanup was skipped.
#[test]
fn scoop_skips_cleanup_star_without_cleanup_flag() {
    let tools = StubTools::new(&["scoop"]);
    tools.set_responses(
        "scoop",
        &[
            StubOutput::new(), // detect: scoop --version
            StubOutput::new(), // check: scoop status
            StubOutput::new(), // run: scoop update
            StubOutput::new(), // run: scoop update *
        ],
    );
    let output = tools
        .command()
        .args(["--only", "scoop"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        &[
            "scoop --version",
            "scoop status",
            "scoop update",
            "scoop update *",
        ],
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("skipped destructive cleanup"),
        "summary: {}",
        stdout(&output)
    );
}

/// With `--cleanup`, `scoop cache rm *` runs after `scoop cleanup *`.
#[test]
fn scoop_runs_cache_rm_star_with_cleanup_flag() {
    let tools = StubTools::new(&["scoop"]);
    tools.set_responses(
        "scoop",
        &[
            StubOutput::new(), // detect: scoop --version
            StubOutput::new(), // check: scoop status
            StubOutput::new(), // run: scoop update
            StubOutput::new(), // run: scoop update *
            StubOutput::new(), // cleanup: scoop cleanup *
            StubOutput::new(), // cleanup: scoop cache rm *
        ],
    );
    let output = tools
        .command()
        .args(["--only", "scoop", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        &[
            "scoop --version",
            "scoop status",
            "scoop update",
            "scoop update *",
            "scoop cleanup *",
            "scoop cache rm *",
        ],
        "calls: {:?}",
        tools.step_calls()
    );
}

/// Without `--cleanup`, `scoop cache rm *` is not invoked.
#[test]
fn scoop_skips_cache_rm_star_without_cleanup_flag() {
    let tools = StubTools::new(&["scoop"]);
    tools.set_responses(
        "scoop",
        &[
            StubOutput::new(), // detect: scoop --version
            StubOutput::new(), // check: scoop status
            StubOutput::new(), // run: scoop update
            StubOutput::new(), // run: scoop update *
        ],
    );
    let output = tools
        .command()
        .args(["--only", "scoop"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        &[
            "scoop --version",
            "scoop status",
            "scoop update",
            "scoop update *",
        ],
        "calls: {:?}",
        tools.step_calls()
    );
}

/// `--list-steps` lists the scoop step with its detection state.
#[test]
fn scoop_list_steps_lists_scoop() {
    let tools = StubTools::new(&["scoop"]);
    tools.set_responses(
        "scoop",
        &[StubOutput::new()], // detect: scoop --version
    );
    let output = tools
        .command()
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("scoop"), "listing: {text}");
    assert!(text.contains("installed"), "listing: {text}");
}

/// Scoop is skipped when it is absent: detection fails and no `scoop` calls
/// are made.
#[test]
fn scoop_is_skipped_when_scoop_is_absent() {
    let tools = StubTools::new(&[]);
    let output = tools
        .command()
        .args(["--only", "scoop"])
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
