//! Parity tests for the Colima/lima and Docker orchestration.
//!
//! Every test runs the built binary against stub `brew`, `colima` and `docker`
//! executables and asserts the exact command transcript of the corresponding
//! block in `update-all.sh`. No real container runtime is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;

use support::{
    brew_autoremove, brew_cask_query, brew_cleanup, brew_formula_query, brew_formula_upgrade,
    brew_update, colima_start, colima_stop, db, docker_builder_prune, docker_image_prune,
    docker_start_containers, docker_stop_containers, docker_version, project_ps, web, StubTools,
    ALL_STEPS, COMPOSE_JSON, PS_CALL, PS_OUTPUT,
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
  "compose version")
    exit 0
    ;;
  "compose ls --format json")
    printf '%s' "$FRESHEN_COMPOSE_JSON"
    ;;
  "ps --format "*)
    printf '%s' "$FRESHEN_PS_OUTPUT"
    ;;
  "ps --filter label=com.docker.compose.project="*)
    project=""
    for arg in "$@"; do
      case "$arg" in
        label=com.docker.compose.project=*)
          project="${arg#label=com.docker.compose.project=}"
          ;;
      esac
    done
    case "$project" in
      web) printf '%s' "$FRESHEN_CONTAINERS_WEB" ;;
      db)  printf '%s' "$FRESHEN_CONTAINERS_DB" ;;
      *)   printf '%s' "$FRESHEN_PS_OUTPUT" ;;
    esac
    ;;
  "inspect --format {{.State.Health.Status}} abc")
    printf '%s' "$FRESHEN_HEALTH_ABC"
    ;;
  "inspect --format {{.State.Health.Status}} def")
    printf '%s' "$FRESHEN_HEALTH_DEF"
    ;;
  "inspect --format {{.State.Health.Status}} ghi")
    # Test-only: the first poll reports "starting", later polls "healthy".
    state="${FRESHEN_STUB_STATE:?}"
    count_file="$state/docker-inspect-ghi.count"
    count=0
    [ -f "$count_file" ] && read -r count < "$count_file"
    count=$((count + 1))
    printf '%s\n' "$count" > "$count_file"
    if [ "$count" -eq 1 ]; then
      printf 'starting'
    else
      printf 'healthy'
    fi
    ;;
  "inspect --format {{.State.Health.Status}} "*)
    printf 'healthy'
    ;;
  "image prune -f")
    exit 0
  ;;
  "image prune -f --filter until=*)")
    exit 0
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

    let inspect_abc = "docker inspect --format {{.State.Health.Status}} abc";
    let inspect_def = "docker inspect --format {{.State.Health.Status}} def";

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "colima\n")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .env("FRESHEN_CONTAINERS_WEB", "abc\tweb-1\tweb")
        .env("FRESHEN_CONTAINERS_DB", "def\tdb-1\tdb")
        .env("FRESHEN_HEALTH_WAIT_SECS", "0")
        .env("FRESHEN_HEALTH_POLL_SECS", "1")
        .env("FRESHEN_HEALTH_ABC", "healthy")
        .env("FRESHEN_HEALTH_DEF", "healthy")
        .env("FRESHEN_HEALTH_GHI", "healthy")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

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
    expected.push(project_ps("web"));
    expected.push(inspect_abc.to_string());
    expected.push(db("up -d"));
    expected.push(project_ps("db"));
    expected.push(inspect_def.to_string());
    expected.extend(brew_cask_query().iter().map(|&s| s.to_string()));
    expected.extend(brew_autoremove().iter().map(|&s| s.to_string()));
    expected.extend(brew_cleanup().iter().map(|&s| s.to_string()));
    expected.push("colima status".to_string());
    expected.push("docker compose ls --format json".to_string());
    expected.push(project_ps("web"));
    expected.push(web("pull --quiet"));
    expected.push(web("up -d"));
    expected.push(project_ps("web"));
    expected.push(inspect_abc.to_string());
    expected.push(project_ps("db"));
    expected.push(db("pull --quiet"));
    expected.push(db("up -d"));
    expected.push(project_ps("db"));
    expected.push(inspect_def.to_string());
    expected.extend(docker_image_prune().iter().map(|&s| s.to_string()));
    expected.extend(docker_builder_prune().iter().map(|&s| s.to_string()));
    expected.push("docker system df".to_string());
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
    assert!(text.contains("Docker: prune build cache"), "output: {text}");
}

#[test]
fn healthy_containers_are_not_reported_after_up_d() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .env("FRESHEN_CONTAINERS_WEB", "abc\tweb-1\tweb")
        .env("FRESHEN_CONTAINERS_DB", "def\tdb-1\tdb")
        .env("FRESHEN_HEALTH_ABC", "healthy")
        .env("FRESHEN_HEALTH_DEF", "healthy")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(calls.contains(&project_ps("web")), "calls: {calls:?}");
    assert!(calls.contains(&project_ps("db")), "calls: {calls:?}");
    assert!(
        calls.contains(&"docker inspect --format {{.State.Health.Status}} abc".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"docker inspect --format {{.State.Health.Status}} def".to_owned()),
        "calls: {calls:?}"
    );
    let text = stdout(&output);
    assert!(!text.contains("did not become healthy"), "output: {text}");
}

#[test]
fn prunes_images_older_than_the_configured_age() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .args([
            "--only",
            ALL_STEPS,
            "--cleanup",
            "--prune-unused-older-than",
            "30",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"docker image prune -f --filter until=720h".to_owned()),
        "calls: {calls:?}"
    );
    let text = stdout(&output);
    assert!(
        text.contains("Docker: prune dangling images"),
        "output: {text}"
    );
}

#[test]
fn containers_that_do_not_become_healthy_are_reported() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    // A zero wait probes each container once and reports what is not healthy.
    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .env("FRESHEN_CONTAINERS_WEB", "abc\tweb-1\tweb")
        .env("FRESHEN_CONTAINERS_DB", "def\tdb-1\tdb")
        .env("FRESHEN_HEALTH_WAIT_SECS", "0")
        .env("FRESHEN_HEALTH_ABC", "unhealthy")
        .env("FRESHEN_HEALTH_DEF", "healthy")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("did not become healthy: web-1"),
        "output: {text}"
    );
    assert!(!text.contains("db-1"), "output: {text}");
}

#[test]
fn an_unhealthy_recreated_container_gets_a_per_container_rollback_hint() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    // A zero wait probes each container once and reports what is not healthy.
    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        // web-1 was running nginx@sha256:old before the recreation.
        .env(
            "FRESHEN_CONTAINERS_WEB",
            "abc\tweb-1\tweb\tnginx@sha256:old",
        )
        .env("FRESHEN_CONTAINERS_DB", "def\tdb-1\tdb")
        .env("FRESHEN_HEALTH_WAIT_SECS", "0")
        .env("FRESHEN_HEALTH_ABC", "unhealthy")
        .env("FRESHEN_HEALTH_DEF", "healthy")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("did not become healthy: web-1"),
        "output: {text}"
    );
    assert!(
        text.contains("rollback: docker pull nginx@sha256:old"),
        "output: {text}"
    );
    assert!(!text.contains("db-1"), "output: {text}");
    let calls = tools.step_calls();
    // The previous image is captured before `pull`, then again after `up -d`.
    assert!(
        calls
            .iter()
            .filter(|call| call.as_str() == project_ps("web"))
            .count()
            == 2,
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"docker inspect --format {{.State.Health.Status}} abc".to_owned()),
        "calls: {calls:?}"
    );
}

#[test]
fn a_container_that_becomes_healthy_after_waiting_is_not_reported() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .env("FRESHEN_CONTAINERS_WEB", "ghi\tweb-1\tweb")
        .env("FRESHEN_CONTAINERS_DB", "def\tdb-1\tdb")
        .env("FRESHEN_HEALTH_WAIT_SECS", "2")
        .env("FRESHEN_HEALTH_POLL_SECS", "1")
        .env("FRESHEN_HEALTH_ABC", "healthy")
        .env("FRESHEN_HEALTH_DEF", "healthy")
        .env("FRESHEN_HEALTH_GHI", "healthy")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("did not become healthy"), "output: {text}");
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
    expected.extend(docker_builder_prune().iter().map(|&s| s.to_string()));
    expected.push("docker system df".to_string());
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

#[test]
fn a_compose_project_with_the_skip_label_is_not_pulled_or_started() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let compose_json = r#"[
        {"Name":"web","ConfigFiles":"/srv/web/compose.yml,/srv/web/override.yml"},
        {"Name":"db","ConfigFiles":"/srv/db/compose.yml","Labels":{"freshen.com/skip":"true"}}
    ]"#;

    let output = run(
        &tools,
        &Scenario {
            compose_json,
            ..Scenario::default()
        },
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();

    assert!(calls.contains(&web("pull --quiet")), "calls: {calls:?}");
    assert!(calls.contains(&web("up -d")), "calls: {calls:?}");
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose -p db")),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("docker compose pull + up -d: web"),
        "output: {text}"
    );
    assert!(
        !text.contains("docker compose pull + up -d: db"),
        "output: {text}"
    );
}

#[test]
fn a_compose_project_in_the_config_exclude_list_is_not_pulled_or_started() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let config = tools.home().join("config.toml");
    fs::write(
        &config,
        r#"[exclude]
projects = ["db"]
"#,
    )
    .unwrap_or_else(|error| panic!("failed to write config: {error}"));

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "colima\n")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .args(["--only", ALL_STEPS, "--cleanup", "--config"])
        .arg(&config)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();

    assert!(calls.contains(&web("pull --quiet")), "calls: {calls:?}");
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose -p db")),
        "calls: {calls:?}"
    );
}

/// Creates `bin/<tool>` executables inside `prefix`, without putting anything
/// on `PATH`.
fn install_in_prefix(prefix: &Path, tools: &[&str]) {
    let bin = prefix.join("bin");
    fs::create_dir_all(&bin)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", bin.display()));
    for tool in tools {
        let path = bin.join(tool);
        fs::write(&path, "#!/bin/sh\n").unwrap_or_else(|error| panic!("{error}"));
        fs::set_permissions(&path, Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

/// The `--list-steps` line that describes `step`.
fn listed_step(text: &str, step: &str) -> String {
    text.lines()
        .find(|line| line.contains(step))
        .unwrap_or_else(|| panic!("step `{step}` is not listed: {text}"))
        .to_owned()
}

#[test]
fn colima_is_detected_when_installed_only_in_a_non_path_homebrew_prefix() {
    let tools = StubTools::new(&[]);
    let prefix = tools.home().join("homebrew");
    install_in_prefix(&prefix, &["brew", "colima", "limactl"]);

    let output = tools
        .command()
        .env("FRESHEN_BREW_PREFIXES", &prefix)
        .arg("--list-steps")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let text = stdout(&output);
    for step in ["colima-stop", "colima-start"] {
        let line = listed_step(&text, step);
        assert!(
            line.contains("installed") && !line.contains("skipped"),
            "{step}: {line}"
        );
    }
}

#[test]
fn limactl_alone_in_the_prefix_detects_the_colima_runtime() {
    let tools = StubTools::new(&[]);
    let prefix = tools.home().join("homebrew");
    install_in_prefix(&prefix, &["brew", "limactl"]);

    let output = tools
        .command()
        .env("FRESHEN_BREW_PREFIXES", &prefix)
        .arg("--list-steps")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let text = stdout(&output);
    assert!(
        listed_step(&text, "colima-stop").contains("installed"),
        "lima installed through Homebrew must be detected: {text}"
    );
}

#[test]
fn colima_is_not_detected_when_only_brew_is_in_the_prefix() {
    let tools = StubTools::new(&[]);
    let prefix = tools.home().join("homebrew");
    install_in_prefix(&prefix, &["brew"]);

    let output = tools
        .command()
        .env("FRESHEN_BREW_PREFIXES", &prefix)
        .arg("--list-steps")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let text = stdout(&output);
    let line = listed_step(&text, "colima-stop");
    assert!(
        line.contains("not installed") && line.contains("skipped: not installed"),
        "{line}"
    );
}

/// Writes a compose file into the stub home and returns its path plus the
/// `docker compose ls` JSON that references it.
fn compose_scenario(tools: &StubTools, name: &str, services: &str) -> (PathBuf, String) {
    let compose = tools.home().join(format!("{name}.yml"));
    fs::write(&compose, services)
        .unwrap_or_else(|error| panic!("failed to write {name}.yml: {error}"));
    let json = format!(
        r#"[{{"Name":"{name}","ConfigFiles":"{}"}}]"#,
        compose.display()
    );
    (compose, json)
}

#[test]
fn pinned_images_are_skipped_without_docker_bump() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let (compose, compose_json) = compose_scenario(
        &tools,
        "web",
        "services:\n  web:\n    image: nginx@sha256:old\n  app:\n    image: node\n",
    );

    let pull_all = format!(
        "docker compose -p web -f {} pull --quiet",
        compose.display()
    );
    let pull_app = format!("{pull_all} app");

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", &compose_json)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .env("FRESHEN_CONTAINERS_WEB", "abc\tweb-1\tweb")
        .env("FRESHEN_HEALTH_WAIT_SECS", "0")
        .env("FRESHEN_HEALTH_ABC", "healthy")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&pull_app),
        "the unpinned app image should be pulled: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == &pull_all),
        "the pinned web image should not be pulled: {calls:?}"
    );
    let text = stdout(&output);
    assert!(
        text.contains("skipping pinned images: web"),
        "output: {text}"
    );
}

#[test]
fn pinned_images_are_pulled_with_docker_bump() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let (compose, compose_json) = compose_scenario(
        &tools,
        "web",
        "services:\n  web:\n    image: nginx@sha256:old\n  app:\n    image: node\n",
    );

    let pull_all = format!(
        "docker compose -p web -f {} pull --quiet",
        compose.display()
    );

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", &compose_json)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .env("FRESHEN_CONTAINERS_WEB", "abc\tweb-1\tweb")
        .env("FRESHEN_HEALTH_WAIT_SECS", "0")
        .env("FRESHEN_HEALTH_ABC", "healthy")
        .args(["--only", ALL_STEPS, "--cleanup", "--docker-bump"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&pull_all),
        "all images should be pulled with --docker-bump: {calls:?}"
    );
    let text = stdout(&output);
    assert!(!text.contains("skipping pinned images"), "output: {text}");
}

#[test]
fn a_fixed_tag_is_treated_as_pinned() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);
    install_docker(&tools);

    let (compose, compose_json) = compose_scenario(
        &tools,
        "web",
        "services:\n  web:\n    image: node:20\n  app:\n    image: node\n",
    );

    let pull_all = format!(
        "docker compose -p web -f {} pull --quiet",
        compose.display()
    );
    let pull_app = format!("{pull_all} app");

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", &compose_json)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .env("FRESHEN_CONTAINERS_WEB", "abc\tweb-1\tweb")
        .env("FRESHEN_HEALTH_WAIT_SECS", "0")
        .env("FRESHEN_HEALTH_ABC", "healthy")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&pull_app),
        "the untagged app image should be pulled: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == &pull_all),
        "the pinned node:20 image should not be pulled: {calls:?}"
    );
}

#[test]
fn docker_system_df_reports_build_cache_and_volume_usage() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools);
    install_colima(&tools);

    tools.set_script(
        "docker",
        r#"case "$*" in
  "compose version")
    exit 0
    ;;
  "compose ls --format json")
    printf '[]'
    ;;
  "ps --format "*)
    printf ''
    ;;
  "image prune -f")
    exit 0
    ;;
  "system df")
    printf '%s' "$FRESHEN_DOCKER_SYSTEM_DF"
    ;;
esac"#,
    );

    const DOCKER_SYSTEM_DF: &str = "\
TYPE            TOTAL     ACTIVE    SIZE      RECLAIMABLE
Images          2         2         1.2GB     0B
Containers      1         1         50MB      0B
Local Volumes   3         2         500MB     100MB
Build Cache     10        2         2.5GB     1.5GB
";

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "")
        .env("FRESHEN_OUTDATED_CASKS", "")
        .env("FRESHEN_BREW_UPGRADE_STATUS", "0")
        .env("FRESHEN_COLIMA_STATUS", "0")
        .env("FRESHEN_COLIMA_START_STATUS", "0")
        .env("FRESHEN_COMPOSE_JSON", "[]")
        .env("FRESHEN_PS_OUTPUT", "")
        .env("FRESHEN_DOCKER_SYSTEM_DF", DOCKER_SYSTEM_DF)
        .env("FRESHEN_HEALTH_WAIT_SECS", "0")
        .args(["--only", ALL_STEPS, "--cleanup", "--json"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"docker system df".to_owned()),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    let report: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("invalid JSON report: {error}: {text}"));

    let docker_step = report["steps"]
        .as_array()
        .and_then(|steps| steps.iter().find(|step| step["id"] == "docker"))
        .unwrap_or_else(|| panic!("docker step not found in report: {text}"));

    let volumes = &docker_step["docker_df"]["volumes"];
    assert_eq!(volumes["total"], 3);
    assert_eq!(volumes["active"], 2);
    assert_eq!(volumes["size"], "500MB");
    assert_eq!(volumes["reclaimable"], "100MB");

    let build_cache = &docker_step["docker_df"]["build_cache"];
    assert_eq!(build_cache["total"], 10);
    assert_eq!(build_cache["active"], 2);
    assert_eq!(build_cache["size"], "2.5GB");
    assert_eq!(build_cache["reclaimable"], "1.5GB");
}

#[test]
fn docker_steps_never_touch_a_remote_engine() {
    let tools = StubTools::new(&["docker", "colima"]);

    let output = tools
        .command()
        .env("DOCKER_HOST", "tcp://remote.example.com:2375")
        .args(["--only", ALL_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // A remote engine is never touched: no docker command is issued, not even
    // the read-only compose version probe, and no container is stopped.
    assert!(
        tools.step_calls().is_empty(),
        "no calls against a remote engine: {:?}",
        tools.step_calls()
    );
}
