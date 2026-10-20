//! End-to-end checks for the `freshen` binary's command-line surface.
//!
//! The binary is run with an empty step registry, so these tests never touch a
//! package manager or any other real tool on the host.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    match Command::new(env!("CARGO_BIN_EXE_freshen"))
        .args(args)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("failed to run freshen: {error}"),
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_lists_every_flag() {
    let output = run(&["--help"]);

    assert!(output.status.success());
    let text = stdout(&output);
    for flag in [
        "--dry-run",
        "--list-steps",
        "--only",
        "--skip",
        "--yes",
        "--force-kill",
    ] {
        assert!(text.contains(flag), "help does not mention {flag}: {text}");
    }
}

#[test]
fn list_steps_with_no_steps_succeeds_and_says_so() {
    let output = run(&["--list-steps"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "No known steps.\n");
}

#[test]
fn version_prints_the_binary_name_and_version() {
    let output = run(&["--version"]);

    assert!(output.status.success());
    assert!(stdout(&output).starts_with("freshen "));
}

#[test]
fn dry_run_with_no_steps_succeeds_and_plans_nothing() {
    let output = run(&["--dry-run"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("0 ok, 0 planned, 0 skipped, 0 failed"));
}

#[test]
fn yes_and_force_kill_are_accepted_with_dry_run() {
    let output = run(&["--yes", "--force-kill", "--dry-run"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
}

#[test]
fn unknown_only_step_fails_with_a_clear_message() {
    let output = run(&["--only", "nope"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("unknown step `nope` in --only"));
}

#[test]
fn unknown_skip_step_fails_with_a_clear_message() {
    let output = run(&["--skip", "nope"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("unknown step `nope` in --skip"));
}
