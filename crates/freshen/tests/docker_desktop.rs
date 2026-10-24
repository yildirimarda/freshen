//! Stub-based tests for Docker Desktop on macOS.
//!
//! The runtime is detected from `DOCKER_HOST`, stopped before its cask is
//! upgraded and started again afterwards. No real Docker Desktop is touched.

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
  "desktop version"|"desktop status"|"desktop start")
    exit 0
    ;;
  "desktop stop")
    exit "${FRESHEN_DOCKER_STOP_STATUS:-0}"
    ;;
  "compose ls --format json")
    printf '%s' "$FRESHEN_COMPOSE_JSON"
    ;;
  "ps --format "*)
    printf '%s' "$FRESHEN_PS_OUTPUT"
    ;;
esac"#,
    );
}

fn run(tools: &StubTools, outdated_casks: &str, docker_stop_status: &str) -> Output {
    tools
        .command()
        .env("DOCKER_HOST", "unix:///Users/me/.docker/run/docker.sock")
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", outdated_casks)
        .env("FRESHEN_DOCKER_STOP_STATUS", docker_stop_status)
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
fn stops_docker_desktop_before_its_cask_and_restores_containers_afterwards() {
    let tools = StubTools::new(&["brew", "docker", "osascript"]);
    install_brew(&tools);
    install_docker(&tools);

    let output = run(&tools, "docker-desktop\n", "0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let stop = index_of(&calls, "docker desktop stop");
    let upgrade = index_of(&calls, "brew upgrade --cask");
    let start = index_of(&calls, "docker desktop start");

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
        text.contains("docker-desktop is outdated and docker desktop is running"),
        "output: {text}"
    );
    assert!(
        text.contains("Starting docker desktop and containers"),
        "output: {text}"
    );
}

#[test]
fn falls_back_to_the_applescript_quit_when_the_cli_stop_fails() {
    let tools = StubTools::new(&["brew", "docker", "osascript"]);
    install_brew(&tools);
    install_docker(&tools);

    let output = run(&tools, "docker-desktop\n", "1");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let stop = index_of(&calls, "docker desktop stop");
    let applescript = calls
        .iter()
        .position(|call| call.starts_with("osascript "))
        .unwrap_or_else(|| panic!("no AppleScript quit in {calls:?}"));
    let upgrade = index_of(&calls, "brew upgrade --cask");
    let start = index_of(&calls, "docker desktop start");

    assert!(stop < applescript, "calls: {calls:?}");
    assert!(applescript < upgrade, "calls: {calls:?}");
    assert!(upgrade < start, "calls: {calls:?}");
    assert!(
        calls.contains(&"docker start def".to_owned()),
        "calls: {calls:?}"
    );
    assert!(calls.contains(&web("up -d")), "calls: {calls:?}");
}

#[test]
fn an_up_to_date_docker_desktop_is_never_stopped() {
    let tools = StubTools::new(&["brew", "docker", "osascript"]);
    install_brew(&tools);
    install_docker(&tools);

    let output = run(&tools, "", "0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker desktop stop")),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker desktop start")),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"docker image prune -f".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        stdout(&output).contains("No running compose projects.")
            || stdout(&output).contains("docker compose pull + up -d: web"),
        "output: {}",
        stdout(&output)
    );
}
