//! Parity tests for the .NET global tools step.
//!
//! Every test runs the built binary against a stub `dotnet` executable and
//! asserts the exact command transcript of the `dotnet tool list --global`
//! + `dotnet tool update --global <tool>` block. No real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "dotnet-tools", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn list_output() -> &'static str {
    "\
Package Id         Versions
--------------------------------------
tool-one           1.0.0
tool-two           2.0.0
"
}

#[test]
fn dotnet_tools_updates_each_installed_tool() {
    let tools = StubTools::new(&["dotnet"]);
    tools.set_responses(
        "dotnet",
        &[
            StubOutput::stdout(list_output()), // dotnet tool list --global
            StubOutput::new(),                 // dotnet tool update --global tool-one
            StubOutput::new(),                 // dotnet tool update --global tool-two
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::dotnet_tools_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
}

#[test]
fn dotnet_tools_skips_when_dotnet_is_absent() {
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
fn a_failing_dotnet_tool_update_fails_the_step() {
    let tools = StubTools::new(&["dotnet"]);
    tools.set_responses(
        "dotnet",
        &[
            StubOutput::stdout(list_output()), // dotnet tool list --global
            StubOutput::new().with_status(1),  // dotnet tool update --global tool-one fails
            StubOutput::new(),                 // dotnet tool update --global tool-two still runs
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::dotnet_tools_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_dotnet_tools_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["dotnet"]);

    let output = tools
        .command()
        .args(["--check", "--only", "dotnet-tools"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
