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
            "mise where node",
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
        [
            "brew list --formula mise",
            "mise upgrade",
            "mise prune -y",
            "mise where node",
        ]
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
        [
            "mise self-update -y",
            "mise upgrade",
            "mise prune -y",
            "mise where node"
        ]
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
        [
            "mise self-update -y",
            "mise upgrade",
            "mise prune -y",
            "mise where node"
        ]
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
        [
            "mise self-update -y",
            "mise upgrade",
            "mise prune -y",
            "mise where node"
        ]
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

fn install_stale_node(tools: &StubTools) {
    tools.set_script(
        "mise",
        r#"case "$*" in
  "where node")
    printf '/home/user/.local/share/mise/installs/node/22.1.0\n'
    ;;
esac"#,
    );
    tools.set_script(
        "ps",
        r#"case "$*" in
  "-axo pid=,ppid=,command=")
    printf '%s\n' \
      '  100     1 /home/user/.local/share/mise/installs/node/20.0.0/bin/node /path/old server.js' \
      '  101     1 /home/user/.local/share/mise/installs/node/22.1.0/bin/node /path/current.js' \
      '  102     1 /usr/bin/node /path/system.js'
    ;;
  "-o comm= -p 1")
    printf '/Applications/LM Studio.app/Contents/MacOS/LM Studio\n'
    ;;
esac"#,
    );
}

#[test]
fn warns_about_processes_still_running_an_old_node_version() {
    let tools = StubTools::new(&["mise", "ps"]);
    install_stale_node(&tools);

    let output = run(&tools, "mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains(
            "warning: Processes still running on an old node version (restart the app that started them so mise can prune it):"
        ),
        "output: {text}"
    );
    assert!(
        text.contains(
            "   - pid 100 (started by: LM Studio): /home/user/.local/share/mise/installs/node/20.0.0/bin/node /path/old server.js"
        ),
        "output: {text}"
    );
    assert!(
        !text.contains("pid 101"),
        "the active node version must not be reported: {text}"
    );
    assert_eq!(
        tools.calls(),
        [
            "mise self-update -y",
            "mise upgrade",
            "mise prune -y",
            "mise where node",
            "ps -axo pid=,ppid=,command=",
            "ps -o comm= -p 1",
        ]
    );
}

#[test]
fn no_stale_node_warning_when_every_process_uses_the_active_version() {
    let tools = StubTools::new(&["mise", "ps"]);
    tools.set_script(
        "mise",
        r#"case "$*" in
  "where node")
    printf '/home/user/.local/share/mise/installs/node/22.1.0\n'
    ;;
esac"#,
    );
    tools.set_script(
        "ps",
        r#"case "$*" in
  "-axo pid=,ppid=,command=")
    printf '  101 1 /home/user/.local/share/mise/installs/node/22.1.0/bin/node /path/current.js\n'
    ;;
esac"#,
    );

    let output = run(&tools, "mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains("Processes still running on an old node version"),
        "output: {}",
        stdout(&output)
    );
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call.starts_with("ps -o comm=")),
        "calls: {:?}",
        tools.calls()
    );
}
