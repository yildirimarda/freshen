//! End-to-end checks for the `freshen self-update` subcommand.
//!
//! These tests run the built binary against a stub `brew`, so no test can
//! invoke the real Homebrew.

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

fn run_self_update(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["self-update"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn self_update_upgrades_freshen_through_brew() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &[
            // Homebrew owns freshen.
            StubOutput::new().with_status(0),
            // The upgrade itself.
            StubOutput::new().with_status(0),
        ],
    );

    let output = run_self_update(&tools);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));

    let calls = tools.step_calls();
    assert!(
        calls
            .iter()
            .any(|call| call == "brew list --formula freshen"),
        "ownership check was not run: {calls:?}"
    );
    assert!(
        calls.iter().any(|call| call == "brew upgrade freshen"),
        "upgrade was not run: {calls:?}"
    );
    let text = stdout(&output);
    assert!(text.contains("self-update via Homebrew"), "stdout: {text}");
    assert!(text.contains("self-update complete"), "stdout: {text}");
}

#[test]
fn self_update_reports_when_brew_is_not_available() {
    let tools = StubTools::new(&[]);
    let output = run_self_update(&tools);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("brew"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn self_update_reports_when_freshen_is_not_owned_by_brew() {
    let tools = StubTools::new(&["brew"]);
    // `brew list --formula freshen` reports that Homebrew does not own freshen.
    tools.set_responses("brew", &[StubOutput::new().with_status(1)]);

    let output = run_self_update(&tools);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("not installed through Homebrew"),
        "stderr: {}",
        stderr(&output)
    );
    // The upgrade must never be attempted when freshen is not owned by brew.
    let calls = tools.step_calls();
    assert!(
        !calls.iter().any(|call| call.contains("brew upgrade")),
        "upgrade was attempted although brew does not own freshen: {calls:?}"
    );
}
