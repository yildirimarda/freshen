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
const HOMEBREW_STEPS: &str =
    "brew,brew-formulae,brew-casks,brew-autoremove,brew-cleanup,brew-reopen";

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
        stdout(&output).contains("0 ok, 0 planned, 6 skipped, 0 failed"),
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
        text.contains("5 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {text}"
    );
    assert!(text.contains("failed"), "summary: {text}");
}

fn app_tools(osascript_quits: bool) -> (StubTools, std::path::PathBuf, std::path::PathBuf) {
    let tools = StubTools::new(&["brew", "defaults", "ps", "osascript", "open", "kill"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --cask --greedy --quiet")
    printf 'firefox\n'
    ;;
  "info --cask --json=v2 firefox")
    printf '%s\n' "$FRESHEN_BREW_INFO_JSON"
    ;;
esac"#,
    );

    let app = tools.home().join("Applications/Firefox.app");
    std::fs::create_dir_all(&app)
        .unwrap_or_else(|error| panic!("failed to create test app: {error}"));

    tools.set_script("defaults", "printf 'com.example.firefox\\n'");
    let state = tools.home().join("ps.txt");
    std::fs::write(
        &state,
        format!("  123 {}/Contents/MacOS/firefox\n", app.display()),
    )
    .unwrap_or_else(|error| panic!("failed to write process table: {error}"));
    tools.set_script(
        "ps",
        "while IFS= read -r line; do printf '%s\\n' \"$line\"; done < \"$FRESHEN_PS_STATE\"",
    );
    tools.set_script(
        "osascript",
        if osascript_quits {
            ": > \"$FRESHEN_PS_STATE\""
        } else {
            ":"
        },
    );

    (tools, app, state)
}

fn run_apps(
    tools: &StubTools,
    state: &std::path::Path,
    extra_env: &[(&str, &str)],
    extra_args: &[&str],
) -> Output {
    let mut command = tools.command();
    command
        .env("FRESHEN_BREW_INFO_JSON", CASK_INFO)
        .env("FRESHEN_PS_STATE", state)
        .envs(extra_env.iter().copied())
        .args(["--only", HOMEBREW_STEPS, "--yes"])
        .args(extra_args);
    command
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn quits_running_cask_apps_before_the_upgrade_and_reopens_after_cleanup() {
    let (tools, app, state) = app_tools(true);

    let output = run_apps(&tools, &state, &[], &[]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let info = app.join("Contents/Info");
    assert_eq!(
        tools.calls(),
        [
            "brew update".to_owned(),
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!("defaults read {} CFBundleIdentifier", info.display()),
            "ps -axo command=".to_owned(),
            "osascript -e with timeout of 3 seconds -e tell application id \"com.example.firefox\" to quit -e end timeout".to_owned(),
            "ps -axo command=".to_owned(),
            "brew upgrade --cask --greedy".to_owned(),
            "brew autoremove".to_owned(),
            "brew cleanup --prune=all".to_owned(),
            "open -g -b com.example.firefox".to_owned(),
        ]
    );

    let text = stdout(&output);
    assert!(
        text.contains("Running apps that will be quit and reopened:"),
        "output: {text}"
    );
    assert!(text.contains("   - Firefox.app"), "output: {text}");
    assert!(text.contains("Quitting: Firefox.app"), "output: {text}");
    assert!(
        text.contains("Reopening apps that were quit"),
        "output: {text}"
    );
    assert!(
        text.contains("Opening: com.example.firefox"),
        "output: {text}"
    );
}

#[test]
fn never_quits_the_terminal_running_the_script() {
    let (tools, _app, state) = app_tools(true);

    let output = run_apps(
        &tools,
        &state,
        &[("__CFBundleIdentifier", "com.example.firefox")],
        &[],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.calls(),
        [
            "brew update".to_owned(),
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!(
                "defaults read {} CFBundleIdentifier",
                tools
                    .home()
                    .join("Applications/Firefox.app/Contents/Info")
                    .display()
            ),
            "brew upgrade --cask --greedy".to_owned(),
            "brew autoremove".to_owned(),
            "brew cleanup --prune=all".to_owned(),
        ]
    );

    let text = stdout(&output);
    assert!(
        text.contains("is the terminal running this script, not quitting it"),
        "output: {text}"
    );
    assert!(!text.contains("Quitting:"), "output: {text}");
}

#[test]
fn force_kill_terminates_an_app_that_does_not_quit_gracefully() {
    let (tools, _app, state) = app_tools(false);

    let output = run_apps(&tools, &state, &[("QUIT_TIMEOUT", "0")], &["--force-kill"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.calls();
    assert!(
        calls.contains(&"ps -axo pid=,command=".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"kill -9 123".to_owned()),
        "calls: {calls:?}"
    );
    assert_eq!(
        calls.last().map(String::as_str),
        Some("open -g -b com.example.firefox")
    );

    let text = stdout(&output);
    assert!(
        text.contains("did not quit, force-killing"),
        "output: {text}"
    );
}
