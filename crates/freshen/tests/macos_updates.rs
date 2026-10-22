//! End-to-end checks for the macOS system updates step.
//!
//! The macOS system updates exist only on macOS, so the step only runs there.
//! The run cases are therefore behind a macOS gate, and every other platform
//! checks that the step is not part of a run.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::StubTools;

#[cfg(target_os = "macos")]
use support::StubOutput;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The list `softwareupdate --list` prints on a machine with one update.
#[cfg(target_os = "macos")]
const LIST_OUTPUT: &str = "Software Update Tool\n\nFinding available software\nSoftware Update found the following new or updated software:\n* Label: macOS Sonoma 14.5-23F79\n    Title: macOS Sonoma Update, Version: 14.5, Size: 4.7GB, Recommended: YES, Action: restart,\n";

#[cfg(target_os = "macos")]
#[test]
fn the_available_updates_are_reported_and_not_installed() {
    let tools = StubTools::new(&["softwareupdate"]);
    tools.set_responses(
        "softwareupdate",
        &[
            // The list before the step runs.
            StubOutput::stdout(LIST_OUTPUT),
            // The list after the step ran: the same updates are still there,
            // because nothing was installed.
            StubOutput::stdout(LIST_OUTPUT),
        ],
    );

    let output = tools
        .command()
        .args(["--only", "macos-updates", "--yes"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        ["softwareupdate --list", "softwareupdate --list"]
    );

    let text = stdout(&output);
    assert!(
        text.contains("macOS Sonoma 14.5-23F79"),
        "the update is not in the summary: {text}"
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

#[cfg(target_os = "macos")]
#[test]
fn the_updates_are_installed_with_the_opt_in() {
    let tools = StubTools::new(&["softwareupdate", "sudo"]);
    tools.set_responses(
        "softwareupdate",
        &[
            // The list before the installation.
            StubOutput::stdout(LIST_OUTPUT),
            // The list after the installation: nothing is left to update.
            StubOutput::stdout("No new software available.\n"),
        ],
    );
    tools.set_responses(
        "sudo",
        &[StubOutput::stdout(
            "Installing macOS Sonoma Update\nDone.\n",
        )],
    );

    let output = tools
        .command()
        .args([
            "--only",
            "macos-updates",
            "--yes",
            "--install-macos-updates",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "softwareupdate --list",
            "sudo -n softwareupdate --install --all",
            "softwareupdate --list",
        ]
    );

    let text = stdout(&output);
    assert!(
        text.contains("macOS Sonoma 14.5-23F79"),
        "the installed update is not in the summary: {text}"
    );
    assert!(
        text.contains("-> 14.5"),
        "the installed version is not in the summary: {text}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_available_updates_are_reported_in_check_mode() {
    let tools = StubTools::new(&["softwareupdate"]);
    tools.set_responses("softwareupdate", &[StubOutput::stdout(LIST_OUTPUT)]);

    let output = tools
        .command()
        .args(["--only", "macos-updates", "--check"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), ["softwareupdate --list"]);

    let text = stdout(&output);
    assert!(
        text.contains("macOS Sonoma 14.5-23F79"),
        "the update is not in the report: {text}"
    );
    assert!(
        text.contains("needs a restart"),
        "the report does not say that the update needs a restart: {text}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn an_update_that_needs_a_restart_is_reported() {
    let tools = StubTools::new(&["softwareupdate"]);
    tools.set_responses(
        "softwareupdate",
        &[
            StubOutput::stdout(
                "Software Update found the following new or updated software:\n* Label: macOS Sonoma 14.5-23F79\n    Title: macOS Sonoma Update, Version: 14.5, Recommended: YES, Action: restart,\n",
            ),
            StubOutput::stdout(
                "Software Update found the following new or updated software:\n* Label: macOS Sonoma 14.5-23F79\n    Title: macOS Sonoma Update, Version: 14.5, Recommended: YES, Action: restart,\n",
            ),
        ],
    );

    let output = tools
        .command()
        .args(["--only", "macos-updates", "--yes"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let text = stdout(&output);
    assert!(
        text.contains("will restart the machine"),
        "the summary does not report the pending restart: {text}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_step_is_not_available_on_this_platform() {
    let tools = StubTools::new(&["softwareupdate"]);

    let output = tools
        .command()
        .args(["--only", "macos-updates"])
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

#[cfg(not(target_os = "macos"))]
#[test]
fn the_step_is_listed_as_unsupported_on_this_platform() {
    let tools = StubTools::new(&["softwareupdate"]);

    let output = tools
        .command()
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("macos-updates"), "listing: {text}");
    assert!(
        text.contains("not supported on this platform"),
        "listing: {text}"
    );
    assert!(tools.calls().is_empty(), "the step must not be detected");
}
