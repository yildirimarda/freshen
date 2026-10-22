//! Parity tests for the Homebrew steps.
//!
//! Each test runs the built binary against a stub `brew` that records its
//! invocations and replays recorded output, so the expected command transcript
//! is exactly what `update-all.sh` runs.

#![cfg(all(feature = "test-harness", not(windows)))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

const CASK_INFO: &str = include_str!("../../freshen-lib/tests/fixtures/brew-info-casks.json");
const CASK_INFO_WITH_ADMINISTRATOR: &str =
    include_str!("../../freshen-lib/tests/fixtures/brew-info-casks-administrator.json");
const CASK_INFO_WITH_GREEDY: &str =
    include_str!("../../freshen-lib/tests/fixtures/brew-info-casks-greedy.json");
/// A menu-bar helper: no `.app` artifact, only the bundle id in the
/// `uninstall` stanza names the app to quit.
const CASK_INFO_WITHOUT_APP_ARTIFACTS: &str = r#"{
    "casks": [
        {
            "token": "menu-bar",
            "artifacts": [
                {
                    "uninstall": [
                        {
                            "quit": "com.example.menubar",
                            "launchctl": "com.example.helper"
                        }
                    ]
                }
            ]
        }
    ]
}"#;
const HOMEBREW_STEPS: &str =
    "brew,brew-formulae,brew-services,brew-casks,brew-autoremove,brew-cleanup,brew-reopen";

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
        .arg("--cleanup")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn homebrew_responses(transcript: &[StubOutput]) -> Vec<StubOutput> {
    let mut responses = vec![StubOutput::new(); 3];
    responses.extend_from_slice(transcript);
    responses
}

#[test]
fn command_transcript_matches_the_zsh_homebrew_block() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &homebrew_responses(&[
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("jq\n"),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("firefox\ngoogle-chrome\n"),
            StubOutput::stdout(CASK_INFO),
            StubOutput::new(),
            StubOutput::stdout("==> Would autoremove 1 unneeded formula:\nold-dependency\n"),
            StubOutput::new(),
            StubOutput::stdout("Would remove: /tmp/old (1.2MB)\n"),
            StubOutput::new(),
        ]),
    );
    std::fs::create_dir_all(tools.home().join("Applications/Firefox.app"))
        .unwrap_or_else(|error| panic!("failed to create test app: {error}"));

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew list --pinned --formula",
            "brew services list",
            "brew upgrade --formula",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox google-chrome",
            "brew upgrade --cask",
            "brew autoremove --dry-run",
            "brew autoremove",
            "brew cleanup --dry-run --prune=all",
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
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew autoremove --dry-run",
            "brew cleanup --dry-run --prune=all",
        ]
    );
    assert!(
        stdout(&output).contains("No outdated formulae."),
        "output: {}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("No outdated casks."),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn restarts_running_services_whose_formulae_were_upgraded_and_reports_them() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula")
    printf 'postgresql@16 (15.2) < 15.3\nredis (7.0) < 7.2\n'
    ;;
  "outdated --formula --quiet")
    printf 'postgresql@16\nredis\n'
    ;;
  "services list")
    printf 'Name              Status  User File\n'
    printf 'postgresql@16     started user ~/Library/LaunchAgents/homebrew.mxcl.postgresql@16.plist\n'
    printf 'redis             stopped user ~/Library/LaunchAgents/homebrew.mxcl.redis.plist\n'
    printf 'mysql             started user ~/Library/LaunchAgents/homebrew.mxcl.mysql.plist\n'
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", "brew,brew-formulae,brew-services"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"brew services list".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"brew services restart postgresql@16".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls.contains(&"brew services restart redis".to_owned()),
        "a stopped service was started: {calls:?}"
    );
    assert!(
        !calls.contains(&"brew services restart mysql".to_owned()),
        "a service whose formula was not upgraded was restarted: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("Restarting Homebrew services: postgresql@16"),
        "output: {text}"
    );
    assert!(
        text.contains("Restarted Homebrew services: postgresql@16"),
        "output: {text}"
    );
}

#[test]
fn a_failing_services_list_does_not_stop_the_formula_upgrade() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula --quiet")
    printf 'jq\n'
    ;;
  "services list")
    exit 1
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", "brew,brew-formulae,brew-services"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"brew upgrade --formula".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("brew services restart")),
        "calls: {calls:?}"
    );
    assert!(
        stderr(&output).contains("could not list Homebrew services"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn a_failing_restart_is_reported_and_does_not_stop_the_other_services() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula --quiet")
    printf 'postgresql@16\nredis\n'
    ;;
  "services list")
    printf 'Name Status\npostgresql@16 started\nredis started\n'
    ;;
  "services restart postgresql@16")
    exit 1
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", "brew,brew-formulae,brew-services"]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"brew services restart postgresql@16".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"brew services restart redis".to_owned()),
        "calls: {calls:?}"
    );
    let text = stdout(&output);
    assert!(
        text.contains("Restarted Homebrew services: redis"),
        "output: {text}"
    );
}

#[test]
fn a_missing_brew_skips_every_homebrew_step_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 7 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn invalid_cask_metadata_does_not_stop_the_cask_upgrade() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &homebrew_responses(&[
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("jq\n"),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("firefox\n"),
            StubOutput::stdout("not json\n"),
            StubOutput::new(),
            StubOutput::stdout("==> Would autoremove 1 unneeded formula:\nold-dependency\n"),
            StubOutput::new(),
            StubOutput::stdout("Would remove: /tmp/old (1.2MB)\n"),
            StubOutput::new(),
        ]),
    );

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew list --pinned --formula",
            "brew services list",
            "brew upgrade --formula",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox",
            "brew upgrade --cask --greedy",
            "brew autoremove --dry-run",
            "brew autoremove",
            "brew cleanup --dry-run --prune=all",
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
    tools.set_responses(
        "brew",
        &homebrew_responses(&[
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("jq\n"),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new().with_status(1),
            StubOutput::new(),
            StubOutput::stdout("firefox\n"),
            StubOutput::stdout(CASK_INFO),
            StubOutput::new(),
            StubOutput::stdout("==> Would autoremove 1 unneeded formula:\nold-dependency\n"),
            StubOutput::new(),
            StubOutput::stdout("Would remove: /tmp/old (1.2MB)\n"),
            StubOutput::new(),
        ]),
    );

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew list --pinned --formula",
            "brew services list",
            "brew upgrade --formula",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox",
            "brew upgrade --cask",
            "brew autoremove --dry-run",
            "brew autoremove",
            "brew cleanup --dry-run --prune=all",
            "brew cleanup --prune=all",
        ]
    );
    let text = stdout(&output);
    assert!(
        text.contains("6 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {text}"
    );
    assert!(text.contains("failed"), "summary: {text}");
}

#[test]
fn a_cask_that_needs_an_administrator_password_is_skipped_without_a_terminal() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &homebrew_responses(&[
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("jq\n"),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("firefox\n1password\n"),
            StubOutput::stdout(CASK_INFO_WITH_ADMINISTRATOR),
            StubOutput::new(),
            StubOutput::stdout("==> Would autoremove 1 unneeded formula:\nold-dependency\n"),
            StubOutput::new(),
            StubOutput::stdout("Would remove: /tmp/old (1.2MB)\n"),
            StubOutput::new(),
        ]),
    );

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "brew update",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew list --pinned --formula",
            "brew services list",
            "brew upgrade --formula",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox 1password",
            "brew upgrade --cask firefox",
            "brew autoremove --dry-run",
            "brew autoremove",
            "brew cleanup --dry-run --prune=all",
            "brew cleanup --prune=all",
        ]
    );

    let text = stdout(&output);
    assert!(text.contains("skipping cask `1password`"), "output: {text}");
    assert!(
        text.contains("needs an administrator password"),
        "output: {text}"
    );
}

#[test]
fn no_cask_is_upgraded_when_every_cask_needs_an_administrator_password() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &homebrew_responses(&[
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("jq\n"),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout("1password\n"),
            StubOutput::stdout(CASK_INFO_WITH_ADMINISTRATOR),
            StubOutput::stdout("==> Would autoremove 1 unneeded formula:\nold-dependency\n"),
            StubOutput::new(),
            StubOutput::stdout("Would remove: /tmp/old (1.2MB)\n"),
            StubOutput::new(),
        ]),
    );

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("brew upgrade --cask")),
        "calls: {:?}",
        tools.step_calls()
    );

    let text = stdout(&output);
    assert!(text.contains("skipping cask `1password`"), "output: {text}");
    assert!(text.contains("No casks upgraded"), "output: {text}");
}

#[cfg(unix)]
fn app_tools(osascript_quits: bool) -> (StubTools, std::path::PathBuf, std::path::PathBuf) {
    let tools = StubTools::new(&["brew", "defaults", "ps", "osascript", "open", "kill"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula --quiet")
    printf 'jq\n'
    ;;
  "outdated --cask --greedy --quiet")
    printf 'firefox\n'
    ;;
  "info --cask --json=v2 firefox")
    printf '%s\n' "$FRESHEN_BREW_INFO_JSON"
    ;;
  "autoremove --dry-run")
    printf '==> Would autoremove 1 unneeded formula:\nold-dependency\n'
    ;;
  "cleanup --dry-run --prune=all")
    printf 'Would remove: /tmp/old (1.2MB)\n'
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
        &osascript_script(if osascript_quits {
            ": > \"$FRESHEN_PS_STATE\""
        } else {
            ":"
        }),
    );

    (tools, app, state)
}

/// The `osascript` stub that answers the questions freshen asks about an app.
///
/// The hidden state and the place of the window are answered from the
/// environment, so a test can describe how the app looked before the quit.
/// The `quit` body stands for what the stub does when the app is quit.
#[cfg(unix)]
fn osascript_script(quit_body: &str) -> String {
    r#"case "$*" in
  *"AXHidden"*)
    printf '%s\n' "${FRESHEN_APP_HIDDEN:-false}"
    ;;
  *"position of window 1"*)
    if [ -n "${FRESHEN_APP_WINDOW:-}" ]; then printf '%s\n' "$FRESHEN_APP_WINDOW"; fi
    ;;
  *"to quit"*)
    QUIT_BODY
    ;;
esac"#
        .replace("QUIT_BODY", quit_body)
}

/// Makes the `osascript` stub answer like macOS does when the Automation
/// permission was never granted.
#[cfg(unix)]
fn deny_automation(tools: &StubTools) {
    tools.set_script(
        "osascript",
        r#"printf 'execution error: Not authorized to send Apple events to "com.example.firefox". (-1743)\n' >&2
exit 1"#,
    );
}

#[cfg(unix)]
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
        .args(["--only", HOMEBREW_STEPS, "--yes", "--cleanup"])
        .args(extra_args);
    command
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[cfg(unix)]
fn run_apps_with_stdin(tools: &StubTools, state: &std::path::Path, stdin: &str) -> Output {
    let mut command = tools.command();
    command
        .env("FRESHEN_BREW_INFO_JSON", CASK_INFO)
        .env("FRESHEN_PS_STATE", state)
        .env_remove("ZELLIJ")
        .args(["--only", HOMEBREW_STEPS, "--cleanup"]);
    tools.run_with_stdin(command, stdin)
}

#[cfg(unix)]
#[test]
fn quits_running_cask_apps_before_the_upgrade_and_reopens_after_cleanup() {
    let (tools, app, state) = app_tools(true);

    let output = run_apps(&tools, &state, &[], &[]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let info = app.join("Contents/Info");
    assert_eq!(
        tools.step_calls(),
        [
            "brew update".to_owned(),
            "brew outdated --formula".to_owned(),
            "brew outdated --formula --quiet".to_owned(),
            "brew list --pinned --formula".to_owned(),
            "brew services list".to_owned(),
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!("defaults read {} CFBundleIdentifier", info.display()),
            "ps -axo command=".to_owned(),
            format!("defaults read {} LSUIElement", info.display()),
            "osascript -e tell application \"System Events\" -e set appProcess to first process whose bundle identifier is \"com.example.firefox\" -e get value of attribute \"AXHidden\" of appProcess -e end tell".to_owned(),
            "osascript -e tell application \"System Events\" -e set appProcess to first process whose bundle identifier is \"com.example.firefox\" -e tell appProcess -e get {position of window 1, size of window 1} -e end tell -e end tell".to_owned(),
            "osascript -e with timeout of 3 seconds -e tell application id \"com.example.firefox\" to quit -e end timeout".to_owned(),
            "ps -axo command=".to_owned(),
            "brew upgrade --cask".to_owned(),
            "brew autoremove --dry-run".to_owned(),
            "brew autoremove".to_owned(),
            "brew cleanup --dry-run --prune=all".to_owned(),
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

#[cfg(unix)]
#[test]
fn quits_and_reopens_a_cask_app_known_only_by_its_uninstall_bundle_id() {
    let tools = StubTools::new(&["brew", "osascript", "open", "kill"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula --quiet")
    printf 'jq\n'
    ;;
  "outdated --cask --greedy --quiet")
    printf 'menu-bar\n'
    ;;
  "info --cask --json=v2 menu-bar")
    printf '%s\n' "$FRESHEN_BREW_INFO_JSON"
    ;;
esac"#,
    );

    let running = tools.home().join("running-bundle.txt");
    std::fs::write(&running, "true\n")
        .unwrap_or_else(|error| panic!("failed to write running state: {error}"));
    tools.set_script(
        "osascript",
        r#"case "$*" in
  *"is running"*)
    IFS= read -r running < "$FRESHEN_RUNNING_BUNDLE"
    printf '%s\n' "$running"
    ;;
  *"to quit"*)
    : > "$FRESHEN_RUNNING_BUNDLE"
    ;;
esac"#,
    );

    let output = tools
        .command()
        .env("FRESHEN_BREW_INFO_JSON", CASK_INFO_WITHOUT_APP_ARTIFACTS)
        .env("FRESHEN_RUNNING_BUNDLE", &running)
        .args(["--only", HOMEBREW_STEPS, "--yes", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let running_query = "osascript -e application id \"com.example.menubar\" is running".to_owned();
    assert!(
        calls.contains(&running_query),
        "the bundle id was not checked for running: {calls:?}"
    );
    assert!(
        calls.contains(&"osascript -e with timeout of 3 seconds -e tell application id \"com.example.menubar\" to quit -e end timeout".to_owned()),
        "the bundle id was not quit: {calls:?}"
    );
    assert!(
        calls.contains(&"open -g -b com.example.menubar".to_owned()),
        "the bundle id was not reopened: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("Quitting: com.example.menubar"),
        "output: {text}"
    );
    assert!(
        text.contains("Opening: com.example.menubar"),
        "output: {text}"
    );
}

#[cfg(unix)]
#[test]
fn yes_at_the_prompt_quits_apps_and_upgrades_casks() {
    let (tools, app, state) = app_tools(true);

    let output = run_apps_with_stdin(&tools, &state, "yes\n");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let info = app.join("Contents/Info");
    assert_eq!(
        tools.step_calls(),
        [
            "brew update".to_owned(),
            "brew outdated --formula".to_owned(),
            "brew outdated --formula --quiet".to_owned(),
            "brew list --pinned --formula".to_owned(),
            "brew services list".to_owned(),
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!("defaults read {} CFBundleIdentifier", info.display()),
            "ps -axo command=".to_owned(),
            format!("defaults read {} LSUIElement", info.display()),
            "osascript -e tell application \"System Events\" -e set appProcess to first process whose bundle identifier is \"com.example.firefox\" -e get value of attribute \"AXHidden\" of appProcess -e end tell".to_owned(),
            "osascript -e tell application \"System Events\" -e set appProcess to first process whose bundle identifier is \"com.example.firefox\" -e tell appProcess -e get {position of window 1, size of window 1} -e end tell -e end tell".to_owned(),
            "osascript -e with timeout of 3 seconds -e tell application id \"com.example.firefox\" to quit -e end timeout".to_owned(),
            "ps -axo command=".to_owned(),
            "brew upgrade --cask".to_owned(),
            "brew autoremove --dry-run".to_owned(),
            "brew autoremove".to_owned(),
            "brew cleanup --dry-run --prune=all".to_owned(),
            "brew cleanup --prune=all".to_owned(),
            "open -g -b com.example.firefox".to_owned(),
        ]
    );

    let text = stdout(&output);
    assert!(
        text.contains("Quit these apps and upgrade casks? [Y]es / [s]kip quitting / [n]o:"),
        "prompt was not shown: {text}"
    );
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

#[cfg(unix)]
#[test]
fn an_app_that_was_hidden_is_reopened_hidden() {
    let (tools, _app, state) = app_tools(true);

    let output = run_apps(&tools, &state, &[("FRESHEN_APP_HIDDEN", "true")], &[]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"open -g -j -b com.example.firefox".to_owned()),
        "the app was not reopened hidden: {calls:?}"
    );
    assert!(
        !calls.contains(&"open -g -b com.example.firefox".to_owned()),
        "the app was reopened visible: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("Opening: com.example.firefox (hidden)"),
        "output: {text}"
    );
}

#[cfg(unix)]
#[test]
fn the_window_of_an_app_is_put_back_where_it_was() {
    let (tools, _app, state) = app_tools(true);

    let output = run_apps(
        &tools,
        &state,
        &[("FRESHEN_APP_WINDOW", "{{10, 20}, {1440, 900}}")],
        &[],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"open -g -b com.example.firefox".to_owned()),
        "the app was not reopened: {calls:?}"
    );
    assert!(
        calls.contains(
            &"osascript -e tell application \"System Events\" -e set appProcess to first process whose bundle identifier is \"com.example.firefox\" -e tell appProcess -e set position of window 1 to {10, 20} -e set size of window 1 to {1440, 900} -e end tell -e end tell".to_owned()
        ),
        "the window was not put back: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .position(|call| call.contains("set position of window 1"))
            > calls.iter().position(|call| call.contains("open -g -b")),
        "the window was put back before the app was opened: {calls:?}"
    );
}

#[cfg(unix)]
#[test]
fn an_app_that_was_not_hidden_is_reopened_visible() {
    let (tools, _app, state) = app_tools(true);

    let output = run_apps(&tools, &state, &[("FRESHEN_APP_HIDDEN", "false")], &[]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"open -g -b com.example.firefox".to_owned()),
        "the app was not reopened: {calls:?}"
    );
    assert!(
        !calls.contains(&"open -g -j -b com.example.firefox".to_owned()),
        "the app was reopened hidden: {calls:?}"
    );
}

#[cfg(unix)]
#[test]
fn skip_quitting_at_the_prompt_upgrades_casks_without_quitting_apps() {
    let (tools, app, state) = app_tools(true);

    let output = run_apps_with_stdin(&tools, &state, "s\n");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let info = app.join("Contents/Info");
    assert_eq!(
        tools.step_calls(),
        [
            "brew update".to_owned(),
            "brew outdated --formula".to_owned(),
            "brew outdated --formula --quiet".to_owned(),
            "brew list --pinned --formula".to_owned(),
            "brew services list".to_owned(),
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!("defaults read {} CFBundleIdentifier", info.display()),
            "ps -axo command=".to_owned(),
            format!("defaults read {} LSUIElement", info.display()),
            "brew upgrade --cask".to_owned(),
            "brew autoremove --dry-run".to_owned(),
            "brew autoremove".to_owned(),
            "brew cleanup --dry-run --prune=all".to_owned(),
            "brew cleanup --prune=all".to_owned(),
        ]
    );

    let text = stdout(&output);
    assert!(
        text.contains("Quit these apps and upgrade casks? [Y]es / [s]kip quitting / [n]o:"),
        "prompt was not shown: {text}"
    );
    assert!(!text.contains("Quitting:"), "apps were quit: {text}");
    assert!(
        !text.contains("Reopening apps that were quit"),
        "apps were reopened: {text}"
    );
    assert!(!text.contains("Opening:"), "apps were reopened: {text}");
}

#[cfg(unix)]
#[test]
fn skip_casks_at_the_prompt_skips_the_cask_upgrade() {
    let (tools, app, state) = app_tools(true);

    let output = run_apps_with_stdin(&tools, &state, "n\n");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let info = app.join("Contents/Info");
    assert_eq!(
        tools.step_calls(),
        [
            "brew update".to_owned(),
            "brew outdated --formula".to_owned(),
            "brew outdated --formula --quiet".to_owned(),
            "brew list --pinned --formula".to_owned(),
            "brew services list".to_owned(),
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!("defaults read {} CFBundleIdentifier", info.display()),
            "ps -axo command=".to_owned(),
            format!("defaults read {} LSUIElement", info.display()),
            "brew autoremove --dry-run".to_owned(),
            "brew autoremove".to_owned(),
            "brew cleanup --dry-run --prune=all".to_owned(),
            "brew cleanup --prune=all".to_owned(),
        ]
    );

    let text = stdout(&output);
    assert!(
        text.contains("Quit these apps and upgrade casks? [Y]es / [s]kip quitting / [n]o:"),
        "prompt was not shown: {text}"
    );
    assert!(
        text.contains("warning: Cask upgrade skipped by user."),
        "output: {text}"
    );
    assert!(!text.contains("Quitting:"), "apps were quit: {text}");
    assert!(
        !text.contains("Reopening apps that were quit"),
        "apps were reopened: {text}"
    );
}

#[cfg(unix)]
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
        tools.step_calls(),
        [
            "brew update".to_owned(),
            "brew outdated --formula".to_owned(),
            "brew outdated --formula --quiet".to_owned(),
            "brew list --pinned --formula".to_owned(),
            "brew services list".to_owned(),
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!(
                "defaults read {} CFBundleIdentifier",
                tools
                    .home()
                    .join("Applications/Firefox.app/Contents/Info")
                    .display()
            ),
            "brew upgrade --cask".to_owned(),
            "brew autoremove --dry-run".to_owned(),
            "brew autoremove".to_owned(),
            "brew cleanup --dry-run --prune=all".to_owned(),
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

#[cfg(unix)]
#[test]
fn a_denied_automation_request_names_the_system_settings_pane() {
    let (tools, _app, state) = app_tools(false);
    deny_automation(&tools);

    let output = run_apps(&tools, &state, &[("QUIT_TIMEOUT", "1")], &[]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let text = stdout(&output);
    assert!(
        text.contains("Firefox.app could not be quit"),
        "output: {text}"
    );
    assert!(
        text.contains("macOS denied the request to control it"),
        "output: {text}"
    );
    assert!(
        text.contains("System Settings > Privacy & Security > Automation"),
        "output: {text}"
    );
    assert!(
        !text.contains("did not quit"),
        "the app was reported as unquittable: {text}"
    );
}

#[cfg(unix)]
#[test]
fn a_denied_automation_request_still_upgrades_the_casks() {
    let (tools, _app, state) = app_tools(false);
    deny_automation(&tools);

    let output = run_apps(&tools, &state, &[("QUIT_TIMEOUT", "1")], &[]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"brew upgrade --cask".to_owned()),
        "calls: {calls:?}"
    );
}

/// The tools for a cask app that updates itself with Sparkle.
///
/// The `ps` stub shows the Sparkle updater of the app for the first
/// `updater_reads` reads of the process table, so a test can let the update
/// finish while freshen waits for it.
#[cfg(unix)]
fn updating_app_tools(updater_reads: u32) -> (StubTools, std::path::PathBuf, std::path::PathBuf) {
    let tools = StubTools::new(&["brew", "defaults", "ps", "osascript", "open", "kill"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula --quiet")
    printf 'jq\n'
    ;;
  "outdated --cask --greedy --quiet")
    printf 'firefox\n'
    ;;
  "info --cask --json=v2 firefox")
    printf '%s\n' "$FRESHEN_BREW_INFO_JSON"
    ;;
esac"#,
    );

    let app = tools.home().join("Applications/Firefox.app");
    std::fs::create_dir_all(app.join("Contents/Frameworks/Sparkle.framework"))
        .unwrap_or_else(|error| panic!("failed to create test app: {error}"));

    tools.set_script("defaults", "printf 'com.example.firefox\\n'");

    let state = tools.home().join("ps.txt");
    std::fs::write(
        &state,
        format!("{}/Contents/MacOS/firefox\n", app.display()),
    )
    .unwrap_or_else(|error| panic!("failed to write process table: {error}"));

    let updater = tools.home().join("updater.txt");
    std::fs::write(
        &updater,
        format!(
            "{}/Contents/Frameworks/Sparkle.framework/Versions/B/Autoupdate\n",
            app.display()
        ),
    )
    .unwrap_or_else(|error| panic!("failed to write updater state: {error}"));

    tools.set_script(
        "ps",
        &format!(
            r#"count_file="$FRESHEN_STUB_STATE/ps.count"
count=0
if [ -f "$count_file" ]; then read -r count < "$count_file"; fi
count=$((count + 1))
printf '%s\n' "$count" > "$count_file"
while IFS= read -r line; do printf '%s\n' "$line"; done < "$FRESHEN_PS_STATE"
if [ "$count" -le {updater_reads} ]; then
  while IFS= read -r line; do printf '%s\n' "$line"; done < "$FRESHEN_PS_UPDATER"
fi"#
        ),
    );
    tools.set_script("osascript", ": > \"$FRESHEN_PS_STATE\"");

    (tools, state, updater)
}

/// Runs freshen against an app that updates itself.
#[cfg(unix)]
fn run_updating_apps(
    tools: &StubTools,
    state: &std::path::Path,
    updater: &std::path::Path,
    extra_env: &[(&str, &str)],
) -> Output {
    let mut command = tools.command();
    command
        .env("FRESHEN_BREW_INFO_JSON", CASK_INFO)
        .env("FRESHEN_PS_STATE", state)
        .env("FRESHEN_PS_UPDATER", updater)
        .envs(extra_env.iter().copied())
        .args(["--only", HOMEBREW_STEPS, "--yes", "--cleanup"]);
    command
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[cfg(unix)]
#[test]
fn an_app_that_is_updating_itself_is_not_quit() {
    let (tools, state, updater) = updating_app_tools(99);

    let output = run_updating_apps(&tools, &state, &updater, &[("UPDATE_TIMEOUT", "0")]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        !calls.iter().any(|call| call.contains("to quit")),
        "the app was quit: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.contains("open -g -b")),
        "the app was reopened: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("Firefox.app is updating itself"),
        "output: {text}"
    );
    assert!(text.contains("did not finish"), "output: {text}");
}

#[cfg(unix)]
#[test]
fn an_app_that_finishes_updating_is_quit_after_freshen_waited_for_it() {
    let (tools, state, updater) = updating_app_tools(3);

    let output = run_updating_apps(&tools, &state, &updater, &[("UPDATE_TIMEOUT", "30")]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.iter().any(|call| call.contains("to quit")),
        "the app was not quit: {calls:?}"
    );
    assert!(
        calls.iter().any(|call| call.contains("open -g -b")),
        "the app was not reopened: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("Waiting up to 30s for Firefox.app to finish updating itself (Sparkle)"),
        "output: {text}"
    );
    assert!(text.contains("Quitting: Firefox.app"), "output: {text}");
}

#[cfg(unix)]
#[test]
fn force_kill_terminates_an_app_that_does_not_quit_gracefully() {
    let (tools, _app, state) = app_tools(false);

    let output = run_apps(&tools, &state, &[("QUIT_TIMEOUT", "0")], &["--force-kill"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
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

/// A cask that installs a menu-bar app: the app lives in the menu bar, so it
/// has no icon in the Dock.
#[cfg(unix)]
const CASK_INFO_WITH_MENU_BAR_APP: &str = r#"{
    "casks": [
        {
            "token": "menu-bar",
            "artifacts": [
                {
                    "app": [
                        "MenuBar.app"
                    ]
                }
            ]
        }
    ]
}"#;

/// The stub tools for a cask that installs an app without a Dock icon.
///
/// The `defaults` stub answers the bundle id and the `LSUIElement` key, which
/// is what tells freshen that the app lives in the menu bar. The `kill` stub
/// empties the process table, so the app is gone after it was ended.
#[cfg(unix)]
fn background_app_tools(
    app_name: &str,
    bundle_id: &str,
    processes: &[(&str, u32)],
) -> (StubTools, std::path::PathBuf, std::path::PathBuf) {
    let tools = StubTools::new(&["brew", "defaults", "ps", "osascript", "open", "kill"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula --quiet")
    printf 'jq\n'
    ;;
  "outdated --cask --greedy --quiet")
    printf 'menu-bar\n'
    ;;
  "info --cask --json=v2 menu-bar")
    printf '%s\n' "$FRESHEN_BREW_INFO_JSON"
    ;;
esac"#,
    );

    let app = tools.home().join("Applications").join(app_name);
    std::fs::create_dir_all(&app)
        .unwrap_or_else(|error| panic!("failed to create test app: {error}"));

    tools.set_script(
        "defaults",
        &format!(
            r#"case "$*" in
  *CFBundleIdentifier) printf '{bundle_id}\n' ;;
  *LSUIElement) printf '1\n' ;;
esac"#
        ),
    );

    let state = tools.home().join("ps.txt");
    let table: String = processes
        .iter()
        .map(|(executable, pid)| format!("{pid} {}\n", app.join(executable).display()))
        .collect();
    std::fs::write(&state, table)
        .unwrap_or_else(|error| panic!("failed to write process table: {error}"));

    // The stubs may only use shell builtins, so the killed processes are
    // remembered in a file next to the process table.
    tools.set_script(
        "ps",
        r#"killed="$FRESHEN_PS_STATE.killed"
while IFS=' ' read -r pid rest; do
  gone=0
  if [ -f "$killed" ]; then
    while IFS=' ' read -r dead; do
      if [ "$pid" = "$dead" ]; then gone=1; fi
    done < "$killed"
  fi
  if [ "$gone" = 0 ]; then printf '%s %s\n' "$pid" "$rest"; fi
done < "$FRESHEN_PS_STATE""#,
    );
    tools.set_script(
        "kill",
        r#"killed="$FRESHEN_PS_STATE.killed"
for arg in "$@"; do
  case "$arg" in -*) continue ;; esac
  printf '%s\n' "$arg" >> "$killed"
done"#,
    );

    (tools, app, state)
}

#[cfg(unix)]
#[test]
fn a_menu_bar_app_is_ended_and_reopened_in_the_background() {
    let (tools, app, state) = background_app_tools(
        "MenuBar.app",
        "com.example.menubar",
        &[("Contents/MacOS/menubar", 123)],
    );

    let mut command = tools.command();
    command
        .env("FRESHEN_BREW_INFO_JSON", CASK_INFO_WITH_MENU_BAR_APP)
        .env("FRESHEN_PS_STATE", &state)
        .args(["--only", HOMEBREW_STEPS, "--yes", "--cleanup"]);
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"kill -TERM 123".to_owned()),
        "the app was not ended: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.contains("to quit")),
        "the app was quit through an Apple event: {calls:?}"
    );
    assert_eq!(
        calls.last().map(String::as_str),
        Some("open -g -j -b com.example.menubar")
    );

    let text = stdout(&output);
    assert!(text.contains("Quitting: MenuBar.app"), "output: {text}");
    assert!(
        text.contains("Opening: com.example.menubar (hidden)"),
        "output: {text}"
    );
    assert!(text.contains(&app.display().to_string()), "output: {text}");
}

#[cfg(unix)]
#[test]
fn a_login_item_is_ended_and_reopened_by_its_path() {
    let (tools, app, state) = background_app_tools(
        "MenuBar.app",
        "com.example.menubar",
        &[("Contents/MacOS/menubar", 123)],
    );
    // The app starts a helper at login; the helper runs without a Dock icon.
    let helper = app.join("Contents/Library/LoginItems/Helper.app");
    std::fs::create_dir_all(helper.join("Contents/MacOS"))
        .unwrap_or_else(|error| panic!("failed to create test helper: {error}"));
    tools.set_script(
        "defaults",
        r#"case "$*" in
  *CFBundleIdentifier)
    case "$*" in
      *Helper.app*) printf 'com.example.helper\n' ;;
      *) printf 'com.example.menubar\n' ;;
    esac
    ;;
  *LSUIElement) printf '1\n' ;;
esac"#,
    );
    let mut table = std::fs::read_to_string(&state)
        .unwrap_or_else(|error| panic!("failed to read process table: {error}"));
    table.push_str(&format!(
        "  124 {}\n",
        helper.join("Contents/MacOS/Helper").display()
    ));
    std::fs::write(&state, table)
        .unwrap_or_else(|error| panic!("failed to write process table: {error}"));

    let mut command = tools.command();
    command
        .env("FRESHEN_BREW_INFO_JSON", CASK_INFO_WITH_MENU_BAR_APP)
        .env("FRESHEN_PS_STATE", &state)
        .args(["--only", HOMEBREW_STEPS, "--yes", "--cleanup"]);
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"kill -TERM 124".to_owned()),
        "the login item was not ended: {calls:?}"
    );
    assert!(
        calls.contains(&format!("open -g -j -a {}", helper.display())),
        "the login item was not reopened by its path: {calls:?}"
    );

    let text = stdout(&output);
    assert!(text.contains("Quitting: Helper.app"), "output: {text}");
    assert!(
        text.contains("Opening: com.example.helper (login item)"),
        "output: {text}"
    );
}

#[test]
fn pinned_formulae_are_held_back_and_not_upgraded() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula")
    printf 'jq (1.7.1) < 1.8.0\nnode (20.10.0) < 20.11.0\n'
    ;;
  "outdated --formula --quiet")
    printf 'jq\nnode\n'
    ;;
  "list --pinned --formula")
    printf 'node\n'
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", "brew,brew-formulae"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&"brew upgrade --formula jq".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call == "brew upgrade --formula" || call == "brew upgrade --formula node"),
        "the pinned formula was passed to brew upgrade: {calls:?}"
    );

    let text = stdout(&output);
    assert!(text.contains("Held back (pinned): node"), "output: {text}");
}

#[test]
fn a_run_with_only_pinned_formulae_skips_the_upgrade() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula")
    printf 'node (20.10.0) < 20.11.0\n'
    ;;
  "outdated --formula --quiet")
    printf 'node\n'
    ;;
  "list --pinned --formula")
    printf 'node\n'
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", "brew,brew-formulae"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !tools
            .step_calls()
            .iter()
            .any(|call| call.starts_with("brew upgrade")),
        "the pinned formula was upgraded: {:?}",
        tools.step_calls()
    );

    let text = stdout(&output);
    assert!(text.contains("Held back (pinned): node"), "output: {text}");
    assert!(
        text.contains("every outdated formula is held back (pinned)"),
        "output: {text}"
    );
}

#[test]
fn check_reports_pinned_formulae_as_held_back() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula")
    printf 'jq (1.7.1) < 1.8.0\nnode (20.10.0) < 20.11.0\n'
    ;;
  "list --pinned --formula")
    printf 'node\n'
    ;;
esac"#,
    );

    let output = run(
        &tools,
        &["--check", "--jobs", "1", "--only", "brew-formulae"],
    );

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        ["brew outdated --formula", "brew list --pinned --formula"]
    );

    let text = stdout(&output);
    assert!(text.contains("jq 1.7.1 -> 1.8.0"), "output: {text}");
    assert!(
        text.contains("node 20.10.0 -> 20.11.0 (held back (pinned))"),
        "output: {text}"
    );
}

fn run_formulae(outdated: &str, inside_zellij: bool) -> (Output, Vec<String>) {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &homebrew_responses(&[
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout(outdated),
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::new(),
        ]),
    );

    let mut command = tools.command();
    command.args(["--only", "brew,brew-formulae"]);
    if inside_zellij {
        command.env("ZELLIJ", "0");
    } else {
        command.env_remove("ZELLIJ");
    }

    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    (output, tools.step_calls())
}

#[test]
fn warns_when_zellij_is_upgraded_from_inside_a_zellij_session() {
    let (output, calls) = run_formulae("zellij\nnode\n", true);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        calls,
        [
            "brew update",
            "brew outdated --formula",
            "brew outdated --formula --quiet",
            "brew list --pinned --formula",
            "brew services list",
            "brew upgrade --formula",
        ]
    );
    assert!(
        stdout(&output)
            .contains("warning: zellij will be upgraded while you are inside a zellij session"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn no_zellij_warning_outside_a_session_or_when_zellij_is_current() {
    let (outside, _) = run_formulae("zellij\n", false);
    let (current, _) = run_formulae("node\n", true);

    for output in [outside, current] {
        assert!(output.status.success(), "stderr: {}", stderr(&output));
        assert!(
            !stdout(&output).contains("zellij will be upgraded"),
            "unexpected warning: {}",
            stdout(&output)
        );
    }
}

#[test]
fn check_reports_outdated_versions_and_exits_2() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &[
            StubOutput::stdout("jq (1.7.1) < 1.8.0\nnode (20.10.0) < 20.11.0\n"),
            StubOutput::new(),
            StubOutput::stdout("firefox (121.0) != 122.0\n"),
            StubOutput::stdout(CASK_INFO),
        ],
    );

    let output = run(
        &tools,
        &[
            "--check",
            "--jobs",
            "1",
            "--only",
            "brew-formulae,brew-casks",
        ],
    );

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "brew outdated --formula",
            "brew list --pinned --formula",
            "brew outdated --cask --greedy",
            "brew info --cask --json=v2 firefox",
        ]
    );

    let text = stdout(&output);
    assert!(text.contains("Homebrew formulae:"), "output: {text}");
    assert!(text.contains("jq 1.7.1 -> 1.8.0"), "output: {text}");
    assert!(text.contains("node 20.10.0 -> 20.11.0"), "output: {text}");
    assert!(text.contains("firefox 121.0 -> 122.0"), "output: {text}");
    assert!(
        text.contains("3 update(s) pending across 2 step(s)."),
        "output: {text}"
    );
}

#[test]
fn check_explains_which_casks_need_greedy() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &[
            StubOutput::stdout(
                "spotify (1.2.59) != 1.2.60\nnightly (1.0) != latest\nboth (1.0) != latest\nfirefox (133.0) != 134.0\n",
            ),
            StubOutput::stdout(CASK_INFO_WITH_GREEDY),
        ],
    );

    let output = run(&tools, &["--check", "--jobs", "1", "--only", "brew-casks"]);

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "brew outdated --cask --greedy",
            "brew info --cask --json=v2 spotify nightly both firefox",
        ]
    );

    let text = stdout(&output);
    assert!(
        text.contains("spotify 1.2.59 -> 1.2.60 (auto_updates true; needs --greedy)"),
        "output: {text}"
    );
    assert!(
        text.contains("nightly 1.0 -> latest (version :latest; needs --greedy)"),
        "output: {text}"
    );
    assert!(
        text.contains("both 1.0 -> latest (version :latest and auto_updates true; needs --greedy)"),
        "output: {text}"
    );
    assert!(text.contains("firefox 133.0 -> 134.0\n"), "output: {text}");
}

#[test]
fn check_without_cask_metadata_still_reports_the_casks() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &[
            StubOutput::stdout("nightly (1.0) != latest\n"),
            StubOutput::stdout("not json\n"),
        ],
    );

    let output = run(&tools, &["--check", "--jobs", "1", "--only", "brew-casks"]);

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("nightly 1.0 -> latest"),
        "output: {}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("could not read cask metadata"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn a_cask_that_needs_greedy_makes_the_upgrade_pass_greedy() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &homebrew_responses(&[
            StubOutput::new(),
            StubOutput::stdout("firefox\nnightly\n"),
            StubOutput::stdout(CASK_INFO_WITH_GREEDY),
            StubOutput::new(),
        ]),
    );

    let output = run(&tools, &["--only", "brew-casks"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox nightly",
            "brew upgrade --cask --greedy",
        ]
    );
}

#[test]
fn regular_casks_are_upgraded_without_greedy() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &homebrew_responses(&[
            StubOutput::new(),
            StubOutput::stdout("firefox\n"),
            StubOutput::stdout(CASK_INFO),
            StubOutput::new(),
        ]),
    );

    let output = run(&tools, &["--only", "brew-casks"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox",
            "brew upgrade --cask",
        ]
    );
}

#[test]
fn check_exits_zero_when_homebrew_is_up_to_date() {
    let tools = StubTools::new(&["brew"]);

    let output = run(
        &tools,
        &[
            "--check",
            "--jobs",
            "1",
            "--only",
            "brew-formulae,brew-casks",
        ],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        ["brew outdated --formula", "brew outdated --cask --greedy"]
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_skips_a_missing_tool_and_never_calls_it() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, &["--check", "--only", "brew-formulae"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[cfg(unix)]
#[test]
fn check_reports_a_hung_query_as_timed_out_with_exit_1() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula")
    /bin/sleep 5
    ;;
esac"#,
    );

    let output = run(
        &tools,
        &["--check", "--only", "brew-formulae", "--step-timeout", "1"],
    );

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), ["brew outdated --formula"]);
    let text = stdout(&output);
    assert!(
        text.contains("Homebrew formulae: timed out"),
        "output: {text}"
    );
    assert!(!text.contains("failed during check"), "output: {text}");
}

#[test]
fn check_reports_a_failed_query_with_exit_1() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses("brew", &[StubOutput::new().with_status(1)]);

    let output = run(&tools, &["--check", "--only", "brew-formulae"]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), ["brew outdated --formula"]);
    assert!(
        stdout(&output).contains("failed during check"),
        "output: {}",
        stdout(&output)
    );
}

/// The error Homebrew prints when the Command Line Tools are too old, which is
/// what happens right after a macOS update.
const OUTDATED_COMMAND_LINE_TOOLS: &str = "Error: Your Command Line Tools are too outdated. \
     Update them from Software Update in the App Store.\n";

#[test]
fn the_error_about_the_outdated_command_line_tools_names_the_fix() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &homebrew_responses(&[StubOutput::stderr(OUTDATED_COMMAND_LINE_TOOLS).with_status(1)]),
    );

    let output = run(&tools, &["--only", "brew"]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("the Xcode Command Line Tools are out of date"),
        "output: {text}"
    );
    assert!(
        text.contains("softwareupdate --install \"Command Line Tools for Xcode\""),
        "output: {text}"
    );
}

#[test]
fn a_failed_check_about_the_outdated_command_line_tools_names_the_fix() {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &[StubOutput::stderr(OUTDATED_COMMAND_LINE_TOOLS).with_status(1)],
    );

    let output = run(&tools, &["--check", "--only", "brew-formulae"]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("the Xcode Command Line Tools are out of date"),
        "output: {text}"
    );
    assert!(
        text.contains("softwareupdate --install \"Command Line Tools for Xcode\""),
        "output: {text}"
    );
}
