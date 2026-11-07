//! Parity tests for the volta step.
//!
//! Every test runs the built binary against stub `volta` (and `brew`/`mise`/`sh`
//! where the self-update guard needs them) executables and asserts the exact
//! command transcript. No real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run(tools: &StubTools, steps: &str) -> Output {
    tools
        .command()
        .args(["--only", steps, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

/// The `volta` check queries: the active version and the available versions.
fn volta_check_steps() -> &'static [&'static str] {
    &["volta current node", "volta list"]
}

#[test]
fn volta_check_reports_newer_node_lts_and_exits_2() {
    let tools = StubTools::new(&["volta"]);
    tools.set_responses(
        "volta",
        &[
            // The active version (`volta current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available Node versions (`volta list`).
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "volta"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        volta_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    let text = stdout(&output);
    assert!(text.contains("volta:"), "output: {text}");
    assert!(
        text.contains("node 20.0.0 -> 22.12.0"),
        "missing LTS major report: {text}"
    );
    assert!(
        text.contains("newer Node LTS major available"),
        "missing LTS reason: {text}"
    );
    // No `volta install` — the check must not switch.
    assert!(
        !text.contains("volta install"),
        "must not install automatically: {text}"
    );
}

#[test]
fn volta_check_up_to_date_when_active_is_latest_lts() {
    let tools = StubTools::new(&["volta"]);
    tools.set_responses(
        "volta",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "volta"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        volta_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn volta_check_skips_node_lts_when_node_is_not_active() {
    let tools = StubTools::new(&["volta"]);
    tools.set_responses(
        "volta",
        &[
            // `volta current node` returns nothing when node is not in use.
            StubOutput::new(),
            StubOutput::stdout("22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "volta"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn volta_run_skips_self_update_when_brew_manages_volta() {
    let tools = StubTools::new(&["volta", "brew"]);
    // `brew list --formula volta` lists volta, so Homebrew owns it.
    tools.set_responses("brew", &[StubOutput::new()]);
    tools.set_responses(
        "volta",
        &[
            // The active version (`volta current node`).
            StubOutput::stdout("22.12.0\n"),
            // The available Node versions (`volta list`).
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "volta");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "volta current node".to_string(),
            "volta list".to_string(),
            "brew list --formula volta".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(
        text.contains("volta self-update skipped: managed by brew or mise"),
        "missing skip message: {text}"
    );
    assert!(
        !text.contains("volta: self-update"),
        "volta must not be self-updated when brew manages it: {text}"
    );
}

#[test]
fn volta_run_skips_self_update_when_mise_manages_node() {
    let tools = StubTools::new(&["volta", "mise"]);
    // `mise current node` returns a version, so mise manages Node.
    tools.set_responses("mise", &[StubOutput::stdout("20.0.0\n")]);
    tools.set_responses(
        "volta",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "volta");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "volta current node".to_string(),
            "volta list".to_string(),
            "mise current node".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(
        text.contains("volta self-update skipped: managed by brew or mise"),
        "missing skip message: {text}"
    );
    assert!(
        !text.contains("volta: self-update"),
        "volta must not be self-updated when mise manages node: {text}"
    );
}

#[cfg(unix)]
#[test]
fn volta_run_self_updates_when_not_managed_by_brew_or_mise() {
    let tools = StubTools::new(&["volta", "sh"]);
    // Neither brew nor mise is installed, so volta is self-updated.
    tools.set_responses("sh", &[StubOutput::new()]);
    tools.set_responses(
        "volta",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "volta");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert!(
        tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("sh -c curl -fsSL")),
        "volta self-update must be attempted when not managed: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(
        text.contains("volta: self-update"),
        "missing self-update line: {text}"
    );
}

#[cfg(windows)]
#[test]
fn volta_run_skips_self_update_when_not_managed_on_windows() {
    let tools = StubTools::new(&["volta"]);
    // Neither brew nor mise is installed, and volta's self-update re-runs the
    // Unix install script, which does not apply on Windows.
    tools.set_responses(
        "volta",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "volta");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert!(
        !tools.step_calls().iter().any(|call| call.starts_with("sh")),
        "volta must not be self-updated with the Unix install script on Windows: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(
        text.contains("volta self-update skipped"),
        "missing skip message: {text}"
    );
}

#[test]
fn volta_absent_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, "volta");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}
