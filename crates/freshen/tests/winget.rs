//! End-to-end tests for the winget step.
//!
//! Every test runs the built binary against a stub `winget` executable and
//! asserts that the upgrade exit codes (`0`, the update-not-applicable code
//! and other non-zero codes) map to ok, skipped and failed respectively. No
//! real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "windows"))]

mod support;

use std::process::Output;

use freshen_lib::winget::EXIT_UPDATE_NOT_APPLICABLE;
use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools, upgrade_exit: i32) -> Output {
    tools.set_responses(
        "winget",
        &[
            StubOutput::new(),                           // detect: winget --version
            StubOutput::new(), // check: winget --disable-interactivity upgrade
            StubOutput::new(), // run: winget --disable-interactivity source update
            StubOutput::new().with_status(upgrade_exit), // run: winget upgrade --all ...
        ],
    );
    tools
        .command()
        .args(["--only", "winget"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

/// The commands the winget step issues, in order: detection, the read-only
/// check, the source refresh and the upgrade itself.
const WINGET_CALLS: &[&str] = &[
    "winget --version",
    "winget --disable-interactivity upgrade",
    "winget --disable-interactivity source update",
    "winget --disable-interactivity upgrade --all --silent --accept-package-agreements --accept-source-agreements",
];

#[test]
fn winget_reports_ok_when_the_upgrade_succeeds() {
    let tools = StubTools::new(&["winget"]);
    let output = run(&tools, 0);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WINGET_CALLS,
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
fn winget_reports_skipped_when_no_upgrades_are_applicable() {
    let tools = StubTools::new(&["winget"]);
    let output = run(&tools, EXIT_UPDATE_NOT_APPLICABLE);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WINGET_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn winget_reports_failed_when_the_upgrade_fails() {
    let tools = StubTools::new(&["winget"]);
    let output = run(&tools, 42);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WINGET_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {}",
        stdout(&output)
    );
}
