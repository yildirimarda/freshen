//! Parity tests for the Helm step.
//!
//! Every test runs the built binary against a stub `helm` executable and
//! asserts the exact command transcript of the `helm repo update` step. No
//! real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "helm"])
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
fn helm_updates_repositories() {
    let tools = StubTools::new(&["helm"]);
    tools.set_responses(
        "helm",
        &[StubOutput::new()], // helm repo update
    );

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::helm_transcript());
}

#[test]
fn helm_skips_when_helm_is_missing() {
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
fn check_reports_helm_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["helm"]);

    let output = tools
        .command()
        .args(["--check", "--only", "helm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

fn run_releases(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "helm-releases"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn helm_releases_skips_when_helm_is_absent() {
    let tools = StubTools::new(&[]);

    let output = run_releases(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

fn release_list() -> &'static [&'static str] {
    &[
        r#"[{"name":"my-release","namespace":"default","chart":"nginx-ingress-1.2.3","app_version":"1.2.3"},{"name":"db","namespace":"default","chart":"mysql-8.0.35","app_version":"8.0.35"}]"#,
    ]
}

#[test]
fn check_reports_releases_with_newer_chart_versions() {
    let tools = StubTools::new(&["helm"]);
    tools.set_responses(
        "helm",
        &[
            // `helm list -o json`
            StubOutput::stdout(release_list()[0]),
            // `helm search repo nginx-ingress -o json`
            StubOutput::stdout(
                r#"[{"chart":"stable/nginx-ingress","version":"2.0.0","app_version":"2.0.0"}]"#,
            ),
            // `helm search repo mysql -o json`
            StubOutput::stdout(r#"[{"chart":"mysql","version":"8.0.35","app_version":"8.0.35"}]"#),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "helm-releases"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "helm list -o json",
            "helm search repo nginx-ingress -o json",
            "helm search repo mysql -o json",
        ],
        "calls: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(text.contains("helm releases:"), "output: {text}");
    assert!(
        text.contains("my-release 1.2.3 -> 2.0.0"),
        "missing outdated report: {text}"
    );
    assert!(
        text.contains("1 update(s) pending"),
        "missing pending summary: {text}"
    );
}

#[test]
fn check_reports_helm_releases_up_to_date() {
    let tools = StubTools::new(&["helm"]);
    tools.set_responses(
        "helm",
        &[
            // `helm list -o json`
            StubOutput::stdout(
                r#"[{"name":"my-release","namespace":"default","chart":"nginx-ingress-2.0.0","app_version":"2.0.0"}]"#,
            ),
            // `helm search repo nginx-ingress -o json`
            StubOutput::stdout(r#"[{"chart":"stable/nginx-ingress","version":"2.0.0","app_version":"2.0.0"}]"#),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "helm-releases"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "helm list -o json",
            "helm search repo nginx-ingress -o json",
        ],
        "calls: {:?}",
        tools.step_calls()
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn run_never_upgrades_anything() {
    let tools = StubTools::new(&["helm"]);
    tools.set_responses(
        "helm",
        &[
            StubOutput::stdout(release_list()[0]),
            StubOutput::stdout(
                r#"[{"chart":"stable/nginx-ingress","version":"2.0.0","app_version":"2.0.0"}]"#,
            ),
            StubOutput::stdout(r#"[{"chart":"mysql","version":"8.0.35","app_version":"8.0.35"}]"#),
        ],
    );

    let output = run_releases(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // Report-only: no mutating helm command may ever run.
    assert!(
        !tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("helm upgrade") || call.starts_with("helm install")),
        "calls: {:?}",
        tools.step_calls()
    );
}
