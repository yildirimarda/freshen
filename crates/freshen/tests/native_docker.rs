//! Stub-based tests for a native Docker Engine on Linux.
//!
//! The runtime is forced with `--container-runtime docker`, stopped before the
//! `docker` formula is upgraded and restarted afterwards. No real Docker Engine
//! or systemd unit is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubTools, ALL_STEPS};

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
  "upgrade --formula")
    exit 0
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
  "compose version"|"compose ls --format json"|"image prune -f")
    if [ "$*" = "compose ls --format json" ]; then printf '[]'; fi
    exit 0
    ;;
  "info --format {{.LiveRestoreEnabled}}")
    printf '%s\n' "$FRESHEN_LIVE_RESTORE"
    ;;
  "ps --format "*)
    exit 0
    ;;
esac"#,
    );
}

fn install_systemctl(tools: &StubTools) {
    tools.set_script("systemctl", "exit 0");
}

fn install_apt_docker_upgrade(tools: &StubTools) {
    tools.set_script(
        "apt-get",
        r#"case "$*" in
  "-s upgrade")
    printf 'Inst docker.io [5:20.10.21-0ubuntu1] (5:27.0.3-1~ubuntu22.04.1)\n'
    ;;
  "update")
    exit 0
    ;;
  "upgrade -y")
    exit 0
    ;;
  "autoremove -y")
    exit 0
    ;;
  "autoclean")
    exit 0
    ;;
esac"#,
    );
}

fn install_apt_no_docker_upgrade(tools: &StubTools) {
    tools.set_script(
        "apt-get",
        r#"case "$*" in
  "-s upgrade")
    printf 'Inst curl [7.81.0] (7.88.0 stable [amd64])\n'
    ;;
  "update")
    exit 0
    ;;
  "upgrade -y")
    exit 0
    ;;
  "autoremove -y")
    exit 0
    ;;
  "autoclean")
    exit 0
    ;;
esac"#,
    );
}

fn install_dnf_docker_upgrade(tools: &StubTools) {
    tools.set_script(
        "dnf",
        r#"case "$*" in
  "list --upgrades")
    printf 'docker-ce.x86_64  3:20.10.21-3.el8  docker-ce-stable\n'
    ;;
  "upgrade -y")
    exit 0
    ;;
  "autoremove -y")
    exit 0
    ;;
esac"#,
    );
}

fn install_dnf_no_docker_upgrade(tools: &StubTools) {
    tools.set_script(
        "dnf",
        r#"case "$*" in
  "list --upgrades")
    printf 'curl.x86_64  7.81.0-1  baseos\n'
    ;;
  "upgrade -y")
    exit 0
    ;;
  "autoremove -y")
    exit 0
    ;;
esac"#,
    );
}

fn run_apt(tools: &StubTools, live_restore: &str) -> Output {
    tools
        .command()
        .env("FRESHEN_LIVE_RESTORE", live_restore)
        .args([
            "--container-runtime",
            "docker",
            "--only",
            "apt",
            "--cleanup",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn run_dnf(tools: &StubTools, live_restore: &str) -> Output {
    tools
        .command()
        .env("FRESHEN_LIVE_RESTORE", live_restore)
        .args([
            "--container-runtime",
            "docker",
            "--only",
            "dnf",
            "--cleanup",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn run(tools: &StubTools, live_restore: &str) -> Output {
    tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "docker\n")
        .env("FRESHEN_LIVE_RESTORE", live_restore)
        .args([
            "--container-runtime",
            "docker",
            "--only",
            ALL_STEPS,
            "--cleanup",
        ])
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
fn stops_and_restarts_the_docker_unit_around_the_formula_upgrade() {
    let tools = StubTools::new(&["brew", "docker", "systemctl"]);
    install_brew(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    let output = run(&tools, "false");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let stop = index_of(&calls, "systemctl stop docker");
    let upgrade = index_of(&calls, "brew upgrade --formula");
    let restart = index_of(&calls, "systemctl restart docker");

    assert!(stop < upgrade, "calls: {calls:?}");
    assert!(upgrade < restart, "calls: {calls:?}");

    let text = stdout(&output);
    assert!(
        text.contains(
            "warning: Docker live-restore is off; restarting Docker will stop running containers."
        ),
        "output: {text}"
    );
}

#[test]
fn does_not_warn_when_live_restore_keeps_the_containers_running() {
    let tools = StubTools::new(&["brew", "docker", "systemctl"]);
    install_brew(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    let output = run(&tools, "true");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains("live-restore is off"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn apt_upgrading_docker_io_stops_and_restarts_docker_engine() {
    let tools = StubTools::new(&["apt-get", "docker", "systemctl"]);
    install_apt_docker_upgrade(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    let output = run_apt(&tools, "false");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let stop = index_of(&calls, "systemctl stop docker");
    let upgrade = index_of(&calls, "apt-get upgrade -y");
    let restart = index_of(&calls, "systemctl restart docker");

    assert!(stop < upgrade, "calls: {calls:?}");
    assert!(upgrade < restart, "calls: {calls:?}");

    let text = stdout(&output);
    assert!(
        text.contains(
            "warning: Docker live-restore is off; restarting Docker will stop running containers."
        ),
        "output: {text}"
    );
}

#[test]
fn apt_upgrading_rootless_docker_uses_user_systemd() {
    let tools = StubTools::new(&["apt-get", "docker", "systemctl"]);
    install_apt_docker_upgrade(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    // `$XDG_RUNTIME_DIR/docker.sock` marks the engine as rootless, so it is
    // controlled through `systemctl --user` instead of the system daemon.
    std::fs::File::create(tools.stub_dir().join("docker.sock"))
        .unwrap_or_else(|error| panic!("failed to create rootless socket: {error}"));

    let output = tools
        .command()
        .env("FRESHEN_LIVE_RESTORE", "false")
        .env("XDG_RUNTIME_DIR", tools.stub_dir())
        .args([
            "--container-runtime",
            "docker",
            "--only",
            "apt",
            "--cleanup",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let stop = index_of(&calls, "systemctl --user stop docker");
    let upgrade = index_of(&calls, "apt-get upgrade -y");
    let restart = index_of(&calls, "systemctl --user restart docker");

    assert!(stop < upgrade, "calls: {calls:?}");
    assert!(upgrade < restart, "calls: {calls:?}");

    let text = stdout(&output);
    assert!(
        text.contains(
            "warning: Docker live-restore is off; restarting Docker will stop running containers."
        ),
        "output: {text}"
    );
}

#[test]
fn apt_not_upgrading_docker_does_not_stop_docker_engine() {
    let tools = StubTools::new(&["apt-get", "docker", "systemctl"]);
    install_apt_no_docker_upgrade(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    let output = run_apt(&tools, "false");

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
fn dnf_upgrading_docker_ce_stops_and_restarts_docker_engine() {
    let tools = StubTools::new(&["dnf", "docker", "systemctl"]);
    install_dnf_docker_upgrade(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    let output = run_dnf(&tools, "false");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let stop = index_of(&calls, "systemctl stop docker");
    let upgrade = index_of(&calls, "dnf upgrade -y");
    let restart = index_of(&calls, "systemctl restart docker");

    assert!(stop < upgrade, "calls: {calls:?}");
    assert!(upgrade < restart, "calls: {calls:?}");

    let text = stdout(&output);
    assert!(
        text.contains(
            "warning: Docker live-restore is off; restarting Docker will stop running containers."
        ),
        "output: {text}"
    );
}

#[test]
fn dnf_not_upgrading_docker_does_not_stop_docker_engine() {
    let tools = StubTools::new(&["dnf", "docker", "systemctl"]);
    install_dnf_no_docker_upgrade(&tools);
    install_docker(&tools);
    install_systemctl(&tools);

    let output = run_dnf(&tools, "false");

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
