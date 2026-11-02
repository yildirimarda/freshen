//! Parity tests for the zypper step.
//!
//! Every test runs the built binary against a stub `zypper` executable and
//! asserts the exact command transcript of the `zypper refresh` + `zypper
//! update -y` block. No real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "zypper"])
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
fn zypper_refreshes_and_updates() {
    let tools = StubTools::new(&["zypper"]);
    tools.set_responses(
        "zypper",
        &[
            StubOutput::new(), // zypper refresh
            StubOutput::new(), // zypper update -y
            StubOutput::new(), // zypper ps -s
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::zypper_restart_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn zypper_reports_services_needing_a_restart() {
    let tools = StubTools::new(&["zypper"]);
    tools.set_responses(
        "zypper",
        &[
            StubOutput::new(), // zypper refresh
            StubOutput::new(), // zypper update -y
            StubOutput::new().with_stdout("ssh.service\npostgresql.service\n"), // zypper ps -s
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::zypper_restart_transcript());
    assert!(
        stdout(&output).contains("ssh.service"),
        "stdout: {}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("postgresql.service"),
        "stdout: {}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("service needs restart"),
        "stdout: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_zypper_refresh_still_attempts_the_update_and_fails_the_step() {
    let tools = StubTools::new(&["zypper"]);
    tools.set_responses(
        "zypper",
        &[
            StubOutput::new().with_status(1), // zypper refresh fails
            StubOutput::new(),                // zypper update -y still runs
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::zypper_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_zypper_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools);

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
fn check_reports_zypper_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["zypper"]);

    let output = tools
        .command()
        .args(["--check", "--only", "zypper"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

fn index_of(calls: &[String], call: &str) -> usize {
    calls
        .iter()
        .position(|recorded| recorded == call)
        .unwrap_or_else(|| panic!("missing call `{call}` in {calls:?}"))
}

fn run_docker(tools: &StubTools, live_restore: &str) -> Output {
    tools
        .command()
        .env("FRESHEN_LIVE_RESTORE", live_restore)
        .args(["--container-runtime", "docker", "--only", "zypper"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn install_docker(tools: &StubTools) {
    tools.set_script(
        "docker",
        r#"case "$*" in
  "info --format {{.LiveRestoreEnabled}}")
    printf '%s\n' "$FRESHEN_LIVE_RESTORE"
    ;;
esac"#,
    );
}

fn install_systemctl(tools: &StubTools) {
    tools.set_script("systemctl", "exit 0");
}

fn install_zypper_docker_upgrade(tools: &StubTools) {
    tools.set_script(
        "zypper",
        r#"case "$*" in
  "list-updates")
    printf 'Repository Name Version Arch\nsummary.repo docker 27.1.1-1 x86_64\n'
    ;;
  "refresh")
    exit 0
    ;;
  "update -y")
    exit 0
    ;;
esac"#,
    );
}

fn install_zypper_no_docker_upgrade(tools: &StubTools) {
    tools.set_script(
        "zypper",
        r#"case "$*" in
  "list-updates")
    printf 'Repository Name Version Arch\nsummary.repo curl 7.81.0-1 x86_64\n'
    ;;
  "refresh")
    exit 0
    ;;
  "update -y")
    exit 0
    ;;
esac"#,
    );
}

#[test]
#[cfg(target_os = "linux")]
fn zypper_upgrading_docker_stops_and_restarts_docker_engine() {
    let tools = StubTools::new(&["zypper", "docker", "systemctl"]);
    install_zypper_docker_upgrade(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    let output = run_docker(&tools, "false");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let stop = index_of(&calls, "systemctl stop docker");
    let refresh = index_of(&calls, "zypper refresh");
    let update = index_of(&calls, "zypper update -y");
    let restart = index_of(&calls, "systemctl restart docker");

    assert!(stop < refresh, "calls: {calls:?}");
    assert!(refresh < update, "calls: {calls:?}");
    assert!(update < restart, "calls: {calls:?}");

    let text = stdout(&output);
    assert!(
        text.contains(
            "warning: Docker live-restore is off; restarting Docker will stop running containers."
        ),
        "output: {text}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn zypper_not_upgrading_docker_does_not_stop_docker_engine() {
    let tools = StubTools::new(&["zypper", "docker", "systemctl"]);
    install_zypper_no_docker_upgrade(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    let output = run_docker(&tools, "false");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        !calls.iter().any(|call| call == "systemctl stop docker"),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == "systemctl restart docker"),
        "calls: {calls:?}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn zypper_upgrading_docker_with_live_restore_does_not_warn() {
    let tools = StubTools::new(&["zypper", "docker", "systemctl"]);
    install_zypper_docker_upgrade(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    let output = run_docker(&tools, "true");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("live-restore is off"), "output: {text}");
}
