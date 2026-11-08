//! Parity tests for the gem step.
//!
//! Every test runs the built binary against stub `gem` (and, where a system
//! package manager owns Ruby, stub package-manager) executables and asserts the
//! exact command transcript. No real package manager is touched.

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

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "gem"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn gem_updates_user_installed_gems() {
    let tools = StubTools::new(&["gem"]);
    tools.set_responses("gem", &[StubOutput::new()]);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["gem update".to_string()]);
    assert!(
        stdout(&output).contains("gem: update"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn gem_update_runs_even_when_ruby_is_managed_by_brew() {
    let tools = StubTools::new(&["gem", "brew"]);
    tools.set_responses("brew", &[StubOutput::new()]);
    tools.set_responses("gem", &[StubOutput::new()]);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // User-installed gems are independent of the system package manager, so
    // `gem update` runs and brew is not consulted for RubyGems itself.
    assert_eq!(tools.calls(), vec!["gem update".to_string()]);
    assert!(
        stdout(&output).contains("gem: update"),
        "gem update must run for user-installed gems even when brew owns Ruby: {}",
        stdout(&output)
    );
}

#[test]
fn gem_update_runs_even_when_ruby_is_managed_by_dpkg() {
    let tools = StubTools::new(&["gem", "dpkg"]);
    tools.set_responses("dpkg", &[StubOutput::new()]);
    tools.set_responses("gem", &[StubOutput::new()]);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // User-installed gems are independent of the system package manager, so
    // `gem update` runs and dpkg is not consulted for RubyGems itself.
    assert_eq!(tools.calls(), vec!["gem update".to_string()]);
    assert!(
        stdout(&output).contains("gem: update"),
        "gem update must run for user-installed gems even when dpkg owns Ruby: {}",
        stdout(&output)
    );
}

#[test]
fn gem_skips_when_gem_is_absent() {
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
fn a_failing_gem_update_fails_the_step() {
    let tools = StubTools::new(&["gem"]);
    tools.set_responses("gem", &[StubOutput::new().with_status(1)]);

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["gem update".to_string()]);
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_gem_up_to_date_without_running_anything() {
    let tools = StubTools::new(&["gem"]);

    let output = tools
        .command()
        .args(["--check", "--only", "gem"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
