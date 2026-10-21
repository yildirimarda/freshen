//! Parity tests for the Colima/lima and Docker orchestration.
//!
//! Every test runs the built binary against stub `brew`, `colima` and `docker`
//! executables and asserts the exact command transcript of the corresponding
//! block in `update-all.sh`. No real container runtime is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::StubTools;

const ALL_STEPS: &str = "brew,brew-formulae,brew-casks,brew-autoremove,brew-cleanup,brew-reopen,colima-stop,colima-start,docker";

const PS_CALL: &str =
    "docker ps --format {{.ID}}\t{{.Names}}\t{{.Label \"com.docker.compose.project\"}}";

const COMPOSE_JSON: &str = r#"[
    {"Name":"web","ConfigFiles":"/srv/web/compose.yml,/srv/web/override.yml"},
    {"Name":"db","ConfigFiles":"/srv/db/compose.yml"}
]"#;

const PS_OUTPUT: &str = "abc\tweb-1\tweb\ndef\tlonely\t\nghi\tlonely2\t\n";

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn compose(project: &str, files: &[&str], action: &str) -> String {
    let mut call = format!("docker compose -p {project}");
    for file in files {
        call.push_str(" -f ");
        call.push_str(file);
    }
    call.push(' ');
    call.push_str(action);
    call
}

fn web(action: &str) -> String {
    compose(
        "web",
        &["/srv/web/compose.yml", "/srv/web/override.yml"],
        action,
    )
}

fn db(action: &str) -> String {
    compose("db", &["/srv/db/compose.yml"], action)
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
    assert_eq!(
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula --quiet",
            "colima status",
            "docker compose ls --format json",
            PS_CALL,
            &web("down"),
            &db("down"),
            "docker stop def",
            "docker stop ghi",
            "colima stop",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew list --pinned --formula",
            "brew upgrade --formula",
            "colima start",
            "docker start def",
            "docker start ghi",
            &web("up -d"),
            &db("up -d"),
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew autoremove --dry-run",
            "brew autoremove",
            "brew cleanup --dry-run --prune=all",
            "brew cleanup --prune=all",
            "colima status",
            "docker compose ls --format json",
            &web("pull --quiet"),
            &web("up -d"),
            &db("pull --quiet"),
            &db("up -d"),
            "docker image prune -f",
        ]
    );

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
    assert_eq!(
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula --quiet",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew autoremove --dry-run",
            "brew autoremove",
            "brew cleanup --dry-run --prune=all",
            "brew cleanup --prune=all",
            "colima status",
            "docker compose ls --format json",
            "docker image prune -f",
        ]
    );
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
    assert_eq!(
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula --quiet",
            "colima status",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew list --pinned --formula",
            "brew upgrade --formula",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew autoremove --dry-run",
            "brew autoremove",
            "brew cleanup --dry-run --prune=all",
            "brew cleanup --prune=all",
            "colima status",
        ]
    );
    assert!(
        stdout(&output).contains("warning: colima is not running, skipping Docker image updates."),
        "output: {}",
        stdout(&output)
    );
    assert!(
        !tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("docker ")),
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
    assert_eq!(
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula --quiet",
            "colima status",
            "colima stop",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew list --pinned --formula",
            "brew upgrade --formula",
            "colima start",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew autoremove --dry-run",
            "brew autoremove",
            "brew cleanup --dry-run --prune=all",
            "brew cleanup --prune=all",
        ]
    );
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
