//! Parity tests for the uv and mise steps.
//!
//! Every test runs the built binary against stub `uv`, `mise` and `brew`
//! executables and asserts the exact command transcript of the corresponding
//! block in `update-all.sh`. No real tool is touched.

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

fn install_uv(tools: &StubTools) {
    tools.set_script(
        "uv",
        r#"case "$*" in
  "tool upgrade --all")
    exit "${FRESHEN_UV_UPGRADE_STATUS:-0}"
    ;;
  "cache prune")
    exit "${FRESHEN_UV_PRUNE_STATUS:-0}"
    ;;
esac"#,
    );
}

fn install_mise(tools: &StubTools) {
    tools.set_script(
        "mise",
        r#"case "$*" in
  "self-update -y")
    exit "${FRESHEN_MISE_SELF_UPDATE_STATUS:-0}"
    ;;
  "upgrade")
    exit "${FRESHEN_MISE_UPGRADE_STATUS:-0}"
    ;;
  "prune -y")
    exit "${FRESHEN_MISE_PRUNE_STATUS:-0}"
    ;;
esac"#,
    );
}

fn install_brew(tools: &StubTools) {
    tools.set_script(
        "brew",
        r#"case "$*" in
  "list --formula mise")
    exit "${FRESHEN_MISE_IN_BREW:-1}"
    ;;
esac"#,
    );
}

fn run(tools: &StubTools, steps: &str) -> Output {
    tools
        .command()
        .args(["--only", steps])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn uv_upgrades_tools_and_prunes_the_cache() {
    let tools = StubTools::new(&["uv"]);
    install_uv(&tools);

    let output = run(&tools, "uv");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.calls(), ["uv tool upgrade --all", "uv cache prune"]);
}

#[test]
fn a_failing_uv_upgrade_still_prunes_the_cache_and_fails_the_step() {
    let tools = StubTools::new(&["uv"]);
    install_uv(&tools);

    let output = tools
        .command()
        .env("FRESHEN_UV_UPGRADE_STATUS", "1")
        .args(["--only", "uv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.calls(), ["uv tool upgrade --all", "uv cache prune"]);
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn mise_self_updates_upgrades_and_prunes_when_not_installed_with_brew() {
    let tools = StubTools::new(&["brew", "mise"]);
    install_brew(&tools);
    install_mise(&tools);

    let output = run(&tools, "mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        [
            "brew list --formula mise",
            "mise self-update -y",
            "mise upgrade",
            "mise prune -y",
        ]
    );
}

#[test]
fn mise_installed_with_brew_is_not_self_updated() {
    let tools = StubTools::new(&["brew", "mise"]);
    install_brew(&tools);
    install_mise(&tools);

    let output = tools
        .command()
        .env("FRESHEN_MISE_IN_BREW", "0")
        .args(["--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        ["brew list --formula mise", "mise upgrade", "mise prune -y"]
    );
}

#[test]
fn mise_self_updates_when_brew_is_not_installed() {
    let tools = StubTools::new(&["mise"]);
    install_mise(&tools);

    let output = run(&tools, "mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        ["mise self-update -y", "mise upgrade", "mise prune -y"]
    );
}

#[test]
fn a_failing_mise_self_update_is_only_a_warning() {
    let tools = StubTools::new(&["mise"]);
    install_mise(&tools);

    let output = tools
        .command()
        .env("FRESHEN_MISE_SELF_UPDATE_STATUS", "1")
        .args(["--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        ["mise self-update -y", "mise upgrade", "mise prune -y"]
    );
    assert!(
        stdout(&output).contains("warning: mise self-update skipped/failed"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn a_failing_mise_upgrade_still_prunes_and_fails_the_step() {
    let tools = StubTools::new(&["mise"]);
    install_mise(&tools);

    let output = tools
        .command()
        .env("FRESHEN_MISE_UPGRADE_STATUS", "1")
        .args(["--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        ["mise self-update -y", "mise upgrade", "mise prune -y"]
    );
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn a_missing_uv_or_mise_is_skipped_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, "uv,mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 2 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}
