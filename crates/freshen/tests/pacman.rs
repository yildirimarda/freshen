//! Parity tests for the pacman step.
//!
//! Every test runs the built binary against a stub `pacman` executable and
//! asserts the exact command transcript of the `pacman -Syu --noconfirm`
//! step. No real package manager is touched. The pacman config directory the
//! step scans for `.pacnew` and `.pacsave` files is redirected into the stub
//! home so the real `/etc` is never read.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::fs;
use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .env("FRESHEN_PACMAN_CONFIG_DIR", tools.home().join("etc"))
        .args(["--only", "pacman"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn run_with_cleanup(tools: &StubTools) -> Output {
    tools
        .command()
        .env("FRESHEN_PACMAN_CONFIG_DIR", tools.home().join("etc"))
        .args(["--only", "pacman", "--cleanup"])
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
fn pacman_upgrades_packages() {
    let tools = StubTools::new(&["pacman"]);
    tools.set_responses(
        "pacman",
        &[StubOutput::new()], // pacman -Syu --noconfirm
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::pacman_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn reports_pacnew_and_pacsave_files_after_a_successful_upgrade() {
    let tools = StubTools::new(&["pacman"]);
    tools.set_responses(
        "pacman",
        &[StubOutput::new()], // pacman -Syu --noconfirm succeeds
    );

    let config_dir = tools.home().join("etc");
    fs::create_dir_all(&config_dir)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", config_dir.display()));
    fs::write(config_dir.join("pacman.conf.pacnew"), "")
        .unwrap_or_else(|error| panic!("failed to write .pacnew: {error}"));
    fs::write(config_dir.join("group.pacsave"), "")
        .unwrap_or_else(|error| panic!("failed to write .pacsave: {error}"));
    fs::write(config_dir.join("hosts"), "")
        .unwrap_or_else(|error| panic!("failed to write hosts: {error}"));

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains("pacman.conf.pacnew"),
        "summary lists .pacnew files: {out}"
    );
    assert!(
        out.contains("group.pacsave"),
        "summary lists .pacsave files: {out}"
    );
    assert!(
        !out.contains("hosts"),
        "non-matching files are not reported: {out}"
    );
    assert_eq!(tools.step_calls(), support::pacman_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn json_report_includes_pacman_findings() {
    let tools = StubTools::new(&["pacman"]);
    tools.set_responses(
        "pacman",
        &[StubOutput::new()], // pacman -Syu --noconfirm succeeds
    );

    let config_dir = tools.home().join("etc");
    fs::create_dir_all(&config_dir)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", config_dir.display()));
    fs::write(config_dir.join("pacman.conf.pacnew"), "")
        .unwrap_or_else(|error| panic!("failed to write .pacnew: {error}"));

    let output = tools
        .command()
        .env("FRESHEN_PACMAN_CONFIG_DIR", &config_dir)
        .args(["--json", "--only", "pacman"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let report: serde_json::Value = serde_json::from_str(&stdout(&output))
        .unwrap_or_else(|error| panic!("invalid JSON: {error}"));
    let steps = report["steps"]
        .as_array()
        .unwrap_or_else(|| panic!("report steps is not an array: {report}"));
    let pacman = steps
        .iter()
        .find(|step| step["id"].as_str() == Some("pacman"))
        .unwrap_or_else(|| panic!("pacman step is not in the report: {report}"));
    let findings = pacman["pacman_findings"]
        .as_array()
        .unwrap_or_else(|| panic!("pacman_findings is not an array: {pacman}"));
    assert_eq!(
        findings.len(),
        1,
        "json report lists the injected .pacnew file"
    );
    assert_eq!(findings[0]["kind"].as_str(), Some("pacnew"));
    assert_eq!(
        findings[0]["path"].as_str(),
        Some(config_dir.join("pacman.conf.pacnew").to_str().unwrap_or(""))
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_failing_pacman_upgrade_fails_the_step() {
    let tools = StubTools::new(&["pacman"]);
    tools.set_responses(
        "pacman",
        &[StubOutput::new().with_status(1)], // pacman -Syu --noconfirm fails
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::pacman_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_missing_pacman_is_skipped_without_calling_anything() {
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
fn check_reports_pacman_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["pacman"]);

    let output = tools
        .command()
        .env("FRESHEN_PACMAN_CONFIG_DIR", tools.home().join("etc"))
        .args(["--check", "--only", "pacman"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
#[cfg(target_os = "linux")]
fn pacman_cleans_cache_and_removes_orphans_with_cleanup() {
    let tools = StubTools::new(&["pacman"]);
    tools.set_responses(
        "pacman",
        &[
            StubOutput::new(),                // pacman -Syu --noconfirm
            StubOutput::new(),                // pacman -Sc --noconfirm
            StubOutput::stdout("foo\nbar\n"), // pacman -Qtdq: two orphans
            StubOutput::new(),                // pacman -Rns --noconfirm foo bar
        ],
    );

    let output = run_with_cleanup(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::pacman_cleanup_transcript());
}

#[test]
#[cfg(target_os = "linux")]
fn pacman_removes_no_orphans_when_none_with_cleanup() {
    let tools = StubTools::new(&["pacman"]);
    tools.set_responses(
        "pacman",
        &[
            StubOutput::new(), // pacman -Syu --noconfirm
            StubOutput::new(), // pacman -Sc --noconfirm
            StubOutput::new(), // pacman -Qtdq: no orphans
        ],
    );

    let output = run_with_cleanup(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "pacman -Syu --noconfirm",
            "pacman -Sc --noconfirm",
            "pacman -Qtdq",
        ]
    );
    assert!(
        stdout(&output).contains("pacman: orphans: nothing to remove"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn pacman_skips_cleanup_without_cleanup_flag() {
    let tools = StubTools::new(&["pacman"]);
    tools.set_responses(
        "pacman",
        &[StubOutput::new()], // pacman -Syu --noconfirm
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["pacman -Syu --noconfirm"]);
    assert!(
        stdout(&output).contains("skipped destructive cleanup"),
        "summary: {}",
        stdout(&output)
    );
}
