//! Stub-based tests for Docker Desktop on Windows (WSL2 backend).
//!
//! The WSL2-backed runtime is detected from `docker.exe`, `DOCKER_HOST`,
//! `docker context show` and the well-known named pipe, and its container
//! steps are registered for Windows. No real Docker Desktop is touched.

#![cfg(all(feature = "test-harness", target_os = "windows"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools, args: &[&str]) -> Output {
    tools
        .command()
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn docker_desktop_steps_are_registered_for_windows() {
    let tools = StubTools::new(&["docker"]);
    // `detect_compose_command` runs `docker compose version` at registration.
    tools.set_responses("docker", &[StubOutput::new()]);

    let output = run(
        &tools,
        &["--container-runtime", "docker-desktop", "--list-steps"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("Container runtime: Docker Desktop (--container-runtime)"),
        "listing: {text}"
    );
    for step in ["colima-stop", "colima-start", "docker"] {
        assert!(text.contains(step), "step `{step}` is not listed: {text}");
    }
}

#[test]
fn docker_host_identifies_docker_desktop_on_windows() {
    let tools = StubTools::new(&["docker"]);
    // `detect_compose_command` runs `docker compose version` at registration.
    tools.set_responses("docker", &[StubOutput::new()]);

    let output = tools
        .command()
        .env("DOCKER_HOST", "npipe:////./pipe/docker_engine")
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("Container runtime: Docker Desktop (DOCKER_HOST)"),
        "listing: {}",
        stdout(&output)
    );
}

#[test]
fn docker_context_identifies_docker_desktop_on_windows() {
    let tools = StubTools::new(&["docker"]);
    tools.set_responses(
        "docker",
        &[
            StubOutput::new().with_stdout("desktop-windows\n"), // docker context show
            StubOutput::new(),                                  // docker compose version
        ],
    );

    let output = tools
        .command()
        .env_remove("DOCKER_HOST")
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("Container runtime: Docker Desktop (docker context)"),
        "listing: {}",
        stdout(&output)
    );
}
