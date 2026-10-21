//! Tests for warnings about long-running interactive sessions.
//!
//! The binary runs against stub `brew`, `docker`, `colima` and `ps`
//! executables, so the process table is scripted and no real session or
//! container runtime is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use freshen_lib::warnings::{
    DOCKER_BUILD_WARNING, SSH_WARNING, TMUX_WARNING, ZELLIJ_SERVER_WARNING,
};
use support::StubTools;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn install_outdated_formulae(tools: &StubTools, names: &str) {
    tools.set_script(
        "brew",
        &format!(
            r#"case "$*" in
  "outdated --formula --quiet")
    printf '%s\n' {names:?}
    ;;
esac"#
        ),
    );
}

fn install_process_table(tools: &StubTools, processes: &str) {
    tools.set_script(
        "ps",
        &format!(
            r#"case "$*" in
  "-axo pid=,ppid=,command=")
    printf '%s\n' {processes}
    ;;
esac"#
        ),
    );
}

fn run_formulae(tools: &StubTools) -> Output {
    tools
        .command()
        .env_remove("ZELLIJ")
        .args(["--only", "brew,brew-formulae"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn warns_when_tmux_is_upgraded_while_a_server_is_running() {
    let tools = StubTools::new(&["brew", "ps"]);
    install_outdated_formulae(&tools, "tmux");
    install_process_table(
        &tools,
        "'  500     1 /opt/homebrew/bin/tmux new-session -d -s work'",
    );

    let output = run_formulae(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains(TMUX_WARNING),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn warns_about_a_tmux_session_even_when_the_process_table_is_unavailable() {
    let tools = StubTools::new(&["brew"]);
    install_outdated_formulae(&tools, "tmux");

    let output = tools
        .command()
        .env_remove("ZELLIJ")
        .env("TMUX", "/tmp/tmux-1000/default,123,0")
        .args(["--only", "brew,brew-formulae"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains(TMUX_WARNING),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn warns_when_openssh_is_upgraded_while_an_sshd_session_is_active() {
    let tools = StubTools::new(&["brew", "ps"]);
    install_outdated_formulae(&tools, "openssh");
    install_process_table(&tools, "'  600     1 sshd: user@pts/0'");

    let output = run_formulae(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains(SSH_WARNING),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn warns_about_an_ssh_session_even_when_the_process_table_is_unavailable() {
    let tools = StubTools::new(&["brew"]);
    install_outdated_formulae(&tools, "openssh");

    let output = tools
        .command()
        .env_remove("ZELLIJ")
        .env("SSH_CONNECTION", "192.0.2.1 50000 192.0.2.2 22")
        .args(["--only", "brew,brew-formulae"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains(SSH_WARNING),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn warns_about_a_zellij_server_outside_the_current_session() {
    let tools = StubTools::new(&["brew", "ps"]);
    install_outdated_formulae(&tools, "zellij");
    install_process_table(&tools, "'  700     1 zellij --server /tmp/zellij-1000'");

    let output = run_formulae(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains(ZELLIJ_SERVER_WARNING),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn warns_before_a_container_step_while_a_docker_build_is_running() {
    let tools = StubTools::new(&["docker", "colima", "ps"]);
    install_process_table(&tools, "'  800     1 docker build -t app .'");

    let output = tools
        .command()
        .env_remove("ZELLIJ")
        .args(["--only", "docker"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains(DOCKER_BUILD_WARNING), "output: {text}");
    assert!(text.contains("pid 800"), "output: {text}");
}

#[test]
fn other_docker_commands_do_not_trigger_the_build_warning() {
    let tools = StubTools::new(&["docker", "colima", "ps"]);
    install_process_table(&tools, "'  801     1 docker ps'");

    let output = tools
        .command()
        .env_remove("ZELLIJ")
        .args(["--only", "docker"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains(DOCKER_BUILD_WARNING),
        "output: {}",
        stdout(&output)
    );
}
