//! Tests that the commands freshen parses run in a neutral locale.
//!
//! Package managers and system tools translate their messages and the labels
//! of their tables. The stubs below answer with Turkish output unless the
//! locale they are started with is neutral, so a run that reads their output
//! correctly proves that the locale reached the child process.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::{brew_autoremove_steps, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A stub that only answers when it runs in the neutral locale.
const NEUTRAL_LOCALE_CHECK: &str = r#"if [ "$LC_ALL" != "C" ] || [ "$LANG" != "C" ]; then
  printf 'the locale is not neutral: LC_ALL=%s LANG=%s\n' "$LC_ALL" "$LANG" >&2
  exit 1
fi"#;

#[test]
fn the_mise_table_is_read_in_a_neutral_locale() {
    let tools = StubTools::new(&["mise"]);
    tools.set_script(
        "mise",
        &format!(
            r#"case "$*" in
  "outdated")
    {NEUTRAL_LOCALE_CHECK}
    printf 'Tool  Requested  Current  Latest\n'
    printf 'node  20         20.0.0   20.1.0\n'
    ;;
esac"#
        ),
    );

    let output = tools
        .command()
        .args(["--check", "--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    let report = stdout(&output);
    assert!(report.contains("node"), "report: {report}");
}

#[test]
fn the_uv_tool_list_is_read_in_a_neutral_locale() {
    let tools = StubTools::new(&["uv"]);
    tools.set_script(
        "uv",
        &format!(
            r#"case "$*" in
  "tool list --outdated")
    {NEUTRAL_LOCALE_CHECK}
    printf 'black v23.1.0 [latest: 26.10.1]\n'
    printf '%s\n' '- black'
    ;;
esac"#
        ),
    );

    let output = tools
        .command()
        .args(["--check", "--only", "uv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    let report = stdout(&output);
    assert!(report.contains("black"), "report: {report}");
    assert!(report.contains("26.10.1"), "report: {report}");
}

#[test]
fn the_homebrew_cleanup_listing_is_read_in_a_neutral_locale() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        &format!(
            r#"case "$*" in
  "autoremove --dry-run")
    {NEUTRAL_LOCALE_CHECK}
    printf '==> Would autoremove 1 unneeded formula:\n'
    printf '%s\n' 'old-dependency'
    ;;
  "autoremove")
    printf '%s\n' 'Uninstalling old-dependency (1.2MB)'
    ;;
esac"#
        ),
    );

    let output = tools
        .command()
        .args(["--only", "brew-autoremove", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        brew_autoremove_steps()
            .to_vec()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
}

#[test]
fn a_translated_table_does_not_add_a_tool_to_the_report() {
    let tools = StubTools::new(&["mise"]);
    tools.set_script(
        "mise",
        r#"case "$*" in
  "outdated")
    printf 'Araç  İstenen  Güncel  En Son\n'
    printf 'node  20         20.0.0   20.1.0\n'
    ;;
esac"#,
    );

    let output = tools
        .command()
        .args(["--check", "--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    let report = stdout(&output);
    assert!(report.contains("node"), "report: {report}");
    assert!(!report.contains("Araç"), "report: {report}");
}
