//! Parity tests for the nvm step.
//!
//! Every test runs the built binary against stub `nvm` (and `brew`/`mise`/`sh`
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

/// The `nvm` check queries: the active version and the available LTS versions.
fn nvm_check_steps() -> &'static [&'static str] {
    &["nvm current", "nvm ls-remote --lts"]
}

#[test]
fn nvm_check_reports_newer_node_lts_and_exits_2() {
    let tools = StubTools::new(&["nvm"]);
    tools.set_responses(
        "nvm",
        &[
            // The active version (`nvm current`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`nvm ls-remote --lts`).
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "nvm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        nvm_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    let text = stdout(&output);
    assert!(text.contains("nvm:"), "output: {text}");
    assert!(
        text.contains("node 20.0.0 -> 22.12.0"),
        "missing LTS major report: {text}"
    );
    assert!(
        text.contains("newer Node LTS major available"),
        "missing LTS reason: {text}"
    );
    // No `nvm install` or `nvm use` — the check must not switch.
    assert!(
        !text.contains("nvm install"),
        "must not install automatically: {text}"
    );
    assert!(
        !text.contains("nvm use"),
        "must not switch automatically: {text}"
    );
}

#[test]
fn nvm_check_up_to_date_when_active_is_latest_lts() {
    let tools = StubTools::new(&["nvm"]);
    tools.set_responses(
        "nvm",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "nvm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        nvm_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn nvm_check_skips_node_lts_when_node_is_not_active() {
    let tools = StubTools::new(&["nvm"]);
    tools.set_responses(
        "nvm",
        &[
            // `nvm current` returns nothing when node is not in use.
            StubOutput::new(),
            StubOutput::stdout("22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "nvm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn nvm_run_skips_self_update_when_brew_manages_nvm() {
    let tools = StubTools::new(&["nvm", "brew"]);
    // `brew list --formula nvm` lists nvm, so Homebrew owns it.
    tools.set_responses("brew", &[StubOutput::new()]);
    tools.set_responses(
        "nvm",
        &[
            // The active version (`nvm current`).
            StubOutput::stdout("22.12.0\n"),
            // The available LTS versions (`nvm ls-remote --lts`).
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "nvm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "nvm current".to_string(),
            "nvm ls-remote --lts".to_string(),
            "brew list --formula nvm".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(
        text.contains("nvm self-update skipped: managed by brew or mise"),
        "missing skip message: {text}"
    );
    assert!(
        !text.contains("nvm: self-update"),
        "nvm must not be self-updated when brew manages it: {text}"
    );
}

#[test]
fn nvm_run_skips_self_update_when_mise_manages_node() {
    let tools = StubTools::new(&["nvm", "mise"]);
    // `mise current node` returns a version, so mise manages Node.
    tools.set_responses("mise", &[StubOutput::stdout("20.0.0\n")]);
    tools.set_responses(
        "nvm",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "nvm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "nvm current".to_string(),
            "nvm ls-remote --lts".to_string(),
            "mise current node".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(
        text.contains("nvm self-update skipped: managed by brew or mise"),
        "missing skip message: {text}"
    );
    assert!(
        !text.contains("nvm: self-update"),
        "nvm must not be self-updated when mise manages node: {text}"
    );
}

#[cfg(unix)]
#[test]
fn nvm_run_self_updates_when_not_managed_by_brew_or_mise() {
    let tools = StubTools::new(&["nvm", "sh"]);
    // Neither brew nor mise is installed, so nvm is self-updated.
    tools.set_responses("sh", &[StubOutput::new()]);
    tools.set_responses(
        "nvm",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "nvm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert!(
        tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("sh -c curl -o-")),
        "nvm self-update must be attempted when not managed: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(
        text.contains("nvm: self-update"),
        "missing self-update line: {text}"
    );
}

#[cfg(windows)]
#[test]
fn nvm_run_skips_self_update_when_not_managed_on_windows() {
    let tools = StubTools::new(&["nvm"]);
    // Neither brew nor mise is installed, and nvm's self-update re-runs the
    // Unix install script, which does not apply on Windows.
    tools.set_responses(
        "nvm",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "nvm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert!(
        !tools.step_calls().iter().any(|call| call.starts_with("sh")),
        "nvm must not be self-updated with the Unix install script on Windows: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(
        text.contains("nvm self-update skipped"),
        "missing skip message: {text}"
    );
}

#[test]
fn nvm_absent_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, "nvm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}
