//! End-to-end checks for the `--json` and `--markdown` report output.
//!
//! The reports are produced from a real run against stub tools, so the
//! before/after versions come from the same `Step::check` calls users get.

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

fn install_brew(tools: &StubTools) {
    tools.set_script(
        "brew",
        r#"case "$*" in
  "list --versions")
    printf 'jq 1.7.1\n'
    ;;
  "outdated --formula")
    if [ -f "$FRESHEN_STUB_STATE/upgraded" ]; then
      exit 0
    fi
    printf 'jq (1.7.1) < 1.8.0\n'
    ;;
  "outdated --formula --quiet")
    printf 'jq\n'
    ;;
  "upgrade --formula")
    : > "$FRESHEN_STUB_STATE/upgraded"
    ;;
esac"#,
    );
}

fn run(tools: &StubTools, args: &[&str]) -> Output {
    tools
        .command()
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn json_report_contains_the_before_and_after_versions() {
    let tools = StubTools::new(&["brew"]);
    install_brew(&tools);

    let output = run(&tools, &["--only", "brew-formulae", "--json"]);

    assert!(
        output.status.success(),
        "stdout: {} stderr: {}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    let report: serde_json::Value = match serde_json::from_str(&text) {
        Ok(report) => report,
        Err(error) => panic!("invalid JSON report: {error}: {text}"),
    };

    assert_eq!(report["ok"], 1);
    assert_eq!(report["steps"][0]["id"], "brew-formulae");
    assert_eq!(report["steps"][0]["status"], "ok");
    assert_eq!(report["steps"][0]["changes"][0]["name"], "jq");
    assert_eq!(report["steps"][0]["changes"][0]["before"], "1.7.1");
    assert_eq!(report["steps"][0]["changes"][0]["after"], "1.8.0");
    assert_eq!(report["rollback"][0]["group"], "brew");
    assert_eq!(report["rollback"][0]["command"], "brew install jq@1.7.1");
    assert!(
        report["snapshot"]
            .as_str()
            .is_some_and(|path| path.ends_with(".txt")),
        "report: {report}"
    );
    assert_eq!(
        tools.step_calls(),
        [
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew upgrade --formula",
            "brew outdated --formula",
        ]
    );
}

#[test]
fn markdown_report_contains_a_table_with_the_before_and_after_versions() {
    let tools = StubTools::new(&["brew"]);
    install_brew(&tools);

    let output = run(&tools, &["--only", "brew-formulae", "--markdown"]);

    assert!(
        output.status.success(),
        "stdout: {} stderr: {}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.starts_with("# freshen report\n"), "report: {text}");
    assert!(
        text.contains("| Step | Status | Duration | Changes |"),
        "report: {text}"
    );
    assert!(
        text.contains("Homebrew formulae (`brew-formulae`)"),
        "report: {text}"
    );
    assert!(text.contains("jq 1.7.1 -> 1.8.0"), "report: {text}");
    assert!(text.contains("## Rollback hints"), "report: {text}");
    assert!(text.contains("brew install jq@1.7.1"), "report: {text}");
}

#[test]
fn the_plain_summary_lists_the_same_version_changes() {
    let tools = StubTools::new(&["brew"]);
    install_brew(&tools);

    let output = run(&tools, &["--only", "brew-formulae"]);

    assert!(
        output.status.success(),
        "stdout: {} stderr: {}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("Summary: 1 ok"), "summary: {text}");
    assert!(text.contains("jq 1.7.1 -> 1.8.0"), "summary: {text}");
    assert!(text.contains("Snapshot:"), "summary: {text}");
    assert!(
        text.contains("Rollback hints (previous versions from the snapshot):"),
        "summary: {text}"
    );
    assert!(text.contains("brew install jq@1.7.1"), "summary: {text}");
}

#[test]
fn check_mode_supports_the_json_report() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula")
    printf 'jq (1.7.1) < 1.8.0\n'
    ;;
esac"#,
    );

    let output = run(&tools, &["--check", "--only", "brew-formulae", "--json"]);

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let report: serde_json::Value = match serde_json::from_str(&text) {
        Ok(report) => report,
        Err(error) => panic!("invalid JSON report: {error}: {text}"),
    };

    assert_eq!(report["outdated"], 1);
    assert_eq!(report["pending"], 1);
    assert_eq!(report["steps"][0]["items"][0]["name"], "jq");
    assert_eq!(report["steps"][0]["items"][0]["current"], "1.7.1");
    assert_eq!(report["steps"][0]["items"][0]["latest"], "1.8.0");
}

#[test]
fn check_mode_supports_the_markdown_report() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula")
    printf 'jq (1.7.1) < 1.8.0\n'
    ;;
esac"#,
    );

    let output = run(
        &tools,
        &["--check", "--only", "brew-formulae", "--markdown"],
    );

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.starts_with("# freshen check\n"), "report: {text}");
    assert!(text.contains("jq 1.7.1 -> 1.8.0"), "report: {text}");
}

#[test]
fn json_and_markdown_cannot_be_combined() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, &["--json", "--markdown"]);

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("cannot be used with"),
        "stderr: {}",
        stderr(&output)
    );
}
