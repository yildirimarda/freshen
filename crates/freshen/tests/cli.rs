//! End-to-end checks for the `freshen` binary's command-line surface.
//!
//! These tests never run a package manager: detection only looks for an
//! executable on `PATH`, and the run cases exclude every built-in step.

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
        "--check",
        "--list-steps",
        "--json",
        "--markdown",
        "--only",
        "--skip",
        "--yes",
        "--force-kill",
        "--skip-network-check",
        "--skip-disk-check",
        "--skip-battery-check",
        "--skip-metered-check",
        "--min-free-disk-gib",
        "--battery-threshold",
    ] {
        assert!(text.contains(flag), "help does not mention {flag}: {text}");
    }
}

#[test]
fn list_steps_lists_every_builtin_step() {
    let output = run(&["--list-steps"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Steps (11):"), "listing: {text}");
    assert!(text.contains("brew"), "listing: {text}");
    assert!(text.contains("brew-casks"), "listing: {text}");
    assert!(text.contains("colima-stop"), "listing: {text}");
    assert!(text.contains("docker"), "listing: {text}");
    assert!(text.contains("uv"), "listing: {text}");
    assert!(text.contains("mise"), "listing: {text}");
}

#[test]
fn version_prints_the_binary_name_and_version() {
    let output = run(&["--version"]);

    assert!(output.status.success());
    assert!(stdout(&output).starts_with("freshen "));
}

#[test]
fn dry_run_with_every_builtin_step_excluded_plans_nothing() {
    let output = run(&[
        "--dry-run",
        "--skip",
        "brew,brew-formulae,brew-casks,brew-autoremove,brew-cleanup,brew-reopen,colima-stop,colima-start,docker,uv,mise",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("0 ok, 0 planned, 0 skipped, 0 failed"));
}

#[test]
fn check_with_every_builtin_step_excluded_reports_up_to_date() {
    let output = run(&[
        "--check",
        "--skip",
        "brew,brew-formulae,brew-casks,brew-autoremove,brew-cleanup,brew-reopen,colima-stop,colima-start,docker,uv,mise",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "Everything is up to date.\n");
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
