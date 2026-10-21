//! End-to-end checks for the preflight gate before an update.
//!
//! The binary is run against scripted preflight facts so these tests never
//! touch the real network, battery or disk. The stub harness normally disables
//! preflight; each test here removes that bypass explicitly.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::StubTools;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools, facts: &str, args: &[&str]) -> Output {
    tools
        .command()
        .env_remove("FRESHEN_SKIP_PREFLIGHT")
        .env("FRESHEN_TEST_STEPS", "probe run")
        .env("FRESHEN_TEST_PREFLIGHT", facts)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn an_unreachable_network_blocks_the_run_with_exit_code_three() {
    let tools = StubTools::new(&["probe"]);

    let output = run(&tools, "network=down", &[]);

    assert_eq!(output.status.code(), Some(3), "stderr: {}", stderr(&output));
    assert!(stderr(&output).contains("Preflight checks failed:"));
    assert!(stderr(&output).contains("network is unreachable"));
    assert!(stdout(&output).is_empty());
    assert!(tools.calls().is_empty(), "no step should run");
}

#[test]
fn network_configuration_failures_are_reported_specifically() {
    let cases = [
        (
            "HTTPS_PROXY",
            "socks5://proxy.example:1080",
            "invalid proxy URL",
        ),
        (
            "SSL_CERT_FILE",
            "/freshen/definitely/not/a/certificate.pem",
            "cannot read CA bundle",
        ),
    ];

    for (key, value, message) in cases {
        let tools = StubTools::new(&["probe"]);
        let mut command = tools.command();
        let output = command
            .env_remove("FRESHEN_SKIP_PREFLIGHT")
            .env_remove("FRESHEN_TEST_PREFLIGHT")
            .env_remove("HTTP_PROXY")
            .env_remove("http_proxy")
            .env_remove("HTTPS_PROXY")
            .env_remove("https_proxy")
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .env_remove("SSL_CERT_FILE")
            .env_remove("SSL_CERT_DIR")
            .env_remove("CURL_CA_BUNDLE")
            .env_remove("REQUESTS_CA_BUNDLE")
            .env_remove("NIX_SSL_CERT_FILE")
            .env_remove("NODE_EXTRA_CA_CERTS")
            .env(key, value)
            .env("FRESHEN_TEST_STEPS", "probe run")
            .args([
                "--skip-disk-check",
                "--skip-battery-check",
                "--skip-metered-check",
            ])
            .output()
            .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

        assert_eq!(
            output.status.code(),
            Some(3),
            "{key}: stderr: {}",
            stderr(&output)
        );
        let text = stderr(&output);
        assert!(text.contains(message), "{key}: {text}");
        assert!(!text.contains("network is unreachable"), "{key}: {text}");
        assert!(tools.calls().is_empty(), "{key}");
    }
}

#[test]
fn skipping_the_network_check_lets_the_run_proceed() {
    let tools = StubTools::new(&["probe"]);

    let output = run(&tools, "network=down", &["--skip-network-check"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.calls(), ["probe run"]);
}

#[test]
fn each_failing_check_can_be_skipped_on_its_own() {
    let cases = [
        ("disk=536870912", "--skip-disk-check", "only 0.5 GiB free"),
        (
            "battery=10:discharging",
            "--skip-battery-check",
            "running on battery at 10%",
        ),
        (
            "metered=yes",
            "--skip-metered-check",
            "connection is metered or a hotspot",
        ),
    ];

    for (facts, flag, message) in cases {
        let blocked = StubTools::new(&["probe"]);
        let output = run(&blocked, facts, &[]);
        assert_eq!(
            output.status.code(),
            Some(3),
            "{facts}: stderr: {}",
            stderr(&output)
        );
        assert!(stderr(&output).contains(message), "{facts}");
        assert!(blocked.calls().is_empty(), "{facts}");

        let allowed = StubTools::new(&["probe"]);
        let output = run(&allowed, facts, &[flag]);
        assert!(
            output.status.success(),
            "{facts} with {flag}: {}",
            stderr(&output)
        );
        assert_eq!(allowed.calls(), ["probe run"], "{facts} with {flag}");
    }
}

#[test]
fn custom_thresholds_are_taken_from_the_command_line() {
    let tools = StubTools::new(&["probe"]);

    let output = run(
        &tools,
        "disk=2147483648;battery=25:discharging",
        &["--min-free-disk-gib", "3", "--battery-threshold", "30"],
    );

    assert_eq!(output.status.code(), Some(3), "stderr: {}", stderr(&output));
    let text = stderr(&output);
    assert!(text.contains("only 2.0 GiB free, need at least 3.0 GiB"));
    assert!(text.contains("running on battery at 25%, below the 30% threshold"));
    assert!(tools.calls().is_empty());
}

#[test]
fn dry_run_and_check_do_not_run_preflight() {
    for args in [["--dry-run"], ["--check"]] {
        let tools = StubTools::new(&["probe"]);
        let output = run(&tools, "network=down;disk=0;metered=yes", &args);

        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        assert!(!stderr(&output).contains("Preflight checks failed"));
    }
}

#[test]
fn the_skip_environment_variable_disables_every_check() {
    let tools = StubTools::new(&["probe"]);

    let output = tools
        .command()
        .env("FRESHEN_TEST_STEPS", "probe run")
        .env(
            "FRESHEN_TEST_PREFLIGHT",
            "network=down;disk=0;battery=1;metered=yes",
        )
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.calls(), ["probe run"]);
}
