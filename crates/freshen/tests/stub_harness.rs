//! End-to-end harness: the built binary against stub package managers.
//!
//! Every test runs `freshen` with `PATH` pointing at a temporary directory of
//! stub executables, so no test can invoke the real `brew`, `uv`, `mise`,
//! `docker` or `colima`.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run_stub(tools: &StubTools, tool: &str, args: &[&str]) -> Output {
    tools
        .stub_command(tool)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run stub {tool}: {error}"))
}

#[test]
fn a_stub_returns_canned_stdout_stderr_and_status_per_invocation() {
    let tools = StubTools::new(&["probe"]);
    tools.set_responses(
        "probe",
        &[
            StubOutput::stdout("first\n").with_stderr("first warning\n"),
            StubOutput::stdout("second\n")
                .with_stderr("second warning\n")
                .with_status(7),
        ],
    );

    let first = run_stub(&tools, "probe", &["status"]);
    assert_eq!(first.status.code(), Some(0), "stderr: {}", stderr(&first));
    assert_eq!(stdout(&first), "first\n");
    assert_eq!(stderr(&first), "first warning\n");

    let second = run_stub(&tools, "probe", &["status"]);
    assert_eq!(second.status.code(), Some(7), "stderr: {}", stderr(&second));
    assert_eq!(stdout(&second), "second\n");
    assert_eq!(stderr(&second), "second warning\n");

    let unconfigured = run_stub(&tools, "probe", &["status"]);
    assert_eq!(
        unconfigured.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&unconfigured)
    );
    assert!(stdout(&unconfigured).is_empty());
    assert!(stderr(&unconfigured).is_empty());

    assert_eq!(
        tools.step_calls(),
        ["probe status", "probe status", "probe status"]
    );
}

#[cfg(unix)]
#[test]
fn a_ported_step_replays_a_recorded_command_transcript() {
    const CASK_INFO: &str = include_str!("../../freshen-lib/tests/fixtures/brew-info-casks.json");

    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &[
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("firefox\n"),
            StubOutput::stdout(CASK_INFO),
            StubOutput::stderr("cask upgrade failed\n").with_status(1),
            StubOutput::new(),
            StubOutput::new(),
        ],
    );

    let output = tools
        .command()
        .env_remove("ZELLIJ")
        .args([
            "--only",
            "brew,brew-formulae,brew-casks,brew-autoremove,brew-cleanup",
            "--yes",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew upgrade --formula",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox",
            "brew upgrade --cask --greedy",
            "brew autoremove",
            "brew cleanup --prune=all",
        ]
    );

    let text = stdout(&output);
    assert!(
        text.contains("Casks to upgrade: firefox"),
        "canned stdout was not replayed: {text}"
    );
    assert!(
        text.contains("1 failed"),
        "canned exit status was not reported: {text}"
    );
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
        tools.step_calls(),
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
fn list_steps_reports_detection_without_calling_any_stub() {
    let tools = StubTools::new(&["brew", "uv"]);

    let output = tools.run(
        "brew update;uv tool upgrade --all",
        &["--list-steps", "--skip", "probe-1"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());

    let text = stdout(&output);
    assert!(text.contains("Steps (2):"), "listing: {text}");
    assert!(text.contains("probe-0"), "listing: {text}");
    assert!(text.contains("brew"), "listing: {text}");
    assert!(text.contains("installed"), "listing: {text}");
    assert!(text.contains("excluded by --skip"), "listing: {text}");
}

#[test]
fn dry_run_detects_steps_without_calling_any_stub() {
    let tools = StubTools::new(&["brew", "uv", "mise", "docker", "colima"]);

    let output = tools.run("brew update;uv tool upgrade --all", &["--dry-run"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(stdout(&output).contains("0 ok, 2 planned, 0 skipped, 0 failed"));
}

#[test]
fn a_tool_missing_from_the_stub_path_is_never_replaced_by_the_real_one() {
    let tools = StubTools::new(&["brew"]);

    let output = tools.run("freshen-definitely-not-a-real-tool", &[]);

    assert!(!output.status.success());
    assert!(tools.step_calls().is_empty());
    assert!(stdout(&output).contains("1 failed"));
}

#[test]
fn a_failing_step_is_recorded_and_the_run_continues_to_exit_1() {
    let tools = StubTools::new(&["brew"]);

    let output = tools.run(
        "freshen-definitely-not-a-real-tool --upgrade;brew update",
        &[],
    );

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), ["brew update"]);

    let text = stdout(&output);
    assert!(
        text.contains("1 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {text}"
    );
    let failed_line = text
        .lines()
        .find(|line| line.contains("freshen-definitely-not-a-real-tool"))
        .unwrap_or_else(|| panic!("summary does not list the failed step: {text}"));
    assert!(failed_line.contains("failed"), "summary: {text}");
}
