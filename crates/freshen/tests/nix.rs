//! Parity tests for the Nix channel step.
//!
//! Every test runs the built binary against a stub `nix` executable and
//! asserts the exact command transcript of the `nix channel --update` step.
//! No real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "nix"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn run_profile(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "nix-profile"])
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
#[cfg(target_os = "linux")]
fn nix_updates_channels() {
    let tools = StubTools::new(&["nix"]);
    tools.set_responses(
        "nix",
        &[StubOutput::new()], // nix channel --update
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::nix_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn nix_skips_when_nix_is_missing() {
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
#[cfg(target_os = "linux")]
fn a_failing_nix_channel_update_fails_the_step() {
    let tools = StubTools::new(&["nix"]);
    tools.set_responses(
        "nix",
        &[StubOutput::new().with_status(1)], // nix channel --update fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::nix_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn check_reports_nix_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["nix"]);

    let output = tools
        .command()
        .args(["--check", "--only", "nix"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
#[cfg(target_os = "linux")]
fn nix_profile_upgrades_profiles() {
    let tools = StubTools::new(&["nix"]);
    tools.set_responses(
        "nix",
        &[StubOutput::new()], // nix profile upgrade --all
    );

    let output = run_profile(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::nix_profile_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn nix_profile_skips_when_nix_is_missing() {
    let tools = StubTools::new(&[]);

    let output = run_profile(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_nix_profile_upgrade_fails_the_step() {
    let tools = StubTools::new(&["nix"]);
    tools.set_responses(
        "nix",
        &[StubOutput::new().with_status(1)], // nix profile upgrade --all fails
    );

    let output = run_profile(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::nix_profile_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn check_reports_nix_profile_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["nix"]);

    let output = tools
        .command()
        .args(["--check", "--only", "nix-profile"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

fn run_home_manager(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "home-manager"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn home_manager_config(tools: &StubTools) -> PathBuf {
    tools.config_home().join("nixpkgs").join("home.nix")
}

fn write_home_manager_config(tools: &StubTools) {
    let config = home_manager_config(tools);
    let parent = config.parent().unwrap_or_else(|| {
        panic!(
            "home-manager config path has no parent: {}",
            config.display()
        )
    });
    fs::create_dir_all(parent)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", parent.display()));
    fs::write(&config, "{ config, ... }: {}\n")
        .unwrap_or_else(|error| panic!("failed to write home-manager config: {error}"));
}

#[test]
#[cfg(target_os = "linux")]
fn home_manager_switches_when_config_is_detected() {
    let tools = StubTools::new(&["home-manager"]);
    write_home_manager_config(&tools);
    tools.set_responses("home-manager", &[StubOutput::new()]); // home-manager switch

    let output = run_home_manager(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::home_manager_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn home_manager_skips_when_no_config_is_present() {
    let tools = StubTools::new(&["home-manager"]);

    let output = run_home_manager(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_home_manager_switch_fails_the_step() {
    let tools = StubTools::new(&["home-manager"]);
    write_home_manager_config(&tools);
    tools.set_responses(
        "home-manager",
        &[StubOutput::new().with_status(1)], // home-manager switch fails
    );

    let output = run_home_manager(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::home_manager_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

fn run_collect_garbage_with_cleanup(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "nix-collect-garbage", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn run_collect_garbage(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "nix-collect-garbage"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
#[cfg(target_os = "linux")]
fn nix_collect_garbage_collects_with_cleanup() {
    let tools = StubTools::new(&["nix", "nix-collect-garbage"]);
    tools.set_responses(
        "nix-collect-garbage",
        &[StubOutput::new()], // nix-collect-garbage
    );

    let output = run_collect_garbage_with_cleanup(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::nix_collect_garbage_transcript(),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
#[cfg(target_os = "linux")]
fn nix_collect_garbage_reports_reclaimed_space() {
    let tools = StubTools::new(&["nix", "nix-collect-garbage"]);
    tools.set_responses(
        "nix-collect-garbage",
        &[StubOutput::stdout("reclaimed 2.5GB\n")], // nix-collect-garbage
    );

    let output = run_collect_garbage_with_cleanup(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::nix_collect_garbage_transcript(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("reclaimed space: 2.5GB"),
        "stdout: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn nix_collect_garbage_skips_without_cleanup_flag() {
    let tools = StubTools::new(&["nix"]);
    tools.set_responses("nix-collect-garbage", &[StubOutput::new()]);

    let output = run_collect_garbage(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        Vec::<String>::new(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("skipped destructive cleanup"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn nix_collect_garbage_skips_when_nix_is_missing() {
    let tools = StubTools::new(&["nix-collect-garbage"]);

    let output = run_collect_garbage_with_cleanup(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn check_reports_nix_collect_garbage_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["nix"]);

    let output = tools
        .command()
        .args(["--check", "--only", "nix-collect-garbage"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
