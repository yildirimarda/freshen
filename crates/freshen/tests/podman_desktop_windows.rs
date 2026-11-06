//! Stub-based tests for Podman Desktop on Windows (WSL2 backend).
//!
//! Podman Desktop runs a Podman machine inside WSL2. `wsl --update` shuts down
//! every WSL distribution, so Podman Desktop must be stopped before
//! `wsl --update` runs and started again afterwards. No real Podman Desktop or
//! WSL installation is touched.

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
fn podman_desktop_steps_are_registered_for_windows() {
    let tools = StubTools::new(&["podman", "docker", "wsl"]);
    // `detect_compose_command` runs `docker compose version` at registration.
    tools.set_responses("docker", &[StubOutput::new()]);

    let output = run(
        &tools,
        &["--container-runtime", "podman-desktop", "--list-steps"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("Container runtime: Podman Desktop (--container-runtime)"),
        "listing: {text}"
    );
    for step in ["colima-stop", "colima-start", "docker", "wsl"] {
        assert!(text.contains(step), "step `{step}` is not listed: {text}");
    }
}

#[test]
fn podman_command_identifies_podman_desktop_on_windows() {
    let tools = StubTools::new(&["podman", "docker", "wsl"]);
    tools.set_responses("docker", &[StubOutput::new()]);

    let output = run(
        &tools,
        &["--container-runtime", "podman-desktop", "--list-steps"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("Container runtime: Podman Desktop (--container-runtime)"),
        "listing: {}",
        stdout(&output)
    );
}

#[test]
fn docker_host_identifies_podman_desktop_on_windows() {
    let tools = StubTools::new(&["podman", "docker", "wsl"]);
    tools.set_responses("docker", &[StubOutput::new()]);

    let output = tools
        .command()
        .env("DOCKER_HOST", "npipe:////./pipe/podman_engine")
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("Container runtime: Podman Desktop (DOCKER_HOST)"),
        "listing: {}",
        stdout(&output)
    );
}

#[test]
fn docker_context_identifies_podman_desktop_on_windows() {
    let tools = StubTools::new(&["podman", "docker", "wsl"]);
    tools.set_responses(
        "docker",
        &[
            StubOutput::new().with_stdout("podman-desktop\n"), // docker context show
            StubOutput::new(),                                 // docker compose version
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
        stdout(&output).contains("Container runtime: Podman Desktop (docker context)"),
        "listing: {}",
        stdout(&output)
    );
}
