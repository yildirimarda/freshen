//! End-to-end checks for the other-package-manager gate before an update.
//!
//! The host process table is scripted through the test harness, so these tests
//! never look for a real `brew`, `apt`, `dpkg` or `winget` process.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::StubTools;

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
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
