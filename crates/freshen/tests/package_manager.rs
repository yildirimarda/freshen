//! End-to-end checks for the other-package-manager gate before an update.
//!
//! The host process table is scripted through the test harness, so these tests
//! never look for a real `brew`, `apt`, `dpkg` or `winget` process.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

use support::StubTools;

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[cfg(unix)]
fn send_signal(child: &Child, signal: &str) {
    let status = Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .unwrap_or_else(|error| panic!("failed to run kill: {error}"));
    assert!(status.success(), "kill {signal} failed with {status}");
}

#[cfg(unix)]
fn wait_for_run_lock(tools: &StubTools) {
    let pid_file = tools.home().join("lock").join("freshen.lock.pid");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pid_file.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the run lock at {}",
            pid_file.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn run(tools: &StubTools, managers: &str, args: &[&str]) -> Output {
    tools
        .command()
        .env_remove("FRESHEN_SKIP_PREFLIGHT")
        .env("FRESHEN_TEST_STEPS", "probe run")
        .env("FRESHEN_TEST_PREFLIGHT", "network=up")
        .env("FRESHEN_TEST_PACKAGE_MANAGERS", managers)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn another_package_manager_blocks_the_run_with_exit_code_three() {
    let tools = StubTools::new(&["probe"]);

    let output = run(&tools, "brew:4242", &["--package-manager-wait", "0"]);

    assert_eq!(output.status.code(), Some(3), "stderr: {}", stderr(&output));
    let text = stderr(&output);
    assert!(
        text.contains("another package manager is running"),
        "{text}"
    );
    assert!(text.contains("brew (PID 4242)"), "{text}");
    assert!(tools.calls().is_empty(), "no step should run");
}

#[test]
fn every_running_package_manager_is_named() {
    let tools = StubTools::new(&["probe"]);

    let output = run(
        &tools,
        "apt:11;dpkg:22;winget:33",
        &["--package-manager-wait", "0"],
    );

    assert_eq!(output.status.code(), Some(3), "stderr: {}", stderr(&output));
    let text = stderr(&output);
    for process in ["apt (PID 11)", "dpkg (PID 22)", "winget (PID 33)"] {
        assert!(text.contains(process), "missing {process}: {text}");
    }
}

#[test]
fn skipping_the_check_lets_the_run_proceed() {
    let tools = StubTools::new(&["probe"]);

    let output = run(
        &tools,
        "brew:4242",
        &[
            "--skip-package-manager-check",
            "--package-manager-wait",
            "0",
        ],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.calls(), ["probe run"]);
}

#[test]
fn an_idle_host_lets_the_run_proceed() {
    let tools = StubTools::new(&["probe"]);

    let output = run(&tools, "", &[]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.calls(), ["probe run"]);
}

#[test]
fn dry_run_and_check_do_not_look_for_package_managers() {
    for args in [["--dry-run"], ["--check"]] {
        let tools = StubTools::new(&["probe"]);
        let output = run(&tools, "brew:4242", &args);

        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        assert!(!stderr(&output).contains("another package manager"));
    }
}

#[cfg(unix)]
#[test]
fn sigint_during_the_package_manager_wait_returns_promptly() {
    let tools = StubTools::new(&["probe"]);

    let child = tools
        .command()
        .env_remove("FRESHEN_SKIP_PREFLIGHT")
        .env("FRESHEN_TEST_STEPS", "probe run")
        .env("FRESHEN_TEST_PREFLIGHT", "network=up")
        .env("FRESHEN_TEST_PACKAGE_MANAGERS", "brew:4242")
        .args(["--package-manager-wait", "30"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    wait_for_run_lock(&tools);
    thread::sleep(Duration::from_millis(100));

    let interrupted_at = Instant::now();
    send_signal(&child, "-INT");
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("failed to wait for freshen: {error}"));
    let elapsed = interrupted_at.elapsed();

    assert_eq!(
        output.status.code(),
        Some(130),
        "stderr: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("interrupted"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(tools.calls().is_empty(), "a step ran after the interrupt");
    assert!(
        elapsed < Duration::from_secs(5),
        "SIGINT waited for the package-manager wait: {elapsed:?}"
    );
}
