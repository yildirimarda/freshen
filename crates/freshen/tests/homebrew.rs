//! Parity tests for the Homebrew steps.
//!
//! Each test runs the built binary against a stub `brew` that records its
//! invocations and replays recorded output, so the expected command transcript
//! is exactly what `update-all.sh` runs.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::StubTools;

const CASK_INFO: &str = include_str!("../../freshen-lib/tests/fixtures/brew-info-casks.json");
const HOMEBREW_STEPS: &str = "brew,brew-formulae,brew-casks,brew-autoremove,brew-cleanup";

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools, args: &[&str]) -> Output {
    tools
        .command()
        .env("FRESHEN_BREW_INFO_JSON", CASK_INFO)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn command_transcript_matches_the_zsh_homebrew_block() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --cask --greedy --quiet")
    printf 'firefox\ngoogle-chrome\n'
    ;;
  "info --cask --json=v2 firefox google-chrome")
    printf '%s\n' "$FRESHEN_BREW_INFO_JSON"
    ;;
esac"#,
    );
    std::fs::create_dir_all(tools.home().join("Applications/Firefox.app"))
        .unwrap_or_else(|error| panic!("failed to create test app: {error}"));

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        [
            "brew update",
            "brew upgrade --formula",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox google-chrome",
            "brew upgrade --cask --greedy",
            "brew autoremove",
            "brew cleanup --prune=all",
        ]
    );

    let text = stdout(&output);
    assert!(
        text.contains("Casks to upgrade: firefox google-chrome"),
        "output: {text}"
    );
    assert!(
        text.contains("Cask apps: Firefox.app, Google Chrome.app"),
        "output: {text}"
    );
    assert!(
        text.contains(&format!(
            "Installed cask app: {}",
            tools.home().join("Applications/Firefox.app").display()
        )),
        "output: {text}"
    );
}

#[test]
fn no_outdated_casks_skips_the_cask_upgrade() {
    let tools = StubTools::new(&["brew"]);

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        [
            "brew update",
            "brew upgrade --formula",
            "brew outdated --cask --greedy --quiet",
            "brew autoremove",
            "brew cleanup --prune=all",
        ]
    );
    assert!(
        stdout(&output).contains("No outdated casks."),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn a_missing_brew_skips_every_homebrew_step_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 5 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn invalid_cask_metadata_does_not_stop_the_cask_upgrade() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --cask --greedy --quiet")
    printf 'firefox\n'
    ;;
  "info --cask --json=v2 firefox")
    printf 'not json\n'
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        [
            "brew update",
            "brew upgrade --formula",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox",
            "brew upgrade --cask --greedy",
            "brew autoremove",
            "brew cleanup --prune=all",
        ]
    );
    assert!(
        stderr(&output).contains("could not read cask metadata"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn a_failing_formula_upgrade_does_not_stop_the_cask_steps() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "upgrade --formula")
    exit 1
    ;;
  "outdated --cask --greedy --quiet")
    printf 'firefox\n'
    ;;
  "info --cask --json=v2 firefox")
    printf '%s\n' "$FRESHEN_BREW_INFO_JSON"
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        [
            "brew update",
            "brew upgrade --formula",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox",
            "brew upgrade --cask --greedy",
            "brew autoremove",
            "brew cleanup --prune=all",
        ]
    );
    let text = stdout(&output);
    assert!(
        text.contains("4 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {text}"
    );
    assert!(text.contains("failed"), "summary: {text}");
}
