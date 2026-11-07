//! Parity tests for the asdf step.
//!
//! Every test runs the built binary against a stub `asdf` executable and
//! asserts the exact command transcript. No real package manager is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::fs;
use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "asdf"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn asdf_updates_all_plugins() {
    let tools = StubTools::new(&["asdf"]);
    tools.set_responses("asdf", &[StubOutput::new()]); // asdf plugin update --all

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::asdf_transcript());
}

#[test]
fn asdf_skips_when_asdf_is_absent() {
    let tools = StubTools::new(&[]);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn a_failing_asdf_plugin_update_fails_the_step() {
    let tools = StubTools::new(&["asdf"]);
    tools.set_responses(
        "asdf",
        &[StubOutput::new().with_status(1)], // asdf plugin update --all fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::asdf_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_asdf_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["asdf"]);

    let output = tools
        .command()
        .args(["--check", "--only", "asdf"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_reports_outdated_asdf_tools_and_exits_2() {
    let tools = StubTools::new(&["asdf"]);
    fs::write(
        tools.home().join(".tool-versions"),
        "nodejs 20.0.0\npython 3.11.0\n",
    )
    .unwrap_or_else(|error| panic!("failed to write .tool-versions: {error}"));
    tools.set_responses(
        "asdf",
        &[
            // asdf latest nodejs
            StubOutput::stdout("20.18.0\n"),
            // asdf latest python
            StubOutput::stdout("3.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "asdf"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "asdf latest nodejs".to_string(),
            "asdf latest python".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(text.contains("asdf:"), "output: {text}");
    assert!(text.contains("nodejs 20.0.0 -> 20.18.0"), "output: {text}");
    assert!(text.contains("python 3.11.0 -> 3.12.0"), "output: {text}");
    assert!(
        text.contains("2 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
    // Nothing is changed: `asdf plugin update --all` never runs.
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call == "asdf plugin update --all"),
        "calls: {:?}",
        tools.calls()
    );
}

#[test]
fn check_reports_asdf_up_to_date_when_versions_match() {
    let tools = StubTools::new(&["asdf"]);
    fs::write(tools.home().join(".tool-versions"), "nodejs 20.18.0\n")
        .unwrap_or_else(|error| panic!("failed to write .tool-versions: {error}"));
    tools.set_responses("asdf", &[StubOutput::stdout("20.18.0\n")]);

    let output = tools
        .command()
        .args(["--check", "--only", "asdf"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["asdf latest nodejs".to_string()]);
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_reports_asdf_up_to_date_without_tool_versions() {
    let tools = StubTools::new(&["asdf"]);

    let output = tools
        .command()
        .args(["--check", "--only", "asdf"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_skips_asdf_when_asdf_is_absent() {
    let tools = StubTools::new(&[]);
    fs::write(tools.home().join(".tool-versions"), "nodejs 20.0.0\n")
        .unwrap_or_else(|error| panic!("failed to write .tool-versions: {error}"));

    let output = tools
        .command()
        .args(["--check", "--only", "asdf"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
