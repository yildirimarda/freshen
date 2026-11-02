//! Parity tests for the Termux step.
//!
//! Every test runs the built binary against a stub `pkg` executable with
//! `TERMUX_VERSION` and `PREFIX` injected, and asserts the exact command
//! transcript of `pkg update` + `pkg upgrade`. No real package manager is
//! touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

const TERMUX_ENV: [(&str, &str); 2] = [
    ("TERMUX_VERSION", "1.117.0"),
    ("PREFIX", "/data/data/com.termux/usr"),
];

/// Builds a freshen command with Termux environment injected and the given
/// arguments, then runs it and captures its output.
fn run_termux(tools: &StubTools, args: &[&str]) -> Output {
    let mut command = tools.command();
    for (key, value) in TERMUX_ENV {
        command.env(key, value);
    }
    command
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

/// Builds a freshen command without any Termux environment, runs it and
/// captures its output.
fn run_plain(tools: &StubTools, args: &[&str]) -> Output {
    tools
        .command()
        .args(args)
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
fn termux_updates_and_upgrades_without_root() {
    let tools = StubTools::new(&["pkg"]);
    tools.set_responses(
        "pkg",
        &[
            StubOutput::new(), // pkg update
            StubOutput::new(), // pkg upgrade
        ],
    );

    let output = run_termux(&tools, &["--only", "termux"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["pkg update", "pkg upgrade"]);
}

#[test]
#[cfg(target_os = "linux")]
fn termux_is_skipped_when_not_running_on_termux() {
    let tools = StubTools::new(&["pkg"]);
    let output = run_plain(&tools, &["--only", "termux"]);

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
fn termux_is_skipped_when_prefix_is_outside_termux() {
    let tools = StubTools::new(&["pkg"]);
    let mut command = tools.command();
    command
        .env("TERMUX_VERSION", "1.117.0")
        .env("PREFIX", "/usr/local");
    let output = command
        .args(["--only", "termux"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_pkg_update_still_attempts_the_upgrade_and_fails_the_step() {
    let tools = StubTools::new(&["pkg"]);
    tools.set_responses(
        "pkg",
        &[
            StubOutput::new().with_status(1), // pkg update fails
            StubOutput::new(),                // pkg upgrade still runs
        ],
    );

    let output = run_termux(&tools, &["--only", "termux"]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["pkg update", "pkg upgrade"]);
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn check_reports_termux_up_to_date_without_running_pkg() {
    let tools = StubTools::new(&["pkg"]);
    let output = run_termux(&tools, &["--check", "--only", "termux"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
