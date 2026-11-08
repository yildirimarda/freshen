//! Parity tests for the tfenv step.
//!
//! Every test runs the built binary against a stub `tfenv` executable and
//! asserts the exact command transcript. No real tool is touched.

#![cfg(all(feature = "test-harness", not(windows)))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The `tfenv` check queries: installed versions, then available versions.
fn tfenv_check_steps() -> &'static [&'static str] {
    &["tfenv list", "tfenv list-remote"]
}

#[test]
fn tfenv_check_reports_newer_terraform_versions_and_exits_2() {
    let tools = StubTools::new(&["tfenv"]);
    tools.set_responses(
        "tfenv",
        &[
            // The installed versions (`tfenv list`).
            StubOutput::stdout(
                "  1.5.7\n* 1.6.1 (set by /Users/user/.terraform-version)\n  1.7.0\n",
            ),
            // The versions available to install (`tfenv list-remote`).
            StubOutput::stdout("1.5.7\n1.6.1\n1.6.2\n1.7.1\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "tfenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        tfenv_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    let text = stdout(&output);
    assert!(text.contains("tfenv:"), "output: {text}");
    assert!(
        text.contains("terraform 1.5.7 -> 1.7.1"),
        "missing outdated report: {text}"
    );
    assert!(
        text.contains("terraform 1.6.1 -> 1.7.1"),
        "missing outdated report: {text}"
    );
    assert!(
        text.contains("terraform 1.7.0 -> 1.7.1"),
        "missing outdated report: {text}"
    );
    assert!(
        text.contains("3 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
    // No `tfenv install` or `tfenv use` — the check must not switch.
    assert!(!text.contains("tfenv install"), "must not install: {text}");
    assert!(!text.contains("tfenv use"), "must not switch: {text}");
}

#[test]
fn tfenv_check_up_to_date_when_installed_is_newest() {
    let tools = StubTools::new(&["tfenv"]);
    tools.set_responses(
        "tfenv",
        &[
            StubOutput::stdout("* 1.7.1 (set by /Users/user/.terraform-version)\n"),
            StubOutput::stdout("1.6.1\n1.7.0\n1.7.1\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "tfenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        tfenv_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn tfenv_absent_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = tools
        .command()
        .args(["--only", "tfenv", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}
