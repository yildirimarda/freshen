//! Parity tests for the Conda step.
//!
//! Every test runs the built binary against a stub `conda` executable and
//! asserts the exact command transcript. No real conda is touched: the run
//! upgrades conda in the base environment, and the check reports the other
//! environments without upgrading anything.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "conda"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn env_list_output() -> &'static [&'static str] {
    &[
        "# conda environments:",
        "#",
        "base                  *  /opt/conda",
        "foo                     /opt/conda/envs/foo",
        "bar                    /opt/conda/envs/bar",
    ]
}

#[test]
fn conda_updates_base_conda() {
    let tools = StubTools::new(&["conda"]);
    tools.set_responses("conda", &[StubOutput::new()]);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // The runner checks before it runs, so the environment listing precedes
    // the base conda update.
    assert_eq!(
        tools.step_calls(),
        vec![
            "conda env list".to_string(),
            "conda update -n base conda".to_string()
        ],
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("conda: update -n base conda"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn conda_skips_when_conda_is_absent() {
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
fn a_failing_conda_update_fails_the_step() {
    let tools = StubTools::new(&["conda"]);
    tools.set_responses(
        "conda",
        &[
            StubOutput::new(),                // conda env list (pre-run check)
            StubOutput::new().with_status(1), // conda update -n base conda fails
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "conda env list".to_string(),
            "conda update -n base conda".to_string()
        ],
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_other_environments_and_exits_2() {
    let tools = StubTools::new(&["conda"]);
    tools.set_responses("conda", &[StubOutput::stdout(env_list_output().join("\n"))]);

    let output = tools
        .command()
        .args(["--check", "--only", "conda"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::conda_check_transcript(),
        "calls: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(text.contains("foo"), "output: {text}");
    assert!(text.contains("bar"), "output: {text}");
    assert!(!text.contains("base"), "base must not be reported: {text}");
    assert!(
        text.contains("2 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
}

#[test]
fn check_reports_conda_up_to_date_when_only_base_exists() {
    let tools = StubTools::new(&["conda"]);
    tools.set_responses("conda", &[StubOutput::stdout("base  *  /opt/conda\n")]);

    let output = tools
        .command()
        .args(["--check", "--only", "conda"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::conda_check_transcript(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_reports_a_failed_conda_env_list_with_exit_1() {
    let tools = StubTools::new(&["conda"]);
    tools.set_responses("conda", &[StubOutput::new().with_status(1)]);

    let output = tools
        .command()
        .args(["--check", "--only", "conda"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::conda_check_transcript(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("failed during check"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn run_never_touches_other_environments() {
    let tools = StubTools::new(&["conda"]);
    tools.set_responses("conda", &[StubOutput::new()]);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // Report-only environments are never upgraded: only the base conda
    // update runs, preceded by the read-only environment listing.
    assert_eq!(
        tools.step_calls(),
        vec![
            "conda env list".to_string(),
            "conda update -n base conda".to_string()
        ],
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        !tools.step_calls().iter().any(|call| {
            (call.starts_with("conda update -n ") && call != "conda update -n base conda")
                || call.starts_with("conda install")
                || call.starts_with("conda remove")
        }),
        "calls: {:?}",
        tools.step_calls()
    );
}
