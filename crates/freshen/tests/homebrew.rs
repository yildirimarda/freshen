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
            "brew upgrade --formula",
            "brew outdated --cask --greedy",
            "brew outdated --cask --greedy --quiet",
            "brew info --cask --json=v2 firefox google-chrome",
            "brew upgrade --cask --greedy",
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
fn a_missing_brew_skips_every_homebrew_step_without_calling_anything() {
    let tools = StubTools::new(&[]);

    let output = run(&tools, &["--only", HOMEBREW_STEPS]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 6 skipped, 0 failed"),
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
    let text = stdout(&output);
    assert!(
        text.contains("5 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {text}"
    );
    assert!(text.contains("failed"), "summary: {text}");
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
        if osascript_quits {
            ": > \"$FRESHEN_PS_STATE\""
        } else {
            ":"
        },
    );

    (tools, app, state)
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
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!("defaults read {} CFBundleIdentifier", info.display()),
            "ps -axo command=".to_owned(),
            "osascript -e with timeout of 3 seconds -e tell application id \"com.example.firefox\" to quit -e end timeout".to_owned(),
            "ps -axo command=".to_owned(),
            "brew upgrade --cask --greedy".to_owned(),
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
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!("defaults read {} CFBundleIdentifier", info.display()),
            "ps -axo command=".to_owned(),
            "osascript -e with timeout of 3 seconds -e tell application id \"com.example.firefox\" to quit -e end timeout".to_owned(),
            "ps -axo command=".to_owned(),
            "brew upgrade --cask --greedy".to_owned(),
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
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!("defaults read {} CFBundleIdentifier", info.display()),
            "ps -axo command=".to_owned(),
            "brew upgrade --cask --greedy".to_owned(),
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
            "brew upgrade --formula".to_owned(),
            "brew outdated --cask --greedy".to_owned(),
            "brew outdated --cask --greedy --quiet".to_owned(),
            "brew info --cask --json=v2 firefox".to_owned(),
            format!("defaults read {} CFBundleIdentifier", info.display()),
            "ps -axo command=".to_owned(),
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
            "brew upgrade --cask --greedy".to_owned(),
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

fn run_formulae(outdated: &str, inside_zellij: bool) -> (Output, Vec<String>) {
    let tools = StubTools::new(&["brew"]);
    tools.set_responses(
        "brew",
        &homebrew_responses(&[
            StubOutput::new(),
            StubOutput::new(),
            StubOutput::stdout(outdated),
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
            StubOutput::stdout("firefox (121.0) != 122.0\n"),
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
        ["brew outdated --formula", "brew outdated --cask --greedy"]
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
