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
