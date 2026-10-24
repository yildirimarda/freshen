//! End-to-end harness: the built binary against stub package managers.
//!
//! Every test runs `freshen` with `PATH` pointing at a temporary directory of
//! stub executables, so no test can invoke the real `brew`, `uv`, `mise`,
//! `docker` or `colima`.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

#[cfg(not(windows))]
use support::brew_transcript;
#[cfg(unix)]
use support::slow_transcript;
use support::{
    brew_update_transcript, output_with_retry, probe_transcript, stub_sequence_transcript,
    StubOutput, StubTools,
};

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run_stub(tools: &StubTools, tool: &str, args: &[&str]) -> Output {
    let mut command = tools.stub_command(tool);
    command.args(args);
    output_with_retry(command).unwrap_or_else(|error| panic!("failed to run stub {tool}: {error}"))
}

#[cfg(unix)]
#[test]
fn text_file_busy_is_retried_until_the_stub_is_runnable() {
    use std::io;
    use std::sync::atomic::{AtomicU32, Ordering};

    let attempts = AtomicU32::new(0);
    let result = support::retry_text_file_busy(|| {
        let attempt = attempts.fetch_add(1, Ordering::Relaxed) + 1;
        if attempt < 3 {
            Err(io::Error::from_raw_os_error(libc::ETXTBSY))
        } else {
            Ok(attempt)
        }
    });

    assert_eq!(
        result.unwrap_or_else(|error| panic!("retry failed: {error}")),
        3
    );
    assert_eq!(attempts.load(Ordering::Relaxed), 3);
}

#[cfg(unix)]
#[test]
fn text_file_busy_is_returned_once_the_retry_budget_is_exhausted() {
    use std::io;
    use std::sync::atomic::{AtomicU32, Ordering};

    let attempts = AtomicU32::new(0);
    let result = support::retry_text_file_busy(|| {
        attempts.fetch_add(1, Ordering::Relaxed);
        Err::<(), _>(io::Error::from_raw_os_error(libc::ETXTBSY))
    });

    let error = match result {
        Ok(()) => panic!("expected the busy executable error"),
        Err(error) => error,
    };
    assert_eq!(error.raw_os_error(), Some(libc::ETXTBSY));
    assert!(
        attempts.load(Ordering::Relaxed) > 1,
        "the helper should retry before giving up"
    );
}

#[cfg(unix)]
#[test]
fn other_execution_errors_are_not_retried() {
    use std::io;
    use std::sync::atomic::{AtomicU32, Ordering};

    let attempts = AtomicU32::new(0);
    let result = support::retry_text_file_busy(|| {
        attempts.fetch_add(1, Ordering::Relaxed);
        Err::<(), _>(io::Error::from_raw_os_error(libc::ENOENT))
    });

    let error = match result {
        Ok(()) => panic!("expected the missing executable error"),
        Err(error) => error,
    };
    assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
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
        probe_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
}

#[cfg(not(windows))]
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
            StubOutput::stdout("jq\n"),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("firefox\n"),
            StubOutput::stdout(CASK_INFO),
            StubOutput::stderr("cask upgrade failed\n").with_status(1),
            StubOutput::stdout("==> Would autoremove 1 unneeded formula:\nold-dependency\n"),
            StubOutput::new(),
            StubOutput::stdout("Would remove: /tmp/old (1.2MB)\n"),
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
            "--cleanup",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        brew_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
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
        stub_sequence_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
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

#[cfg(unix)]
#[test]
fn a_step_that_exceeds_the_step_timeout_is_reported_as_timed_out() {
    let tools = StubTools::new(&["slow"]);
    tools.set_script("slow", "/bin/sleep 5");

    let output = tools.run("slow", &["--step-timeout", "1"]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        slow_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
    let text = stdout(&output);
    assert!(text.contains("0 failed, 1 timed out"), "summary: {text}");
    assert!(text.contains("timed out"), "summary: {text}");
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
    assert_eq!(
        tools.step_calls(),
        brew_update_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );

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
