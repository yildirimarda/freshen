//! End-to-end checks for user-defined custom steps.
//!
//! Custom steps run a command of the user's choosing, so anything freshen does
//! not have a built-in step for can be added through the configuration file
//! without code changes. These tests run the built binary against stub
//! executables on `PATH` and never touch a real package manager.

#![cfg(feature = "test-harness")]

mod support;

use std::fs;
use std::path::Path;
use std::process::Output;

use support::{output_with_retry, StubTools};

/// A step whose command is missing entirely.
const EMPTY_COMMAND_CONFIG: &str = r#"
[[custom_steps]]
name = "bad"
command = []
"#;

/// Two steps that run in the order given by their `order` field.
const ORDERED_CONFIG: &str = r#"
[[custom_steps]]
name = "custom-a"
command = ["custom-a"]
order = 2

[[custom_steps]]
name = "custom-b"
command = ["custom-b"]
order = 1
"#;

/// A step whose program is absent, so it is detected as not installed.
const MISSING_PROGRAM_CONFIG: &str = r#"
[[custom_steps]]
name = "missing"
command = ["freshen-does-not-exist-xyz"]
"#;

/// A step whose program is present, so it is detected and run.
const PRESENT_CONFIG: &str = r#"
[[custom_steps]]
name = "custom-a"
command = ["custom-a"]
"#;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn write_config(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", parent.display()));
    }
    fs::write(path, contents)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

fn run(tools: &StubTools, args: &[&str]) -> Output {
    let mut command = tools.command();
    command.args(args);
    output_with_retry(command).unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn an_empty_command_is_refused() {
    let tools = StubTools::new(&[]);
    let path = tools.home().join("custom.toml");
    write_config(&path, EMPTY_COMMAND_CONFIG);

    let output = run(&tools, &["--config", path.to_string_lossy().as_ref()]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("custom step `bad` has an empty command"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn custom_steps_run_in_order() {
    let tools = StubTools::new(&["custom-a", "custom-b"]);
    let path = tools.home().join("custom.toml");
    write_config(&path, ORDERED_CONFIG);

    let output = run(&tools, &["--config", path.to_string_lossy().as_ref()]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        ["custom-b", "custom-a"],
        "calls: {:?}\nstdout: {}\nstderr: {}",
        tools.calls(),
        stdout(&output),
        stderr(&output)
    );
}

#[test]
fn custom_steps_without_an_order_run_after_ordered_ones() {
    let tools = StubTools::new(&["custom-a", "custom-b"]);
    let config = r#"
[[custom_steps]]
name = "custom-a"
command = ["custom-a"]
order = 1

[[custom_steps]]
name = "custom-b"
command = ["custom-b"]
"#;
    let path = tools.home().join("custom.toml");
    write_config(&path, config);

    let output = run(&tools, &["--config", path.to_string_lossy().as_ref()]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        ["custom-a", "custom-b"],
        "calls: {:?}",
        tools.calls()
    );
}

#[test]
fn a_step_whose_program_is_absent_is_skipped_instead_of_run() {
    let tools = StubTools::new(&[]);
    let path = tools.home().join("custom.toml");
    write_config(&path, MISSING_PROGRAM_CONFIG);

    let output = run(&tools, &["--config", path.to_string_lossy().as_ref()]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools.calls().is_empty(),
        "expected no calls, got {:?}",
        tools.calls()
    );
}

#[test]
fn a_detected_custom_step_runs_its_command() {
    let tools = StubTools::new(&["custom-a"]);
    let path = tools.home().join("custom.toml");
    write_config(&path, PRESENT_CONFIG);

    let output = run(&tools, &["--config", path.to_string_lossy().as_ref()]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.calls(), ["custom-a"]);
}

#[test]
fn dry_run_detects_but_never_runs_a_custom_step() {
    let tools = StubTools::new(&["custom-a"]);
    let path = tools.home().join("custom.toml");
    write_config(&path, PRESENT_CONFIG);

    let output = run(
        &tools,
        &["--config", path.to_string_lossy().as_ref(), "--dry-run"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools.calls().is_empty(),
        "expected no calls in dry run, got {:?}",
        tools.calls()
    );
}

#[test]
fn list_steps_names_custom_steps() {
    let tools = StubTools::new(&["custom-a"]);
    let path = tools.home().join("custom.toml");
    write_config(&path, PRESENT_CONFIG);

    let output = run(
        &tools,
        &["--config", path.to_string_lossy().as_ref(), "--list-steps"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("custom-a"),
        "listing does not name custom-a:\n{}",
        stdout(&output)
    );
}

#[test]
fn a_custom_step_prints_its_command_before_running() {
    let tools = StubTools::new(&["custom-a"]);
    let path = tools.home().join("custom.toml");
    write_config(
        &path,
        r#"
[[custom_steps]]
name = "custom-a"
command = ["custom-a", "--flag"]
"#,
    );

    let output = run(&tools, &["--config", path.to_string_lossy().as_ref()]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("custom-a --flag"),
        "expected the exact command to be printed before it ran:\n{}",
        stdout(&output)
    );
}

#[test]
fn quiet_hides_the_custom_step_command() {
    let tools = StubTools::new(&["custom-a"]);
    let path = tools.home().join("custom.toml");
    write_config(
        &path,
        r#"
[[custom_steps]]
name = "custom-a"
command = ["custom-a", "--flag"]
"#,
    );

    let output = run(
        &tools,
        &["--config", path.to_string_lossy().as_ref(), "--quiet"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains("custom-a --flag"),
        "expected the exact command to be hidden in quiet mode:\n{}",
        stdout(&output)
    );
}

#[test]
fn a_hook_prints_its_command_before_running() {
    let tools = StubTools::new(&["custom-a", "pre-hook"]);
    let path = tools.home().join("custom.toml");
    write_config(
        &path,
        r#"
[[custom_steps]]
name = "custom-a"
command = ["custom-a"]

[hooks.custom-a]
pre = ["pre-hook", "--verify"]
"#,
    );

    let output = run(&tools, &["--config", path.to_string_lossy().as_ref()]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("pre-hook --verify"),
        "expected the hook command to be printed before it ran:\n{}",
        stdout(&output)
    );
}
