//! Parity tests for the jenv step.
//!
//! Every test runs the built binary against a stub `jenv` executable and
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

/// The `jenv` check queries: installed versions, then available versions.
fn jenv_check_steps() -> &'static [&'static str] {
    &["jenv versions", "jenv install -l"]
}

fn run(tools: &StubTools, steps: &str) -> Output {
    tools
        .command()
        .args(["--only", steps, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn jenv_check_reports_newer_java_versions_and_exits_2() {
    let tools = StubTools::new(&["jenv"]);
    tools.set_responses(
        "jenv",
        &[
            // The installed versions (`jenv versions`).
            StubOutput::stdout(
                "  system\n  11.0.2\n  17.0.2\n* 21.0.2 (set by /Users/user/.java-version)\n",
            ),
            // The versions available to install (`jenv install -l`).
            StubOutput::stdout(
                "  Adoptium:11.0.22\n  Adoptium:17.0.12\n  Adoptium:21.0.3\n  Adoptium:22.0.1\n",
            ),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "jenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        jenv_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    let text = stdout(&output);
    assert!(text.contains("jenv:"), "output: {text}");
    assert!(
        text.contains("java 11.0.2 -> 22.0.1"),
        "missing outdated report: {text}"
    );
    assert!(
        text.contains("java 17.0.2 -> 22.0.1"),
        "missing outdated report: {text}"
    );
    assert!(
        text.contains("java 21.0.2 -> 22.0.1"),
        "missing outdated report: {text}"
    );
    assert!(
        text.contains("3 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
    // No `jenv install` or `jenv use` — the check must not switch.
    assert!(!text.contains("jenv install"), "must not install: {text}");
    assert!(!text.contains("jenv use"), "must not switch: {text}");
}

#[test]
fn jenv_check_up_to_date_when_installed_is_newest() {
    let tools = StubTools::new(&["jenv"]);
    tools.set_responses(
        "jenv",
        &[
            StubOutput::stdout("  system\n* 22.0.1 (set by /Users/user/.java-version)\n"),
            StubOutput::stdout("  Adoptium:21.0.2\n  Adoptium:22.0.1\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "jenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        jenv_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn jenv_absent_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, "jenv");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}
