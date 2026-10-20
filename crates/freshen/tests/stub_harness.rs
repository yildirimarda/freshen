//! End-to-end harness: the built binary against stub package managers.
//!
//! Every test runs `freshen` with `PATH` pointing at a temporary directory of
//! stub executables, so no test can invoke the real `brew`, `uv`, `mise`,
//! `docker` or `colima`.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::StubTools;

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn runs_stub_tools_in_step_order() {
    let tools = StubTools::new(&["brew", "uv", "mise", "docker", "colima"]);

    let output = tools.run(
        "brew update;\
         brew upgrade --formula;\
         brew upgrade --cask --greedy;\
         uv tool upgrade --all;\
         mise upgrade;\
         docker image prune -f;\
         colima start",
        &["--yes"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        [
            "brew update",
            "brew upgrade --formula",
            "brew upgrade --cask --greedy",
            "uv tool upgrade --all",
            "mise upgrade",
            "docker image prune -f",
            "colima start",
        ]
    );
    assert!(stdout(&output).contains("7 ok, 0 planned, 0 skipped, 0 failed"));
}

#[test]
fn dry_run_detects_steps_without_calling_any_stub() {
    let tools = StubTools::new(&["brew", "uv", "mise", "docker", "colima"]);

    let output = tools.run("brew update;uv tool upgrade --all", &["--dry-run"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.calls().is_empty());
    assert!(stdout(&output).contains("0 ok, 2 planned, 0 skipped, 0 failed"));
}

#[test]
fn a_tool_missing_from_the_stub_path_is_never_replaced_by_the_real_one() {
    let tools = StubTools::new(&["brew"]);

    let output = tools.run("freshen-definitely-not-a-real-tool", &[]);

    assert!(!output.status.success());
    assert!(tools.calls().is_empty());
    assert!(stdout(&output).contains("1 failed"));
}
