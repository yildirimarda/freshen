//! End-to-end checks for the Mac App Store step.
//!
//! The Mac App Store exists only on macOS, so the step only runs there. The
//! run cases are therefore behind a macOS gate, and every other platform
//! checks that the step is not part of a run.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::StubTools;

#[cfg(target_os = "macos")]
use support::{mas_account, mas_check_transcript, mas_full_transcript};

#[cfg(target_os = "macos")]
use support::StubOutput;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[cfg(target_os = "macos")]
#[test]
fn the_outdated_apps_are_reported_and_upgraded() {
    let tools = StubTools::new(&["mas"]);
    tools.set_responses(
        "mas",
        &[
            // `mas account`, which asks whether an account is signed in.
            StubOutput::stdout("someone@example.com\n"),
            // `mas outdated`, before the upgrade.
            StubOutput::stdout("409203825  Numbers  (4.2 -> 4.3.1)\n"),
            // `mas upgrade`, which installs the pending update.
            StubOutput::new(),
            // `mas outdated`, after the upgrade: nothing is left to update.
            StubOutput::new(),
        ],
    );

    let output = tools
        .command()
        .args(["--only", "mas", "--yes"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        mas_full_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );

    let text = stdout(&output);
    assert!(
        text.contains("Numbers 4.2 -> 4.3.1"),
        "the changed version is not in the summary: {text}"
    );
    assert!(
        text.contains("1 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {text}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_machine_without_a_signed_in_account_skips_the_step() {
    let tools = StubTools::new(&["mas"]);
    tools.set_responses(
        "mas",
        &[StubOutput::stderr("Error: Not signed in\n").with_status(1)],
    );

    let output = tools
        .command()
        .args(["--only", "mas", "--yes"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        mas_account()
            .to_vec()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );

    let text = stdout(&output);
    assert!(
        text.contains("Mac App Store"),
        "the step is not in the summary: {text}"
    );
    assert!(
        text.contains("not signed in"),
        "the summary does not name the reason: {text}"
    );
    assert!(
        text.contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {text}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_outdated_apps_are_reported_in_check_mode() {
    let tools = StubTools::new(&["mas"]);
    tools.set_responses(
        "mas",
        &[
            StubOutput::stdout("someone@example.com\n"),
            StubOutput::stdout("409203825  Numbers  (4.2 -> 4.3.1)\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--only", "mas", "--check"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        mas_check_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );

    let text = stdout(&output);
    assert!(
        text.contains("Numbers 4.2 -> 4.3.1"),
        "the outdated app is not in the report: {text}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_filter_keeps_only_the_selected_app_by_name() {
    let tools = StubTools::new(&["mas"]);
    // Write a config file that includes only "Numbers" in packages.
    let config_text = "[include]\npackages = [\"Numbers\"]\n";
    std::fs::create_dir_all(tools.config_path().parent().unwrap()).unwrap();
    std::fs::write(tools.config_path(), config_text).unwrap();

    tools.set_responses(
        "mas",
        &[
            StubOutput::stdout("someone@example.com\n"),
            StubOutput::stdout(
                "409203825  Numbers  (4.2 -> 4.3.1)\n409183694  Keynote  (7.2 -> 7.3.1)\n",
            ),
            StubOutput::stdout(""),
            StubOutput::stdout(""),
        ],
    );

    let output = tools
        .command()
        .args(["--only", "mas", "--yes"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("Numbers"),
        "the selected app should be upgraded: {}",
        stdout(&output)
    );
    // Only Numbers is upgraded (by its id 409203825).
    assert!(
        stdout(&output).contains("mas: upgrade 409203825"),
        "the upgrade should use the app store id: {}",
        stdout(&output)
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_step_is_not_available_on_this_platform() {
    let tools = StubTools::new(&["mas"]);

    let output = tools
        .command()
        .args(["--only", "mas"])
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
    let tools = StubTools::new(&["mas"]);

    let output = tools
        .command()
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("mas"), "listing: {text}");
    assert!(
        text.contains("not supported on this platform"),
        "listing: {text}"
    );
    assert!(tools.calls().is_empty(), "the step must not be detected");
}
