//! End-to-end checks for the firmware updates step.
//!
//! The step runs `fwupdmgr get-updates` to report what is available and
//! `fwupdmgr update` to install them only when `--install-firmware-updates` is
//! given. No real firmware tooling is touched: the stub `fwupdmgr` replays
//! recorded transcripts.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The output of `fwupdmgr get-updates` on a machine with one update.
const GET_UPDATES: &str = "1234abcd-1234-1234-1234-1234abcd1234:\n\
     \tDevice: ThinkPad Thunderbolt Controller\n\
     \tSummary: Thunderbolt firmware update\n\
     \tVersion: 1.2.3\n\
     \tVersion: 1.2.4\n\
     \tRemote id: lvfs\n";

/// The output of `fwupdmgr get-updates` when nothing needs updating.
const NO_UPDATES: &str = "No updates found.\n";

#[test]
fn the_available_firmware_updates_are_reported_and_not_installed() {
    let tools = StubTools::new(&["fwupdmgr"]);
    tools.set_responses("fwupdmgr", &[StubOutput::stdout(GET_UPDATES)]);

    let output = tools
        .command()
        .args(["--only", "firmware", "--yes"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::firmware_report_transcript());
    let text = stdout(&output);
    assert!(
        text.contains("ThinkPad Thunderbolt Controller"),
        "the update is not in the summary: {text}"
    );
    assert!(
        text.contains("1.2.3 -> 1.2.4"),
        "the versions are not in the summary: {text}"
    );
    assert!(
        text.contains("nothing installed"),
        "the summary does not say that nothing was installed: {text}"
    );
    assert!(
        text.contains("1 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {text}"
    );
}

#[test]
fn the_firmware_updates_are_installed_with_the_opt_in() {
    let tools = StubTools::new(&["fwupdmgr"]);
    tools.set_responses(
        "fwupdmgr",
        &[
            // The listing captured by the check before the installation.
            StubOutput::stdout(GET_UPDATES),
            // The installation itself.
            StubOutput::stdout(""),
        ],
    );

    let output = tools
        .command()
        .args(["--only", "firmware", "--yes", "--install-firmware-updates"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::firmware_install_transcript());
}

#[test]
fn check_reports_the_available_updates_and_exits_2() {
    let tools = StubTools::new(&["fwupdmgr"]);
    tools.set_responses("fwupdmgr", &[StubOutput::stdout(GET_UPDATES)]);

    let output = tools
        .command()
        .args(["--check", "--only", "firmware"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::firmware_check_transcript());
    let text = stdout(&output);
    assert!(
        text.contains("ThinkPad Thunderbolt Controller"),
        "the update is not in the report: {text}"
    );
    assert!(
        text.contains("1.2.3 -> 1.2.4"),
        "the versions are not in the report: {text}"
    );
    assert!(
        text.contains("1 update(s) pending across 1 step(s)."),
        "report: {text}"
    );
}

#[test]
fn check_reports_up_to_date_without_running_anything_else() {
    let tools = StubTools::new(&["fwupdmgr"]);
    tools.set_responses("fwupdmgr", &[StubOutput::stdout(NO_UPDATES)]);

    let output = tools
        .command()
        .args(["--check", "--only", "firmware"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::firmware_check_transcript());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn a_missing_fwupdmgr_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = tools
        .command()
        .args(["--only", "firmware"])
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

#[cfg(not(target_os = "linux"))]
#[test]
fn the_step_is_not_available_on_this_platform() {
    let tools = StubTools::new(&["fwupdmgr"]);

    let output = tools
        .command()
        .args(["--only", "firmware"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("not available on"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(tools.calls().is_empty(), "the step must not run");
}

#[cfg(not(target_os = "linux"))]
#[test]
fn the_step_is_listed_as_unsupported_on_this_platform() {
    let tools = StubTools::new(&["fwupdmgr"]);

    let output = tools
        .command()
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("firmware"), "listing: {text}");
    assert!(
        text.contains("not supported on this platform"),
        "listing: {text}"
    );
    assert!(tools.calls().is_empty(), "the step must not be detected");
}
