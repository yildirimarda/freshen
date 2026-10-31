//! Parity tests for the apt step.
//!
//! Every test runs the built binary against a stub `apt-get` executable and
//! asserts the exact command transcript of the `apt-get update` + `apt-get
//! upgrade -y` block. No real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools, steps: &str) -> Output {
    tools
        .command()
        .args(["--only", steps, "--cleanup"])
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
fn apt_updates_and_upgrades() {
    let tools = StubTools::new(&["apt-get"]);
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check, up to date)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
        ],
    );

    let output = run(&tools, "apt");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::apt_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_apt_update_still_attempts_the_upgrade_and_fails_the_step() {
    let tools = StubTools::new(&["apt-get"]);
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(),                // apt-get -s upgrade (check, up to date)
            StubOutput::new().with_status(1), // apt-get update fails
            StubOutput::new(),                // apt-get upgrade -y still runs
        ],
    );

    let output = run(&tools, "apt");

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::apt_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_apt_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, "apt");

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
fn check_reports_outdated_apt_packages_and_exits_2() {
    let tools = StubTools::new(&["apt-get"]);
    tools.set_responses(
        "apt-get",
        &[StubOutput::stdout(
            "Reading package lists... Done\n\
             Building dependency tree... Done\n\
             Reading state information... Done\n\
             The following packages will be upgraded:\n\
               curl git\n\
             Inst curl [7.81.0] (7.88.0 stable [amd64])\n\
             Inst git [2.34.1] (2.40.1 jammy [amd64])\n\
             2 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.\n",
        )],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "apt"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["apt-get -s upgrade".to_string()]);
    let text = stdout(&output);
    assert!(text.contains("apt:"), "output: {text}");
    assert!(text.contains("curl 7.81.0 -> 7.88.0"), "output: {text}");
    assert!(text.contains("git 2.34.1 -> 2.40.1"), "output: {text}");
    assert!(
        text.contains("2 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn check_reports_apt_up_to_date_without_running_anything_else() {
    let tools = StubTools::new(&["apt-get"]);
    tools.set_responses("apt-get", &[StubOutput::new()]);

    let output = tools
        .command()
        .args(["--check", "--only", "apt"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["apt-get -s upgrade".to_string()]);
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
#[cfg(target_os = "linux")]
fn apt_reports_restart_needed_when_needrestart_says_so() {
    let tools = StubTools::new(&["apt-get", "needrestart"]);
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check, up to date)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
        ],
    );
    tools.set_responses(
        "needrestart",
        &[StubOutput::new().with_status(1)], // needrestart -b: restart needed
    );

    let output = run(&tools, "apt");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "apt-get -s upgrade",
            "apt-get update",
            "apt-get upgrade -y",
            "needrestart -b",
        ]
    );
    assert!(
        stdout(&output).contains("restart needed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn apt_does_not_report_restart_when_needrestart_says_no() {
    let tools = StubTools::new(&["apt-get", "needrestart"]);
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check, up to date)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
        ],
    );
    tools.set_responses(
        "needrestart",
        &[StubOutput::new()], // needrestart -b: exit 0, no restart needed
    );

    let output = run(&tools, "apt");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains("restart needed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn apt_does_not_report_restart_when_needrestart_is_not_installed() {
    let tools = StubTools::new(&["apt-get"]);
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check, up to date)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
        ],
    );

    let output = run(&tools, "apt");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains("restart needed"),
        "summary: {}",
        stdout(&output)
    );
    assert_eq!(tools.step_calls(), support::apt_transcript());
}
