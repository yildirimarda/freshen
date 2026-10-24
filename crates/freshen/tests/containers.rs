//! Parity tests for the Colima/lima and Docker orchestration.
//!
//! Every test runs the built binary against stub `brew`, `colima` and `docker`
//! executables and asserts the exact command transcript of the corresponding
//! block in `update-all.sh`. No real container runtime is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::{
    brew_autoremove, brew_cask_query, brew_cleanup, brew_formula_query, brew_formula_upgrade,
    brew_update, colima_start, colima_stop, db, docker_image_prune, docker_start_containers,
    docker_stop_containers, docker_version, web, StubTools, ALL_STEPS, COMPOSE_JSON, PS_CALL,
    PS_OUTPUT,
};

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
  "outdated --formula --quiet")
    printf '%s' "$FRESHEN_OUTDATED_FORMULAE"
    ;;
  "outdated --cask --greedy --quiet")
    printf '%s' "$FRESHEN_OUTDATED_CASKS"
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

fn install_colima(tools: &StubTools) {
    tools.set_script(
        "colima",
        r#"case "$*" in
  "status")
    exit "${FRESHEN_COLIMA_STATUS:-0}"
    ;;
  "start")
    exit "${FRESHEN_COLIMA_START_STATUS:-0}"
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

struct Scenario<'a> {
    outdated_formulae: &'a str,
    outdated_casks: &'a str,
    brew_upgrade_status: &'a str,
    colima_status: &'a str,
    colima_start_status: &'a str,
    compose_json: &'a str,
    ps_output: &'a str,
}

impl Default for Scenario<'_> {
    fn default() -> Self {
        Self {
            outdated_formulae: "colima\n",
            outdated_casks: "",
            brew_upgrade_status: "0",
            colima_status: "0",
            colima_start_status: "0",
            compose_json: COMPOSE_JSON,
            ps_output: PS_OUTPUT,
        }
    }
}

fn run(tools: &StubTools, scenario: &Scenario<'_>) -> Output {
    tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", scenario.outdated_formulae)
        .env("FRESHEN_OUTDATED_CASKS", scenario.outdated_casks)
        .env("FRESHEN_BREW_UPGRADE_STATUS", scenario.brew_upgrade_status)
        .env("FRESHEN_COLIMA_STATUS", scenario.colima_status)
        .env("FRESHEN_COLIMA_START_STATUS", scenario.colima_start_status)
        .env("FRESHEN_COMPOSE_JSON", scenario.compose_json)
        .env("FRESHEN_PS_OUTPUT", scenario.ps_output)
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn stops_colima_and_restores_containers_around_the_formula_upgrade() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let output = run(&tools, &Scenario::default());

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let mut expected = Vec::new();
    expected.extend(docker_version().iter().map(|&s| s.to_string()));
    expected.extend(brew_update().iter().map(|&s| s.to_string()));
    expected.extend(brew_formula_query().iter().map(|&s| s.to_string()));
    expected.push("colima status".to_string());
    expected.push("docker compose ls --format json".to_string());
    expected.push(PS_CALL.to_string());
    expected.push(web("down"));
    expected.push(db("down"));
    expected.extend(docker_stop_containers().iter().map(|&s| s.to_string()));
    expected.extend(colima_stop().iter().map(|&s| s.to_string()));
    expected.extend(brew_formula_upgrade().iter().map(|&s| s.to_string()));
    expected.extend(colima_start().iter().map(|&s| s.to_string()));
    expected.extend(docker_start_containers().iter().map(|&s| s.to_string()));
    expected.push(web("up -d"));
    expected.push(db("up -d"));
    expected.extend(brew_cask_query().iter().map(|&s| s.to_string()));
    expected.extend(brew_autoremove().iter().map(|&s| s.to_string()));
    expected.extend(brew_cleanup().iter().map(|&s| s.to_string()));
    expected.push("colima status".to_string());
    expected.push("docker compose ls --format json".to_string());
    expected.push(web("pull --quiet"));
    expected.push(web("up -d"));
    expected.push(db("pull --quiet"));
    expected.push(db("up -d"));
    expected.extend(docker_image_prune().iter().map(|&s| s.to_string()));
    assert_eq!(tools.step_calls(), expected);

    let text = stdout(&output);
    assert!(
        text.contains(
            "colima/lima is outdated and colima is running: stopping containers and colima"
        ),
        "output: {text}"
    );
    assert!(text.contains("docker compose down: web"), "output: {text}");
    assert!(text.contains("docker stop: lonely"), "output: {text}");
    assert!(
        text.contains("Starting colima and containers"),
        "output: {text}"
    );
    assert!(text.contains("docker start: lonely"), "output: {text}");
    assert!(
        text.contains("Docker: pull newer images for running compose projects"),
        "output: {text}"
    );
    assert!(
        text.contains("Docker: prune dangling images"),
        "output: {text}"
    );
}

#[test]
fn up_to_date_colima_is_never_stopped() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let output = run(
        &tools,
        &Scenario {
            outdated_formulae: "",
            compose_json: "[]",
            ..Scenario::default()
        },
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let mut expected = Vec::new();
    expected.extend(docker_version().iter().map(|&s| s.to_string()));
    expected.extend(brew_update().iter().map(|&s| s.to_string()));
    expected.extend(brew_formula_query().iter().map(|&s| s.to_string()));
    expected.push("brew outdated --formula".to_string());
    expected.extend(brew_formula_query().iter().map(|&s| s.to_string()));
    expected.extend(brew_cask_query().iter().map(|&s| s.to_string()));
    expected.extend(brew_autoremove().iter().map(|&s| s.to_string()));
    expected.extend(brew_cleanup().iter().map(|&s| s.to_string()));
    expected.push("colima status".to_string());
    expected.push("docker compose ls --format json".to_string());
    expected.extend(docker_image_prune().iter().map(|&s| s.to_string()));
    assert_eq!(tools.step_calls(), expected);
    assert!(
        stdout(&output).contains("No running compose projects."),
        "output: {}",
        stdout(&output)
    );
    assert!(
        !stdout(&output).contains("stopping containers and colima"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn a_stopped_colima_is_not_started_and_docker_updates_are_skipped() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let output = run(
        &tools,
        &Scenario {
            colima_status: "1",
            ..Scenario::default()
        },
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let mut expected = Vec::new();
    expected.extend(docker_version().iter().map(|&s| s.to_string()));
    expected.extend(brew_update().iter().map(|&s| s.to_string()));
    expected.extend(brew_formula_query().iter().map(|&s| s.to_string()));
    expected.push("colima status".to_string());
    expected.extend(brew_formula_upgrade().iter().map(|&s| s.to_string()));
    expected.extend(brew_cask_query().iter().map(|&s| s.to_string()));
    expected.extend(brew_autoremove().iter().map(|&s| s.to_string()));
    expected.extend(brew_cleanup().iter().map(|&s| s.to_string()));
    expected.push("colima status".to_string());
    assert_eq!(tools.step_calls(), expected);
    assert!(
        stdout(&output).contains("warning: colima is not running, skipping Docker image updates."),
        "output: {}",
        stdout(&output)
    );
    assert!(
        !tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("docker ") && call != "docker compose version"),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn colima_is_stopped_without_docker_and_the_docker_step_is_skipped() {
    let tools = StubTools::new(&["brew", "colima"]);
    install_brew(&tools);
    install_colima(&tools);

    let output = run(&tools, &Scenario::default());

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let mut expected = Vec::new();
    expected.extend(brew_update().iter().map(|&s| s.to_string()));
    expected.extend(brew_formula_query().iter().map(|&s| s.to_string()));
    expected.push("colima status".to_string());
    expected.extend(colima_stop().iter().map(|&s| s.to_string()));
    expected.extend(brew_formula_upgrade().iter().map(|&s| s.to_string()));
    expected.extend(colima_start().iter().map(|&s| s.to_string()));
    expected.extend(brew_cask_query().iter().map(|&s| s.to_string()));
    expected.extend(brew_autoremove().iter().map(|&s| s.to_string()));
    expected.extend(brew_cleanup().iter().map(|&s| s.to_string()));
    assert_eq!(tools.step_calls(), expected);
    assert!(
        stdout(&output).contains("1 skipped"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn a_failing_colima_start_is_recorded_and_containers_are_not_restored() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let output = run(
        &tools,
        &Scenario {
            colima_start_status: "1",
            compose_json: "[]",
            ..Scenario::default()
        },
    );

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"colima start".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("docker start")),
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
        text.contains("warning: colima did not start; containers were NOT restarted"),
        "output: {text}"
    );
    assert!(text.contains("1 failed"), "summary: {text}");
}

#[test]
fn containers_are_restored_even_when_the_formula_upgrade_fails() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let output = run(
        &tools,
        &Scenario {
            brew_upgrade_status: "1",
            ..Scenario::default()
        },
    );

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"colima start".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"docker start def".to_owned()),
        "calls: {calls:?}"
    );
    assert!(calls.contains(&web("up -d")), "calls: {calls:?}");
    assert!(
        calls.contains(&"docker image prune -f".to_owned()),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(text.contains("1 failed"), "summary: {text}");
    assert!(
        text.contains("Starting colima and containers"),
        "output: {text}"
    );
}

#[test]
fn falls_back_to_docker_compose_when_the_docker_plugin_is_missing() {
    let tools = StubTools::new(&["brew", "colima", "docker", "docker-compose"]);
    install_brew(&tools);
    install_colima(&tools);
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
        "docker-compose",
        r#"case "$*" in
  "ls --format json")
    printf '%s' "$FRESHEN_COMPOSE_JSON"
    ;;
esac"#,
    );

    let output = run(&tools, &Scenario::default());

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"docker compose version".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(
            &"docker-compose -p web -f /srv/web/compose.yml -f /srv/web/override.yml down"
                .to_owned()
        ),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(
            &"docker-compose -p web -f /srv/web/compose.yml -f /srv/web/override.yml up -d"
                .to_owned()
        ),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(
            &"docker-compose -p web -f /srv/web/compose.yml -f /srv/web/override.yml pull --quiet"
                .to_owned()
        ),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose -p")),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(text.contains("docker-compose down: web"), "output: {text}");
    assert!(
        text.contains("docker-compose pull + up -d: web"),
        "output: {text}"
    );
}
