//! Tests for destructive cleanup gating and logging.
//!
//! Cleanup commands must never run unless `--cleanup` is given, and when they
//! do run every removed item and the reclaimed space must reach the output.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::StubTools;

const STEPS: &str = "brew-autoremove,brew-cleanup,uv,mise,docker";

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools, args: &[&str]) -> Output {
    tools
        .command()
        .env_remove("ZELLIJ")
        .args(["--only", STEPS])
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn install_brew(tools: &StubTools) {
    tools.set_script(
        "brew",
        r#"case "$*" in
  "autoremove --dry-run")
    printf '==> Would autoremove 1 unneeded formula:\nfoo\n'
    ;;
  "cleanup --dry-run --prune=all")
    printf 'Would remove: /tmp/foo (2.5MB)\n'
    ;;
  "autoremove")
    printf 'Uninstalling foo (1.2MB)\n'
    printf '==> This operation has freed approximately 1.2MB of disk space.\n'
    ;;
  "cleanup --prune=all")
    printf 'Removing: /tmp/foo (2.5MB)\n'
    printf '==> This operation has freed approximately 2.5MB of disk space.\n'
    ;;
  "list --formula mise")
    exit 0
    ;;
esac"#,
    );
}

fn install_uv(tools: &StubTools) {
    tools.set_script(
        "uv",
        r#"case "$*" in
  "cache prune")
    printf 'Removed 3 files (1.5 MiB)\n'
    ;;
esac"#,
    );
}

fn install_mise(tools: &StubTools) {
    tools.set_script(
        "mise",
        r#"case "$*" in
  "prune -y")
    printf 'pruned node@20.0.0\n'
    ;;
esac"#,
    );
}

fn install_colima(tools: &StubTools) {
    tools.set_script(
        "colima",
        r#"case "$*" in
  "status")
    exit 0
    ;;
esac"#,
    );
}

fn install_docker(tools: &StubTools) {
    tools.set_script(
        "docker",
        r#"case "$*" in
  "compose ls --format json")
    printf '[]'
    ;;
  "image prune -f")
    printf 'Deleted Images:\n'
    printf 'untagged: example:latest\n'
    printf 'Total reclaimed space: 3.5GB\n'
    ;;
esac"#,
    );
}

fn tools() -> StubTools {
    let tools = StubTools::new(&["brew", "colima", "docker", "mise", "uv"]);
    install_brew(&tools);
    install_uv(&tools);
    install_mise(&tools);
    install_colima(&tools);
    install_docker(&tools);
    tools
}

#[test]
fn destructive_cleanup_never_runs_without_the_flag() {
    let tools = tools();

    let output = run(&tools, &[]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    for command in [
        "brew autoremove",
        "brew cleanup --prune=all",
        "uv cache prune",
        "mise prune -y",
        "docker image prune -f",
    ] {
        assert!(
            !calls.iter().any(|call| call == command),
            "{command} ran without --cleanup: {calls:?}"
        );
    }

    let text = stdout(&output);
    for label in [
        "Homebrew autoremove",
        "Homebrew cleanup",
        "uv: cache prune",
        "mise: prune (remove unused old versions)",
        "Docker: prune dangling images",
    ] {
        assert!(
            text.contains(&format!(
                "{label}: skipped destructive cleanup (pass --cleanup to enable)"
            )),
            "missing skip message for {label}: {text}"
        );
    }
}

#[test]
fn cleanup_logs_every_removed_item_and_the_reclaimed_space() {
    let tools = tools();

    let output = run(&tools, &["--cleanup"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    for command in [
        "brew autoremove",
        "brew cleanup --prune=all",
        "uv cache prune",
        "mise prune -y",
        "docker image prune -f",
    ] {
        assert!(
            calls.iter().any(|call| call == command),
            "{command} did not run with --cleanup: {calls:?}"
        );
    }

    let text = stdout(&output);
    for expected in [
        "Uninstalling foo (1.2MB)",
        "Homebrew autoremove: reclaimed space: 1.2MB",
        "Removing: /tmp/foo (2.5MB)",
        "Homebrew cleanup: reclaimed space: 2.5MB",
        "Removed 3 files (1.5 MiB)",
        "uv: cache prune: reclaimed space: 1.5 MiB",
        "pruned node@20.0.0",
        "mise: prune (remove unused old versions): reclaimed space: not reported by the tool",
        "Deleted Images:",
        "Total reclaimed space: 3.5GB",
        "Docker: prune dangling images: reclaimed space: 3.5GB",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
}

#[test]
fn empty_dry_run_listings_skip_the_mutating_commands() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "autoremove --dry-run"|"cleanup --dry-run --prune=all")
    ;;
  "autoremove"|"cleanup --prune=all")
    printf 'MUTATION RAN\n'
    ;;
esac"#,
    );

    let output = run(&tools, &["--cleanup"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.iter().any(|call| call == "brew autoremove --dry-run"),
        "autoremove was not guarded by a dry run: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|call| call == "brew cleanup --dry-run --prune=all"),
        "cleanup was not guarded by a dry run: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == "brew autoremove"),
        "autoremove ran with nothing to remove: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == "brew cleanup --prune=all"),
        "cleanup ran with nothing to remove: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("Homebrew autoremove: nothing to remove"),
        "output: {text}"
    );
    assert!(
        text.contains("Homebrew cleanup: nothing to remove"),
        "output: {text}"
    );
    assert!(!text.contains("MUTATION RAN"), "output: {text}");
}
