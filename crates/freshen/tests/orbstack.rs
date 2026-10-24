//! Stub-based tests for OrbStack on macOS.
//!
//! The runtime is detected from `DOCKER_HOST`, its state comes from
//! `orbctl status`, and it is stopped before its cask is upgraded and started
//! again afterwards. No real OrbStack is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::{db, web, StubTools, COMPOSE_JSON, PS_CALL, PS_OUTPUT};

const ALL_STEPS: &str = "brew,brew-formulae,brew-services,brew-casks,brew-autoremove,brew-cleanup,brew-reopen,colima-stop,colima-start,docker";

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn install_brew(tools: &StubTools) {
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula --quiet"|"outdated --formula")
    printf '%s' "$FRESHEN_OUTDATED_FORMULAE"
    ;;
  "outdated --cask --greedy --quiet"|"outdated --cask --greedy")
    printf '%s' "$FRESHEN_OUTDATED_CASKS"
    ;;
  "info --cask --json=v2 "*)
    printf '%s' '{"casks":[]}'
    ;;
  "upgrade --formula")
    exit "${FRESHEN_BREW_UPGRADE_STATUS:-0}"
    ;;
  "autoremove --dry-run")
    printf '==> Would autoremove 1 unneeded formula:\nold-dependency\n'
    ;;
  "cleanup --dry-run --prune=all")
    printf 'Would remove: /tmp/old (1.2MB)\n'
    ;;
esac"#,
    );
}

fn install_docker(tools: &StubTools) {
    tools.set_script(
        "docker",
        r#"case "$*" in
  "compose ls --format json")
    printf '%s' "$FRESHEN_COMPOSE_JSON"
    ;;
  "ps --format "*)
    printf '%s' "$FRESHEN_PS_OUTPUT"
    ;;
esac"#,
    );
}

fn install_orbctl(tools: &StubTools) {
    tools.set_script(
        "orbctl",
        r#"case "$*" in
  "status")
    printf '%s\n' "${FRESHEN_ORBCTL_STATUS_TEXT:-Running}"
    exit "${FRESHEN_ORBCTL_STATUS:-0}"
    ;;
  "stop")
    exit "${FRESHEN_ORBCTL_STOP_STATUS:-0}"
    ;;
  "start")
    exit "${FRESHEN_ORBCTL_START_STATUS:-0}"
    ;;
esac"#,
    );
}

fn run(tools: &StubTools, outdated_casks: &str, status_text: &str, status_code: &str) -> Output {
    tools
        .command()
        .env("DOCKER_HOST", "unix:///Users/me/.orbstack/run/docker.sock")
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", outdated_casks)
        .env("FRESHEN_ORBCTL_STATUS_TEXT", status_text)
        .env("FRESHEN_ORBCTL_STATUS", status_code)
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn index_of(calls: &[String], call: &str) -> usize {
    calls
        .iter()
        .position(|recorded| recorded == call)
        .unwrap_or_else(|| panic!("missing call `{call}` in {calls:?}"))
}

#[test]
fn stops_orbstack_before_its_cask_and_restores_containers_afterwards() {
    let tools = StubTools::new(&["brew", "docker", "orbctl"]);
    install_brew(&tools);
    install_docker(&tools);
    install_orbctl(&tools);

    let output = run(&tools, "orbstack\n", "Running", "0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let stop = index_of(&calls, "orbctl stop");
    let upgrade = index_of(&calls, "brew upgrade --cask");
    let start = index_of(&calls, "orbctl start");

    assert!(index_of(&calls, "orbctl status") < stop, "calls: {calls:?}");
    assert!(stop < upgrade, "calls: {calls:?}");
    assert!(upgrade < start, "calls: {calls:?}");
    assert!(index_of(&calls, PS_CALL) < stop, "calls: {calls:?}");
    assert!(index_of(&calls, &web("down")) < stop, "calls: {calls:?}");
    assert!(index_of(&calls, &db("down")) < stop, "calls: {calls:?}");
    assert!(
        index_of(&calls, "docker stop def") < stop,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, "docker stop ghi") < stop,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, "docker start def") > start,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, "docker start ghi") > start,
        "calls: {calls:?}"
    );
    assert!(index_of(&calls, &web("up -d")) > start, "calls: {calls:?}");
    assert!(index_of(&calls, &db("up -d")) > start, "calls: {calls:?}");
    assert!(
        index_of(&calls, &web("pull --quiet")) > start,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, "docker image prune -f") > start,
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("orbstack is outdated and orbctl is running"),
        "output: {text}"
    );
    assert!(
        text.contains("Starting orbctl and containers"),
        "output: {text}"
    );
}

#[test]
fn a_stopped_orbstack_is_not_started_and_containers_are_left_alone() {
    let tools = StubTools::new(&["brew", "docker", "orbctl"]);
    install_brew(&tools);
    install_docker(&tools);
    install_orbctl(&tools);

    let output = run(&tools, "orbstack\n", "Stopped", "0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();

    assert!(
        !calls.iter().any(|call| call == "orbctl stop"),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == "orbctl start"),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("docker stop")),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose -p")),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("warning: orbctl is not running, skipping Docker image updates."),
        "output: {text}"
    );
}

#[test]
fn an_up_to_date_orbstack_is_never_stopped() {
    let tools = StubTools::new(&["brew", "docker", "orbctl"]);
    install_brew(&tools);
    install_docker(&tools);
    install_orbctl(&tools);

    let output = run(&tools, "", "Running", "0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        !calls.iter().any(|call| call == "orbctl stop"),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == "orbctl start"),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"docker image prune -f".to_owned()),
        "calls: {calls:?}"
    );
}

#[test]
fn a_failing_orbstack_start_is_recorded_and_containers_are_not_restored() {
    let tools = StubTools::new(&["brew", "docker", "orbctl"]);
    install_brew(&tools);
    install_docker(&tools);
    install_orbctl(&tools);

    let output = tools
        .command()
        .env("DOCKER_HOST", "unix:///Users/me/.orbstack/run/docker.sock")
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "orbstack\n")
        .env("FRESHEN_ORBCTL_STATUS_TEXT", "Running")
        .env("FRESHEN_ORBCTL_STATUS", "0")
        .env("FRESHEN_ORBCTL_START_STATUS", "1")
        .env("FRESHEN_COMPOSE_JSON", "[]")
        .env("FRESHEN_PS_OUTPUT", "")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"orbctl start".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose -p")),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("warning: orbctl did not start; containers were NOT restarted"),
        "output: {text}"
    );
    assert!(text.contains("1 failed"), "summary: {text}");
}
