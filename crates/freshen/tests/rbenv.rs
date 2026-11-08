//! Parity tests for the rbenv step.
//!
//! Every test runs the built binary against a stub `rbenv` executable and
//! asserts the exact command transcript. No real package manager is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "rbenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn rbenv_updates_itself_and_ruby_build() {
    let tools = StubTools::new(&["rbenv"]);
    tools.set_responses(
        "rbenv",
        &[
            // check(): the active version (`rbenv global` reports nothing).
            StubOutput::new(),
            // run(): `rbenv update`.
            StubOutput::new(),
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec!["rbenv global".to_string(), "rbenv update".to_string(),]
    );
}

#[test]
fn rbenv_skips_when_rbenv_is_absent() {
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
fn a_failing_rbenv_update_fails_the_step() {
    let tools = StubTools::new(&["rbenv"]);
    tools.set_responses(
        "rbenv",
        &[
            // check(): the active version (`rbenv global` reports nothing).
            StubOutput::new(),
            // run(): `rbenv update` fails.
            StubOutput::new().with_status(1),
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec!["rbenv global".to_string(), "rbenv update".to_string(),]
    );
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_rbenv_up_to_date_without_installing_anything() {
    let tools = StubTools::new(&["rbenv"]);
    tools.set_responses(
        "rbenv",
        &[
            // The active version (`rbenv global`).
            StubOutput::stdout("3.2.0\n"),
            // The available releases (`rbenv install --list`), none newer.
            StubOutput::stdout("  3.1.2\n  3.2.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "rbenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "rbenv global".to_string(),
            "rbenv install --list".to_string(),
        ]
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
    // Nothing is installed: the only `rbenv install` call is the read-only
    // `--list` query, never `rbenv install <version>`.
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call.starts_with("rbenv install ") && call != "rbenv install --list"),
        "calls: {:?}",
        tools.calls()
    );
}

#[test]
fn check_reports_newer_ruby_version_and_exits_2() {
    let tools = StubTools::new(&["rbenv"]);
    tools.set_responses(
        "rbenv",
        &[
            // The active version (`rbenv global`).
            StubOutput::stdout("3.1.2\n"),
            // The available releases (`rbenv install --list`), 3.3.0 is newest.
            StubOutput::stdout("  3.1.2\n  3.2.0\n  3.3.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "rbenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "rbenv global".to_string(),
            "rbenv install --list".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(text.contains("rbenv:"), "output: {text}");
    assert!(
        text.contains("ruby 3.1.2 -> 3.3.0"),
        "missing version report: {text}"
    );
    assert!(
        text.contains("newer Ruby version available"),
        "missing reason: {text}"
    );
    // Nothing is installed: the only `rbenv install` call is the read-only
    // `--list` query, never `rbenv install <version>`.
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call.starts_with("rbenv install ") && call != "rbenv install --list"),
        "calls: {:?}",
        tools.calls()
    );
    assert!(
        text.contains("1 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
}

#[test]
fn check_up_to_date_when_active_is_the_latest_release() {
    let tools = StubTools::new(&["rbenv"]);
    tools.set_responses(
        "rbenv",
        &[
            StubOutput::stdout("3.3.0\n"),
            StubOutput::stdout("  3.2.0\n  3.3.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "rbenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "rbenv global".to_string(),
            "rbenv install --list".to_string(),
        ]
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_is_up_to_date_when_rbenv_reports_system_ruby() {
    let tools = StubTools::new(&["rbenv"]);
    // `rbenv global` reports `system` when no rbenv-managed Ruby is active.
    tools.set_responses("rbenv", &[StubOutput::stdout("system\n")]);

    let output = tools
        .command()
        .args(["--check", "--only", "rbenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["rbenv global".to_string()]);
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_skips_rbenv_when_rbenv_is_absent() {
    let tools = StubTools::new(&[]);

    let output = tools
        .command()
        .args(["--check", "--only", "rbenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
