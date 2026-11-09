//! Parity tests for the AWS CLI v2 step.
//!
//! Every test runs the built binary against a stub `aws` executable and
//! asserts the exact command transcript. No real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--check", "--only", "aws"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn run_normal(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "aws"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn aws_version_output(version: &str) -> String {
    format!("aws-cli/{version} Python/3.11.9 Darwin/23.4.0 source/x86_64 prompt/off\n")
}

#[test]
fn check_reports_newer_aws_cli_version_for_non_brew_install() {
    let tools = StubTools::new(&["aws"]);
    tools.set_responses(
        "aws",
        &[
            // detect: `aws --version`
            StubOutput::stdout(aws_version_output("2.10.0")),
            // check: `aws --version`
            StubOutput::stdout(aws_version_output("2.10.0")),
        ],
    );

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec!["aws --version", "aws --version"],
        "calls: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(
        text.contains("2.10.0 -> 2.16.0"),
        "missing outdated report: {text}"
    );
    assert!(
        text.contains("1 update(s) pending"),
        "missing pending summary: {text}"
    );
}

#[test]
fn check_reports_aws_cli_up_to_date() {
    let tools = StubTools::new(&["aws"]);
    tools.set_responses(
        "aws",
        &[
            // detect: `aws --version`
            StubOutput::stdout(aws_version_output("2.16.0")),
            // check: `aws --version`
            StubOutput::stdout(aws_version_output("2.16.0")),
        ],
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["aws --version", "aws --version"]);
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_skips_when_aws_is_absent() {
    let tools = StubTools::new(&[]);

    let output = run_normal(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_skips_brew_managed_install() {
    let tools = StubTools::new(&["aws", "brew"]);

    let output = run_normal(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // Homebrew-managed installations are handled by the `brew` step, so the
    // AWS CLI step is skipped without ever invoking `aws`.
    assert!(
        tools.step_calls().is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_skips_aws_cli_v1() {
    let tools = StubTools::new(&["aws"]);
    tools.set_responses("aws", &[StubOutput::stdout(aws_version_output("1.29.34"))]);

    let output = run_normal(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), vec!["aws --version"]);
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn run_never_upgrades_anything() {
    let tools = StubTools::new(&["aws"]);
    tools.set_responses("aws", &[StubOutput::stdout(aws_version_output("2.10.0"))]);

    let output = tools
        .command()
        .args(["--only", "aws"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // Report-only: no mutating aws command may ever run.
    assert!(
        !tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("aws install") || call.starts_with("aws upgrade")),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn run_without_the_opt_in_is_report_only() {
    let tools = StubTools::new(&["aws"]);
    tools.set_responses("aws", &[StubOutput::stdout(aws_version_output("2.10.0"))]);

    let output = tools
        .command()
        .args(["--only", "aws"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // Without the opt-in flag no installer command may run.
    assert!(
        !tools.step_calls().iter().any(|call| {
            call.starts_with("curl") || call.starts_with("unzip") || call.starts_with("sudo")
        }),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn run_with_the_opt_in_installs_via_the_official_installer() {
    let tools = StubTools::new(&["aws", "curl", "unzip", "sudo"]);
    tools.set_responses(
        "aws",
        &[
            // detect: `aws --version`
            StubOutput::stdout(aws_version_output("2.10.0")),
            // check before the run: `aws --version`
            StubOutput::stdout(aws_version_output("2.10.0")),
        ],
    );
    tools.set_responses("curl", &[StubOutput::new()]);
    tools.set_responses("unzip", &[StubOutput::new()]);
    tools.set_responses("sudo", &[StubOutput::new()]);

    let arch = std::env::consts::ARCH;
    let url = format!("https://awscli.amazonaws.com/awscli-exe-linux-{arch}.zip");

    let output = tools
        .command()
        .args(["--only", "aws", "--install-aws-cli-updates"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "aws --version".to_string(),
            "aws --version".to_string(),
            format!("curl {url} -o awscliv2.zip"),
            "unzip -o awscliv2.zip".to_string(),
            "sudo ./aws/install".to_string(),
            // check after the run to compute the version diff
            "aws --version".to_string(),
        ],
        "calls: {:?}",
        tools.step_calls()
    );
}
