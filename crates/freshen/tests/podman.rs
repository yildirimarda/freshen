//! Stub-based tests for the Podman machine on macOS.
//!
//! The runtime is detected from `DOCKER_HOST`, its state comes from
//! `podman machine list --format json`, and it is stopped with
//! `podman machine stop` before the `podman` formula is upgraded and started
//! again with `podman machine start` afterwards. Standalone containers are
//! listed and restored through the `podman` CLI, so a Podman-only host without
//! the Docker CLI still restores its containers. No real Podman is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::{StubTools, COMPOSE_JSON, PS_OUTPUT};

const ALL_STEPS: &str = "brew,brew-formulae,brew-services,brew-casks,brew-autoremove,brew-cleanup,brew-reopen,colima-stop,colima-start,docker";

const MACHINE_RUNNING: &str = r#"[{"Name":"podman-machine-default","Running":true}]"#;
const MACHINE_STOPPED: &str = r#"[{"Name":"podman-machine-default","Running":false}]"#;

/// `podman ps --format ...` with the same tab-separated shape as `docker ps`.
const PODMAN_PS_CALL: &str =
    "podman ps --format {{.ID}}\t{{.Names}}\t{{.Label \"com.docker.compose.project\"}}";

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

fn install_podman(tools: &StubTools) {
    tools.set_script(
        "podman",
        r#"case "$*" in
  "compose version")
    exit 0
    ;;
  "compose ls --format json")
    printf '%s' "$FRESHEN_COMPOSE_JSON"
    ;;
  "machine list --format json")
    printf '%s' "$FRESHEN_PODMAN_MACHINES"
    ;;
  "machine stop")
    exit "${FRESHEN_PODMAN_STOP_STATUS:-0}"
    ;;
  "machine start")
    exit "${FRESHEN_PODMAN_START_STATUS:-0}"
    ;;
  "image prune -f")
    exit "${FRESHEN_PODMAN_PRUNE_STATUS:-0}"
    ;;
  "ps --format "*)
    printf '%s' "$FRESHEN_PS_OUTPUT"
    ;;
  "stop def"|"stop ghi")
    exit 0
    ;;
  "start def"|"start ghi")
    exit 0
    ;;
esac"#,
    );
}

fn run(tools: &StubTools, outdated_formulae: &str, machines: &str) -> Output {
    tools
        .command()
        .env(
            "DOCKER_HOST",
            "unix:///Users/me/.local/share/containers/podman/machine/podman.sock",
        )
        .env("FRESHEN_OUTDATED_FORMULAE", outdated_formulae)
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_PODMAN_MACHINES", machines)
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
fn stops_the_podman_machine_before_its_upgrade_and_restores_containers_afterwards() {
    // A Podman-only host: no Docker CLI on PATH, yet containers are still
    // listed and restored through `podman ps` / `podman stop` / `podman start`.
    let tools = StubTools::new(&["brew", "podman"]);
    install_brew(&tools);
    install_podman(&tools);

    let output = run(&tools, "podman\n", MACHINE_RUNNING);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let stop = index_of(&calls, "podman machine stop");
    let upgrade = index_of(&calls, "brew upgrade --formula");
    let start = index_of(&calls, "podman machine start");
    let podman_web = |action: &str| {
        format!("podman compose -p web -f /srv/web/compose.yml -f /srv/web/override.yml {action}")
    };
    let podman_db = |action: &str| format!("podman compose -p db -f /srv/db/compose.yml {action}");

    assert!(
        index_of(&calls, "podman machine list --format json") < stop,
        "calls: {calls:?}"
    );
    assert!(stop < upgrade, "calls: {calls:?}");
    assert!(upgrade < start, "calls: {calls:?}");
    assert!(index_of(&calls, PODMAN_PS_CALL) < stop, "calls: {calls:?}");
    assert!(
        index_of(&calls, &podman_web("down")) < stop,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, &podman_db("down")) < stop,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, "podman stop def") < stop,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, "podman stop ghi") < stop,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, "podman start def") > start,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, "podman start ghi") > start,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, &podman_web("up -d")) > start,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, &podman_db("up -d")) > start,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, &podman_web("pull --quiet")) > start,
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"podman image prune -f".to_owned()),
        "calls: {calls:?}"
    );
    let prune = index_of(&calls, "podman image prune -f");
    assert!(prune > start, "calls: {calls:?}");
    assert!(
        prune > index_of(&calls, &podman_web("pull --quiet")),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("podman-machine is outdated and podman machine is running"),
        "output: {text}"
    );
    assert!(
        text.contains("Starting podman machine and containers"),
        "output: {text}"
    );
}

#[test]
fn a_stopped_podman_machine_is_not_started_and_containers_are_left_alone() {
    let tools = StubTools::new(&["brew", "podman"]);
    install_brew(&tools);
    install_podman(&tools);

    let output = run(&tools, "podman\n", MACHINE_STOPPED);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();

    assert!(
        !calls.iter().any(|call| call == "podman machine stop"),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == "podman machine start"),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("podman stop")),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("podman compose -p")),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("warning: podman machine is not running, skipping Docker image updates."),
        "output: {text}"
    );
}

#[test]
fn an_up_to_date_podman_is_never_stopped() {
    let tools = StubTools::new(&["brew", "podman"]);
    install_brew(&tools);
    install_podman(&tools);

    let output = run(&tools, "", MACHINE_RUNNING);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        !calls.iter().any(|call| call == "podman machine stop"),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == "podman machine start"),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"podman image prune -f".to_owned()),
        "calls: {calls:?}"
    );
}

#[test]
fn a_failing_podman_machine_start_is_recorded_and_containers_are_not_restored() {
    let tools = StubTools::new(&["brew", "podman"]);
    install_brew(&tools);
    install_podman(&tools);

    let output = tools
        .command()
        .env(
            "DOCKER_HOST",
            "unix:///Users/me/.local/share/containers/podman/machine/podman.sock",
        )
        .env("FRESHEN_OUTDATED_FORMULAE", "podman\n")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_PODMAN_MACHINES", MACHINE_RUNNING)
        .env("FRESHEN_PODMAN_START_STATUS", "1")
        .env("FRESHEN_COMPOSE_JSON", "[]")
        .env("FRESHEN_PS_OUTPUT", "")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"podman machine start".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("podman compose -p")),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("warning: podman machine did not start; containers were NOT restarted"),
        "output: {text}"
    );
    assert!(text.contains("1 failed"), "summary: {text}");
}

#[test]
fn uses_podman_compose_when_docker_has_no_compose_plugin() {
    let tools = StubTools::new(&["brew", "docker", "podman"]);
    install_brew(&tools);
    tools.set_script(
        "docker",
        r#"case "$*" in
  "compose version")
    exit 1
    ;;
  "ps --format "*)
    printf '%s' "$FRESHEN_PS_OUTPUT"
    ;;
esac"#,
    );
    tools.set_script(
        "podman",
        r#"case "$*" in
  "compose version")
    exit 0
    ;;
  "compose ls --format json")
    printf '%s' "$FRESHEN_COMPOSE_JSON"
    ;;
  "machine list --format json")
    printf '%s' "$FRESHEN_PODMAN_MACHINES"
    ;;
esac"#,
    );

    let output = run(&tools, "podman\n", MACHINE_RUNNING);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let podman_compose = |action: &str| {
        format!("podman compose -p web -f /srv/web/compose.yml -f /srv/web/override.yml {action}")
    };

    assert!(
        calls.contains(&"podman compose version".to_owned()),
        "calls: {calls:?}"
    );
    assert!(calls.contains(&podman_compose("down")), "calls: {calls:?}");
    assert!(calls.contains(&podman_compose("up -d")), "calls: {calls:?}");
    assert!(
        calls.contains(&podman_compose("pull --quiet")),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose -p")),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(text.contains("podman compose down: web"), "output: {text}");
    assert!(
        text.contains("podman compose pull + up -d: web"),
        "output: {text}"
    );
}

#[test]
fn prefers_podman_compose_even_when_docker_compose_is_installed() {
    let tools = StubTools::new(&["brew", "docker", "podman"]);
    install_brew(&tools);
    tools.set_script(
        "docker",
        r#"case "$*" in
  "compose version")
    exit 0
    ;;
  "compose ls --format json")
    printf '%s' "$FRESHEN_COMPOSE_JSON"
    ;;
  "ps --format "*)
    printf '%s' "$FRESHEN_PS_OUTPUT"
    ;;
esac"#,
    );
    tools.set_script(
        "podman",
        r#"case "$*" in
  "compose version")
    exit 0
    ;;
  "compose ls --format json")
    printf '%s' "$FRESHEN_COMPOSE_JSON"
    ;;
  "machine list --format json")
    printf '%s' "$FRESHEN_PODMAN_MACHINES"
    ;;
  "machine stop")
    exit 0
    ;;
  "machine start")
    exit 0
    ;;
esac"#,
    );

    let output = tools
        .command()
        .env(
            "DOCKER_HOST",
            "unix:///Users/me/.local/share/containers/podman/machine/podman.sock",
        )
        .env("FRESHEN_OUTDATED_FORMULAE", "podman\n")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_PODMAN_MACHINES", MACHINE_RUNNING)
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .args([
            "--container-runtime",
            "podman",
            "--only",
            ALL_STEPS,
            "--cleanup",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let podman_compose = |action: &str| {
        format!("podman compose -p web -f /srv/web/compose.yml -f /srv/web/override.yml {action}")
    };

    assert!(
        calls.contains(&"podman compose version".to_owned()),
        "calls: {calls:?}"
    );
    assert!(calls.contains(&podman_compose("down")), "calls: {calls:?}");
    assert!(
        calls.contains(&podman_compose("pull --quiet")),
        "calls: {calls:?}"
    );
    assert!(calls.contains(&podman_compose("up -d")), "calls: {calls:?}");
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose -p")),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == "docker compose version"),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(text.contains("podman compose down: web"), "output: {text}");
    assert!(
        text.contains("podman compose pull + up -d: web"),
        "output: {text}"
    );
}
