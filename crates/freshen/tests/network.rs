//! End-to-end checks for network failure handling.
//!
//! A stub `uv` reports a transient download error, a permanently unreachable
//! registry or an ordinary failure. The run must retry only the transient case,
//! skip the step when the registry cannot be reached and keep reporting other
//! failures normally. The uv step is available on every supported platform, so
//! the same transcript runs on Windows, macOS and Linux.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn uv_responses(attempt: StubOutput) -> [StubOutput; 3] {
    [StubOutput::new(), StubOutput::new(), attempt]
}

fn run_update(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "uv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn a_transient_download_error_is_retried_until_it_succeeds() {
    let tools = StubTools::new(&["uv"]);
    let responses = uv_responses(
        StubOutput::stderr("curl: (56) Failure when receiving data from the peer\n").with_status(1),
    );
    tools.set_responses("uv", &responses);

    let output = run_update(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "uv tool list --outdated",
            "uv tool upgrade --all",
            "uv tool upgrade --all"
        ]
    );
    assert!(
        stdout(&output).contains("1 ok"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn an_unreachable_registry_skips_the_step_without_failing_the_run() {
    let tools = StubTools::new(&["uv"]);
    let responses = uv_responses(
        StubOutput::stderr("curl: (6) Could not resolve host: pypi.org\n").with_status(1),
    );
    tools.set_responses("uv", &responses);

    let output = run_update(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        ["uv tool list --outdated", "uv tool upgrade --all"]
    );
    let text = stdout(&output);
    assert!(text.contains("1 skipped"), "summary: {text}");
    assert!(text.contains("registry unreachable"), "summary: {text}");
    assert!(text.contains("0 failed"), "summary: {text}");
}

#[test]
fn an_ordinary_failure_is_not_retried_or_skipped() {
    let tools = StubTools::new(&["uv"]);
    let responses = uv_responses(StubOutput::stderr("error: tool not found\n").with_status(1));
    tools.set_responses("uv", &responses);

    let output = run_update(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        ["uv tool list --outdated", "uv tool upgrade --all"]
    );
    let text = stdout(&output);
    assert!(text.contains("1 failed"), "summary: {text}");
    assert!(!text.contains("registry unreachable"), "summary: {text}");
}
