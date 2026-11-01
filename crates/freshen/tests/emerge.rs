//! Parity tests for the emerge step.
//!
//! Every test runs the built binary against a stub `emerge` executable and
//! asserts the exact command transcript of the `emerge --sync` +
//! `emerge -uDN @world` block. No real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools, steps: &str) -> Output {
    tools
        .command()
        .args(["--only", steps])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
#[cfg(target_os = "linux")]
fn emerge_syncs_and_upgrades_world() {
    let tools = StubTools::new(&["emerge"]);
    tools.set_responses(
        "emerge",
        &[
            StubOutput::new(), // emerge -puDN @world (check, run before the upgrade)
            StubOutput::new(), // emerge --sync
            StubOutput::new(), // emerge -uDN @world
        ],
    );

    let output = run(&tools, "emerge");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::emerge_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_emerge_sync_still_attempts_the_world_upgrade_and_fails_the_step() {
    let tools = StubTools::new(&["emerge"]);
    tools.set_responses(
        "emerge",
        &[
            StubOutput::new(),                // emerge -puDN @world (check)
            StubOutput::new().with_status(1), // emerge --sync fails
            StubOutput::new(),                // emerge -uDN @world still runs
        ],
    );

    let output = run(&tools, "emerge");

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::emerge_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_emerge_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, "emerge");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn check_reports_outdated_emerge_packages_and_exits_2() {
    let tools = StubTools::new(&["emerge"]);
    tools.set_responses(
        "emerge",
        &[StubOutput::stdout(
            "Calculating dependencies... done!\n\
             [ebuild  U     ] app-editors/vim-9.0.1000::gentoo [9.0.999::gentoo] USE=\"X\"\n\
             [ebuild  U     ] sys-apps/portage-3.0.52::gentoo [3.0.51::gentoo]\n\
             All done.\n",
        )],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "emerge"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["emerge -puDN @world".to_string()]);
    let text = stdout(&output);
    assert!(text.contains("emerge:"), "output: {text}");
    assert!(
        text.contains("app-editors/vim 9.0.999 -> 9.0.1000"),
        "output: {text}"
    );
    assert!(
        text.contains("2 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn check_reports_emerge_up_to_date_without_running_anything_else() {
    let tools = StubTools::new(&["emerge"]);
    tools.set_responses("emerge", &[StubOutput::new()]);

    let output = tools
        .command()
        .args(["--check", "--only", "emerge"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["emerge -puDN @world".to_string()]);
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
