//! Parity tests for the pyenv step.
//!
//! Every test runs the built binary against a stub `pyenv` executable and
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
        .args(["--only", "pyenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn pyenv_updates_itself_and_plugins() {
    let tools = StubTools::new(&["pyenv"]);
    tools.set_responses(
        "pyenv",
        &[
            // check(): the active version (`pyenv global` reports nothing).
            StubOutput::new(),
            // run(): `pyenv update`.
            StubOutput::new(),
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec!["pyenv global".to_string(), "pyenv update".to_string(),]
    );
}

#[test]
fn pyenv_skips_when_pyenv_is_absent() {
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
fn a_failing_pyenv_update_fails_the_step() {
    let tools = StubTools::new(&["pyenv"]);
    tools.set_responses(
        "pyenv",
        &[
            // check(): the active version (`pyenv global` reports nothing).
            StubOutput::new(),
            // run(): `pyenv update` fails.
            StubOutput::new().with_status(1),
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec!["pyenv global".to_string(), "pyenv update".to_string(),]
    );
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_pyenv_up_to_date_without_installing_anything() {
    let tools = StubTools::new(&["pyenv"]);
    tools.set_responses(
        "pyenv",
        &[
            // The active version (`pyenv global`).
            StubOutput::stdout("3.12.5\n"),
            // The available releases (`pyenv install --list`), none newer.
            StubOutput::stdout("  3.12.3\n  3.12.4\n  3.12.5\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "pyenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "pyenv global".to_string(),
            "pyenv install --list".to_string(),
        ]
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
    // Nothing is installed: the only `pyenv install` call is the read-only
    // `--list` query, never `pyenv install <version>`.
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call.starts_with("pyenv install ") && call != "pyenv install --list"),
        "calls: {:?}",
        tools.calls()
    );
}

#[test]
fn check_reports_newer_python_version_and_exits_2() {
    let tools = StubTools::new(&["pyenv"]);
    tools.set_responses(
        "pyenv",
        &[
            // The active version (`pyenv global`).
            StubOutput::stdout("3.12.0\n"),
            // The available releases (`pyenv install --list`), 3.13.0 is newest.
            StubOutput::stdout("  3.12.0\n  3.12.1\n  3.13.0\n  3.14.0-dev\n  pypy3.10-7.3.13\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "pyenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "pyenv global".to_string(),
            "pyenv install --list".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(text.contains("pyenv:"), "output: {text}");
    assert!(
        text.contains("python 3.12.0 -> 3.13.0"),
        "missing version report: {text}"
    );
    assert!(
        text.contains("newer Python version available"),
        "missing reason: {text}"
    );
    // Nothing is installed: the only `pyenv install` call is the read-only
    // `--list` query, never `pyenv install <version>`.
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call.starts_with("pyenv install ") && call != "pyenv install --list"),
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
    let tools = StubTools::new(&["pyenv"]);
    tools.set_responses(
        "pyenv",
        &[
            StubOutput::stdout("3.13.0\n"),
            StubOutput::stdout("  3.12.0\n  3.13.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "pyenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "pyenv global".to_string(),
            "pyenv install --list".to_string(),
        ]
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_is_up_to_date_when_pyenv_reports_system_python() {
    let tools = StubTools::new(&["pyenv"]);
    // `pyenv global` reports `system` when no pyenv-managed Python is active.
    tools.set_responses("pyenv", &[StubOutput::stdout("system\n")]);

    let output = tools
        .command()
        .args(["--check", "--only", "pyenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["pyenv global".to_string()]);
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_skips_pyenv_when_pyenv_is_absent() {
    let tools = StubTools::new(&[]);

    let output = tools
        .command()
        .args(["--check", "--only", "pyenv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
