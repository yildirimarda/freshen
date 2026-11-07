//! Parity tests for the Go step.
//!
//! Every test runs the built binary against a stub `go` executable and asserts
//! the exact command transcript of the `go list -m -u` step. No real tool is
//! touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools, reinstall: bool) -> Output {
    let mut args = vec!["--only", "go"];
    if reinstall {
        args.push("--reinstall-go");
    }
    tools
        .command()
        .args(&args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn outdated_response() -> StubOutput {
    StubOutput::stdout("example.com/old v1.0.0 [v1.1.0]\nexample.com/current v2.0.0\n")
}

#[test]
fn go_check_reports_outdated_modules() {
    let tools = StubTools::new(&["go"]);
    tools.set_responses("go", &[outdated_response()]); // go list -m -u

    let output = tools
        .command()
        .args(["--check", "--only", "go"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    // `--check` exits with 2 when updates are pending.
    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::go_transcript());
    assert!(
        stdout(&output).contains("example.com/old v1.0.0 -> v1.1.0"),
        "stdout: {}",
        stdout(&output)
    );
    assert!(
        !stdout(&output).contains("example.com/current"),
        "up-to-date module must not be reported: {}",
        stdout(&output)
    );
}

#[test]
fn go_check_reports_up_to_date_without_outdated() {
    let tools = StubTools::new(&["go"]);
    tools.set_responses(
        "go",
        &[StubOutput::stdout("example.com/current v2.0.0\n")], // go list -m -u
    );

    let output = tools
        .command()
        .args(["--check", "--only", "go"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::go_transcript());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn go_skips_when_go_is_absent() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, false);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn a_failing_go_list_fails_the_step() {
    // A run queries `go list -m -u` before and during the upgrade, so every
    // query must fail for the step to be reported as failed.
    let tools = StubTools::new(&["go"]);
    tools.set_responses(
        "go",
        &[
            StubOutput::new().with_status(1), // go list -m -u (before)
            StubOutput::new().with_status(1), // go list -m -u (upgrade)
        ],
    );

    let output = run(&tools, true);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.calls(), vec!["go list -m -u", "go list -m -u"]);
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn go_run_installs_outdated_modules() {
    let tools = StubTools::new(&["go"]);
    tools.set_responses(
        "go",
        &[
            outdated_response(), // go list -m -u (before)
            outdated_response(), // go list -m -u (upgrade)
            StubOutput::new(),   // go install example.com/old@v1.1.0
            outdated_response(), // go list -m -u (after, for the version diff)
        ],
    );

    let output = run(&tools, true);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        vec![
            "go list -m -u",
            "go list -m -u",
            "go install example.com/old@v1.1.0",
            "go list -m -u",
        ]
    );
}

#[test]
fn go_run_without_the_opt_in_does_not_reinstall() {
    let tools = StubTools::new(&["go"]);
    tools.set_responses(
        "go",
        &[
            outdated_response(), // go list -m -u (before)
            outdated_response(), // go list -m -u (after, for the version diff)
        ],
    );

    let output = run(&tools, false);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        vec!["go list -m -u", "go list -m -u"],
        "calls: {:?}",
        tools.calls()
    );
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call.starts_with("go install")),
        "no binary may be reinstalled without --reinstall-go: {:?}",
        tools.calls()
    );
}
