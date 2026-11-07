//! Parity tests for the fnm step.
//!
//! Every test runs the built binary against stub `fnm` (and `brew`/`mise`/`sh`
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

/// The `fnm` check queries: the active version and the available LTS versions.
fn fnm_check_steps() -> &'static [&'static str] {
    &["fnm current", "fnm ls-remote --lts"]
}

#[test]
fn fnm_check_reports_newer_node_lts_and_exits_2() {
    let tools = StubTools::new(&["fnm"]);
    tools.set_responses(
        "fnm",
        &[
            // The active version (`fnm current`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`fnm ls-remote --lts`).
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "fnm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        fnm_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    let text = stdout(&output);
    assert!(text.contains("fnm:"), "output: {text}");
    assert!(
        text.contains("node 20.0.0 -> 22.12.0"),
        "missing LTS major report: {text}"
    );
    assert!(
        text.contains("newer Node LTS major available"),
        "missing LTS reason: {text}"
    );
    // No `fnm install` or `fnm use` — the check must not switch.
    assert!(
        !text.contains("fnm install"),
        "must not install automatically: {text}"
    );
    assert!(
        !text.contains("fnm use"),
        "must not switch automatically: {text}"
    );
}

#[test]
fn fnm_check_up_to_date_when_active_is_latest_lts() {
    let tools = StubTools::new(&["fnm"]);
    tools.set_responses(
        "fnm",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "fnm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        fnm_check_steps()
            .iter()
            .map(|call| call.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn fnm_check_skips_node_lts_when_node_is_not_active() {
    let tools = StubTools::new(&["fnm"]);
    tools.set_responses(
        "fnm",
        &[
            // `fnm current` returns nothing when node is not in use.
            StubOutput::new(),
            StubOutput::stdout("22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "fnm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn fnm_run_skips_self_update_when_brew_manages_fnm() {
    let tools = StubTools::new(&["fnm", "brew"]);
    // `brew list --formula fnm` lists fnm, so Homebrew owns it.
    tools.set_responses("brew", &[StubOutput::new()]);
    tools.set_responses(
        "fnm",
        &[
            // The active version (`fnm current`).
            StubOutput::stdout("22.12.0\n"),
            // The available LTS versions (`fnm ls-remote --lts`).
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "fnm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "fnm current".to_string(),
            "fnm ls-remote --lts".to_string(),
            "brew list --formula fnm".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(
        text.contains("fnm self-update skipped: managed by brew or mise"),
        "missing skip message: {text}"
    );
    assert!(
        !text.contains("fnm: self-update"),
        "fnm must not be self-updated when brew manages it: {text}"
    );
}

#[test]
fn fnm_run_skips_self_update_when_mise_manages_node() {
    let tools = StubTools::new(&["fnm", "mise"]);
    // `mise current node` returns a version, so mise manages Node.
    tools.set_responses("mise", &[StubOutput::stdout("20.0.0\n")]);
    tools.set_responses(
        "fnm",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "fnm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "fnm current".to_string(),
            "fnm ls-remote --lts".to_string(),
            "mise current node".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(
        text.contains("fnm self-update skipped: managed by brew or mise"),
        "missing skip message: {text}"
    );
    assert!(
        !text.contains("fnm: self-update"),
        "fnm must not be self-updated when mise manages node: {text}"
    );
}

#[cfg(unix)]
#[test]
fn fnm_run_self_updates_when_not_managed_by_brew_or_mise() {
    let tools = StubTools::new(&["fnm", "sh"]);
    // Neither brew nor mise is installed, so fnm is self-updated.
    tools.set_responses("sh", &[StubOutput::new()]);
    tools.set_responses(
        "fnm",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "fnm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert!(
        tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("sh -c curl -fsSL")),
        "fnm self-update must be attempted when not managed: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(
        text.contains("fnm: self-update"),
        "missing self-update line: {text}"
    );
}

#[cfg(windows)]
#[test]
fn fnm_run_skips_self_update_when_not_managed_on_windows() {
    let tools = StubTools::new(&["fnm"]);
    // Neither brew nor mise is installed, and fnm's self-update re-runs the
    // Unix install script, which does not apply on Windows.
    tools.set_responses(
        "fnm",
        &[
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = run(&tools, "fnm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert!(
        !tools.step_calls().iter().any(|call| call.starts_with("sh")),
        "fnm must not be self-updated with the Unix install script on Windows: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(
        text.contains("fnm self-update skipped"),
        "missing skip message: {text}"
    );
}

#[test]
fn fnm_absent_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, "fnm");

    assert!(output.status.success(), "stderr: {}", stdout(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}
