//! Detection and override of the active container runtime.
//!
//! The tests inject the environment and the socket files instead of touching a
//! real Docker installation. `docker context show` is served by a stub.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::fs;
use std::process::Output;

use support::StubTools;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn install_context(tools: &StubTools, context: &str) {
    tools.set_script(
        "docker",
        &format!(
            r#"case "$*" in
  "context show")
    printf '%s\n' '{context}'
    ;;
esac"#
        ),
    );
}

#[test]
fn list_steps_shows_the_runtime_from_docker_host() {
    let tools = StubTools::new(&[]);

    let output = tools
        .command()
        .env("DOCKER_HOST", "unix:///Users/me/.orbstack/run/docker.sock")
        .arg("--list-steps")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("Container runtime: OrbStack (DOCKER_HOST)"),
        "listing: {}",
        stdout(&output)
    );
}

#[test]
fn list_steps_shows_the_runtime_from_docker_context() {
    let tools = StubTools::new(&["docker"]);
    install_context(&tools, "desktop-linux");

    let output = tools
        .command()
        .env_remove("DOCKER_HOST")
        .arg("--list-steps")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("Container runtime: Docker Desktop (docker context)"),
        "listing: {}",
        stdout(&output)
    );
}

#[test]
fn list_steps_shows_the_runtime_from_a_well_known_socket() {
    let tools = StubTools::new(&[]);
    let socket = tools.home().join(".colima/default/docker.sock");
    let parent = socket
        .parent()
        .unwrap_or_else(|| panic!("socket path has no parent: {}", socket.display()));
    fs::create_dir_all(parent)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", parent.display()));
    fs::write(&socket, "")
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", socket.display()));

    let output = tools
        .command()
        .env_remove("DOCKER_HOST")
        .arg("--list-steps")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("Container runtime: Colima ("),
        "listing: {text}"
    );
    assert!(
        text.contains(&socket.display().to_string()),
        "listing: {text}"
    );
}

#[test]
fn the_context_wins_over_a_socket_path() {
    let tools = StubTools::new(&["docker"]);
    install_context(&tools, "rancher-desktop");
    let socket = tools.home().join(".colima/default/docker.sock");
    let parent = socket
        .parent()
        .unwrap_or_else(|| panic!("socket path has no parent: {}", socket.display()));
    fs::create_dir_all(parent)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", parent.display()));
    fs::write(&socket, "")
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", socket.display()));

    let output = tools
        .command()
        .env_remove("DOCKER_HOST")
        .arg("--list-steps")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("Container runtime: Rancher Desktop (docker context)"),
        "listing: {}",
        stdout(&output)
    );
}

#[test]
fn the_container_runtime_flag_overrides_detection() {
    let tools = StubTools::new(&[]);

    let output = tools
        .command()
        .env("DOCKER_HOST", "unix:///Users/me/.orbstack/run/docker.sock")
        .args(["--container-runtime", "podman", "--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("Container runtime: Podman (--container-runtime)"),
        "listing: {text}"
    );
    assert!(!text.contains("OrbStack"), "listing: {text}");
}

#[test]
fn an_unknown_container_runtime_is_rejected() {
    let tools = StubTools::new(&[]);

    let output = tools
        .command()
        .args(["--container-runtime", "nope", "--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("unknown container runtime `nope`"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn the_run_summary_shows_the_active_runtime() {
    let tools = StubTools::new(&["uv"]);

    let output = tools
        .command()
        .env("DOCKER_HOST", "unix:///var/run/docker.sock")
        .args(["--dry-run", "--only", "uv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("Container runtime: Docker Engine (DOCKER_HOST)"),
        "summary: {text}"
    );
    assert!(text.contains("Summary:"), "summary: {text}");
}

#[test]
fn the_run_summary_shows_an_overridden_runtime() {
    let tools = StubTools::new(&["uv"]);

    let output = tools
        .command()
        .args([
            "--container-runtime",
            "docker-desktop",
            "--dry-run",
            "--only",
            "uv",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("Container runtime: Docker Desktop (--container-runtime)"),
        "summary: {}",
        stdout(&output)
    );
}
