//! `--verbose` and `--quiet` change how much human output the binary prints.
//!
//! The probe steps emit an informational line with `human!` and a verbose
//! line with `human_verbose!`, so a run against a stubbed `probe` step lets us
//! assert which lines survive each output level. These tests need the
//! `test-harness` feature, which registers the probe steps.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::StubTools;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools, args: &[&str]) -> Output {
    tools.run("probe", args)
}

#[test]
fn quiet_suppresses_informational_output() {
    let tools = StubTools::new(&["probe"]);
    let output = run(&tools, &["--quiet"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        !text.contains("running probe"),
        "quiet suppresses progress: {text}"
    );
    assert!(
        !text.contains("verbose diagnostics"),
        "quiet suppresses verbose detail: {text}"
    );
}

#[test]
fn default_shows_progress_but_not_verbose_detail() {
    let tools = StubTools::new(&["probe"]);
    let output = run(&tools, &[]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("running probe"),
        "default shows progress: {text}"
    );
    assert!(
        !text.contains("verbose diagnostics"),
        "default hides verbose detail: {text}"
    );
}

#[test]
fn verbose_shows_progress_and_verbose_detail() {
    let tools = StubTools::new(&["probe"]);
    let output = run(&tools, &["--verbose"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("running probe"),
        "verbose shows progress: {text}"
    );
    assert!(
        text.contains("verbose diagnostics"),
        "verbose shows diagnostics: {text}"
    );
}

#[test]
fn quiet_and_verbose_can_be_combined_with_dry_run() {
    let tools = StubTools::new(&["probe"]);
    for args in [["--quiet", "--dry-run"], ["--verbose", "--dry-run"]] {
        let output = run(&tools, &args);
        assert!(
            output.status.success(),
            "{args:?}: stderr: {}",
            stderr(&output)
        );
    }
}
