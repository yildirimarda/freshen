//! Parity tests for the uv and mise steps.
//!
//! Every test runs the built binary against stub `uv`, `mise` and `brew`
//! executables and asserts the exact command transcript of the corresponding
//! block in `update-all.sh`. No real tool is touched.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn uv_responses(transcript: &[StubOutput]) -> Vec<StubOutput> {
    let mut responses = vec![StubOutput::new()];
    responses.extend_from_slice(transcript);
    responses
}

fn mise_responses(transcript: &[StubOutput]) -> Vec<StubOutput> {
    let mut responses = vec![StubOutput::new()];
    responses.extend_from_slice(transcript);
    responses
}

fn run(tools: &StubTools, steps: &str) -> Output {
    tools
        .command()
        .args(["--only", steps, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn uv_upgrades_tools_and_prunes_the_cache() {
    let tools = StubTools::new(&["uv"]);
    tools.set_responses(
        "uv",
        &uv_responses(&[StubOutput::new(), StubOutput::new(), StubOutput::new()]),
    );

    let output = run(&tools, "uv");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::uv_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
}

#[test]
fn a_failing_uv_upgrade_still_prunes_the_cache_and_fails_the_step() {
    let tools = StubTools::new(&["uv"]);
    tools.set_responses(
        "uv",
        &uv_responses(&[
            StubOutput::new(),
            StubOutput::new().with_status(1),
            StubOutput::new(),
        ]),
    );

    let output = tools
        .command()
        .args(["--only", "uv", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::uv_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn mise_self_updates_upgrades_and_prunes_when_not_installed_with_brew() {
    let tools = StubTools::new(&["brew", "mise"]);
    tools.set_responses("brew", &[StubOutput::new().with_status(1)]);
    tools.set_responses(
        "mise",
        &mise_responses(&[
            // The `check()` query (`mise outdated`).
            StubOutput::new(),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n"),
            // The self-update (`mise self-update -y`).
            StubOutput::new(),
            // The active version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The upgrade (`mise upgrade`).
            StubOutput::new(),
            // The active version after the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ]),
    );

    let output = run(&tools, "mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::mise_brew_failed_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
}

#[test]
fn mise_installed_with_brew_is_not_self_updated() {
    let tools = StubTools::new(&["brew", "mise"]);
    tools.set_responses("brew", &[StubOutput::new()]);
    tools.set_responses(
        "mise",
        &mise_responses(&[
            // The `check()` query (`mise outdated`).
            StubOutput::new(),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n"),
            // The active version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The upgrade (`mise upgrade`).
            StubOutput::new(),
            // The active version after the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ]),
    );

    let output = tools
        .command()
        .args(["--only", "mise", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::mise_brew_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
}

#[test]
fn mise_self_updates_when_brew_is_not_installed() {
    let tools = StubTools::new(&["mise"]);
    tools.set_responses(
        "mise",
        &mise_responses(&[
            // The `check()` query (`mise outdated`).
            StubOutput::new(),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n"),
            // The self-update (`mise self-update -y`).
            StubOutput::new(),
            // The active version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The upgrade (`mise upgrade`).
            StubOutput::new(),
            // The active version after the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ]),
    );

    let output = run(&tools, "mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::mise_full_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
}

#[test]
fn a_failing_mise_self_update_is_only_a_warning() {
    let tools = StubTools::new(&["mise"]);
    tools.set_responses(
        "mise",
        &mise_responses(&[
            // The `check()` query (`mise outdated`).
            StubOutput::new(),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n"),
            // The self-update (`mise self-update -y`).
            StubOutput::new().with_status(1),
            // The active version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The upgrade (`mise upgrade`).
            StubOutput::new(),
            // The active version after the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ]),
    );

    let output = tools
        .command()
        .args(["--only", "mise", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::mise_full_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
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
    tools.set_responses(
        "mise",
        &mise_responses(&[
            // The `check()` query (`mise outdated`).
            StubOutput::new(),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n"),
            // The self-update (`mise self-update -y`).
            StubOutput::new(),
            // The active version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The upgrade (`mise upgrade`).
            StubOutput::new().with_status(1),
            // The active version after the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ]),
    );

    let output = tools
        .command()
        .args(["--only", "mise", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        support::mise_full_transcript()
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
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
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 2 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[cfg(unix)]
fn install_stale_node(tools: &StubTools) {
    tools.set_script(
        "mise",
        r#"case "$*" in
  "outdated")
    printf '%s' ''
    ;;
  "current node")
    printf '%s' '20.0.0'
    ;;
  "ls-remote node@lts")
    printf '%s' '20.18.0'
    ;;
  "ls --prunable --json")
    printf '%s' '{"node":[{"version":"20.0.0","install_path":"/home/user/.local/share/mise/installs/node/20.0.0"}]}'
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
      '  102     1 /usr/bin/node /path/system.js' \
      '  103     1 /home/user/.local/share/mise/installs/node/24.0.0/bin/node /path/newer.js'
    ;;
  "-o comm= -p 1")
    printf '/Applications/LM Studio.app/Contents/MacOS/LM Studio\n'
    ;;
esac"#,
    );
}

#[cfg(unix)]
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
    assert!(
        !text.contains("pid 103"),
        "a newer non-active version that mise keeps must not be reported as old: {text}"
    );
    let mut expected_calls = support::mise_full_transcript();
    expected_calls.extend(support::stale_node_ps_steps().iter().copied());
    assert_eq!(
        tools.step_calls(),
        expected_calls
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
    );
}

#[cfg(unix)]
#[test]
fn warns_about_old_node_python_java_and_go_processes_with_their_parent_apps() {
    let tools = StubTools::new(&["mise", "ps"]);
    tools.set_script(
        "mise",
        r#"case "$*" in
  "ls --prunable --json")
    printf '%s' '{"node":[{"version":"20.0.0","install_path":"/home/user/.local/share/mise/installs/node/20.0.0"}],"python":[{"version":"3.10.0","install_path":"/home/user/.local/share/mise/installs/python/3.10.0"}],"java":[{"version":"17.0.0","install_path":"/home/user/.local/share/mise/installs/java/17.0.0"}],"go":[{"version":"1.21.0","install_path":"/home/user/.local/share/mise/installs/go/1.21.0"}]}'
    ;;
esac"#,
    );
    tools.set_script(
        "ps",
        r#"case "$*" in
  "-axo pid=,ppid=,command=")
    printf '%s\n' \
      '  100    10 /home/user/.local/share/mise/installs/node/20.0.0/bin/node /path/old.js' \
      '  101    10 /home/user/.local/share/mise/installs/node/22.1.0/bin/node /path/current.js' \
      '  200    20 /home/user/.local/share/mise/installs/python/3.10.0/bin/python3 /path/old.py' \
      '  201    20 /home/user/.local/share/mise/installs/python/3.12.0/bin/python3 /path/current.py' \
      '  300    30 /home/user/.local/share/mise/installs/java/17.0.0/bin/java -jar old.jar' \
      '  301    30 /home/user/.local/share/mise/installs/java/21.0.0/bin/java -jar current.jar' \
      '  400    40 /home/user/.local/share/mise/installs/go/1.21.0/bin/go run main.go' \
      '  401    40 /home/user/.local/share/mise/installs/go/1.22.0/bin/go run main.go'
    ;;
  "-o comm= -p 10")
    printf '/Applications/LM Studio.app/Contents/MacOS/LM Studio\n'
    ;;
  "-o comm= -p 20")
    printf '/Applications/Code.app/Contents/MacOS/Electron\n'
    ;;
  "-o comm= -p 30")
    printf '/Applications/IntelliJ IDEA.app/Contents/MacOS/idea\n'
    ;;
  "-o comm= -p 40")
    printf '/usr/local/bin/zsh\n'
    ;;
esac"#,
    );

    let output = run(&tools, "mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    for (tool, pid, parent, command) in [
        (
            "node",
            "100",
            "LM Studio",
            "/home/user/.local/share/mise/installs/node/20.0.0/bin/node /path/old.js",
        ),
        (
            "python",
            "200",
            "Electron",
            "/home/user/.local/share/mise/installs/python/3.10.0/bin/python3 /path/old.py",
        ),
        (
            "java",
            "300",
            "idea",
            "/home/user/.local/share/mise/installs/java/17.0.0/bin/java -jar old.jar",
        ),
        (
            "go",
            "400",
            "zsh",
            "/home/user/.local/share/mise/installs/go/1.21.0/bin/go run main.go",
        ),
    ] {
        assert!(
            text.contains(&format!(
                "warning: Processes still running on an old {tool} version (restart the app that started them so mise can prune it):"
            )),
            "missing {tool} warning: {text}"
        );
        assert!(
            text.contains(&format!("   - pid {pid} (started by: {parent}): {command}")),
            "missing {tool} process line: {text}"
        );
    }
    for pid in ["101", "201", "301", "401"] {
        assert!(
            !text.contains(&format!("pid {pid}")),
            "the active version must not be reported: {text}"
        );
    }
}

#[cfg(unix)]
#[test]
fn no_stale_node_warning_when_every_process_uses_the_active_version() {
    let tools = StubTools::new(&["mise", "ps"]);
    tools.set_script(
        "mise",
        r#"case "$*" in
  "ls --prunable --json")
    printf '%s' '{"node":[{"version":"20.0.0","install_path":"/home/user/.local/share/mise/installs/node/20.0.0"}]}'
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
        tools.step_calls()
    );
}

#[test]
fn check_reports_outdated_uv_tools_and_exits_2() {
    let tools = StubTools::new(&["uv"]);
    tools.set_responses(
        "uv",
        &[StubOutput::stdout(
            "black v23.1.0 [latest: 26.10.1]\n- black\n- blackd\nruff v0.6.0 [latest: 0.7.0]\n- ruff\n",
        )],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "uv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec!["uv tool list --outdated".to_string()]
    );
    let text = stdout(&output);
    assert!(text.contains("uv:"), "output: {text}");
    assert!(text.contains("black 23.1.0 -> 26.10.1"), "output: {text}");
    assert!(text.contains("ruff 0.6.0 -> 0.7.0"), "output: {text}");
    assert!(
        text.contains("2 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
}

#[test]
fn check_reports_outdated_mise_tools_and_exits_2() {
    let tools = StubTools::new(&["mise"]);
    tools.set_responses(
        "mise",
        &[
            // The `check()` query (`mise outdated`).
            StubOutput::stdout(
                "Plugin  Requested  Current  Latest\nnode    20         20.0.0   20.1.0\npython  3.11       3.11.0   3.11.1\n",
            ),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "mise outdated".to_string(),
            "mise current node".to_string(),
            "mise ls-remote node@lts".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(text.contains("mise:"), "output: {text}");
    assert!(text.contains("node 20.0.0 -> 20.1.0"), "output: {text}");
    assert!(text.contains("python 3.11.0 -> 3.11.1"), "output: {text}");
    assert!(
        text.contains("node 20.0.0 -> 22.12.0"),
        "missing LTS major report: {text}"
    );
    assert!(
        text.contains("3 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
}

#[test]
fn check_reports_uv_and_mise_up_to_date_without_running_them() {
    let tools = StubTools::new(&["uv", "mise"]);
    tools.set_responses("uv", &[StubOutput::stdout("No tools installed\n")]);
    tools.set_responses(
        "mise",
        &[
            StubOutput::new(),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--jobs", "1", "--only", "uv,mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "uv tool list --outdated".to_string(),
            "mise outdated".to_string(),
            "mise current node".to_string(),
            "mise ls-remote node@lts".to_string(),
        ]
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_reports_a_failed_tool_query_with_exit_1() {
    let tools = StubTools::new(&["uv"]);
    tools.set_responses("uv", &[StubOutput::new().with_status(1)]);

    let output = tools
        .command()
        .args(["--check", "--only", "uv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec!["uv tool list --outdated".to_string()]
    );
    assert!(
        stdout(&output).contains("failed during check"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_newer_node_lts_major_without_switching() {
    let tools = StubTools::new(&["mise"]);
    tools.set_responses(
        "mise",
        &[
            // The `check()` query (`mise outdated`).
            StubOutput::new(),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "mise outdated".to_string(),
            "mise current node".to_string(),
            "mise ls-remote node@lts".to_string(),
        ]
    );
    let text = stdout(&output);
    assert!(text.contains("mise:"), "output: {text}");
    assert!(
        text.contains("node 20.0.0 -> 22.12.0"),
        "missing LTS major report: {text}"
    );
    assert!(
        text.contains("newer Node LTS major available"),
        "missing LTS reason: {text}"
    );
    // No `mise use` or `mise install` — the check must not switch.
    assert!(
        !text.contains("mise use"),
        "must not switch automatically: {text}"
    );
    assert!(
        !text.contains("mise install"),
        "must not install automatically: {text}"
    );
}

#[test]
fn check_does_not_report_node_lts_when_active_is_latest_lts() {
    let tools = StubTools::new(&["mise"]);
    tools.set_responses(
        "mise",
        &[
            StubOutput::new(),
            StubOutput::stdout("22.12.0\n"),
            StubOutput::stdout("18.20.0\n20.18.0\n22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "mise outdated".to_string(),
            "mise current node".to_string(),
            "mise ls-remote node@lts".to_string(),
        ]
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_skips_node_lts_when_node_is_not_active() {
    let tools = StubTools::new(&["mise"]);
    tools.set_responses(
        "mise",
        &[
            StubOutput::new(),
            // `mise current node` returns nothing when node is not installed.
            StubOutput::new(),
            StubOutput::stdout("22.12.0\n"),
        ],
    );

    let output = tools
        .command()
        .args(["--check", "--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn mise_upgrade_without_bump_keeps_lts_and_latest() {
    let tools = StubTools::new(&["mise"]);
    tools.set_responses(
        "mise",
        &[
            // The `check()` query (`mise outdated`).
            StubOutput::stdout(
                "Plugin  Requested  Current  Latest\n\
                 node    lts        20.0.0   22.12.0\n\
                 python  latest     3.11.0   3.11.1\n",
            ),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n22.12.0\n"),
            // The self-update (`mise self-update -y`).
            StubOutput::new(),
            // The active version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The upgrade (`mise upgrade` — one command, keeps lts/latest).
            StubOutput::new(),
            // The active version after the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ],
    );

    let output = run(&tools, "mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "mise outdated",
            "mise current node",
            "mise ls-remote node@lts",
            "mise self-update -y",
            "mise current node",
            "mise upgrade",
            "mise current node",
            "mise ls --prunable --json",
            "mise prune -y",
        ]
    );
    let text = stdout(&output);
    assert!(
        !text.contains("warning: mise --bump replaces lts/latest with a fixed version"),
        "unexpected bump warning: {text}"
    );
}

#[test]
fn mise_bump_replaces_lts_and_latest_with_a_fixed_version() {
    let tools = StubTools::new(&["mise"]);
    tools.set_responses(
        "mise",
        &mise_responses(&[
            // The `check()` query (`mise outdated`).
            StubOutput::stdout(
                "Plugin  Requested  Current  Latest\n\
                 node    lts        20.0.0   22.12.0\n\
                 python  latest     3.11.0   3.11.1\n",
            ),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n22.12.0\n"),
            // The self-update (`mise self-update -y`).
            StubOutput::new(),
            // The active version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The `upgrade_tools` bump query (`mise outdated` again).
            StubOutput::stdout(
                "Plugin  Requested  Current  Latest\n\
                 node    lts        20.0.0   22.12.0\n\
                 python  latest     3.11.0   3.11.1\n",
            ),
            // The upgrade pins `lts`/`latest` to a fixed version.
            StubOutput::new(),
            StubOutput::new(),
            // The active version after the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ]),
    );

    let output = tools
        .command()
        .args(["--only", "mise", "--cleanup", "--bump"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "mise outdated",
            "mise current node",
            "mise ls-remote node@lts",
            "mise self-update -y",
            "mise current node",
            "mise outdated",
            "mise install node@22.12.0",
            "mise install python@3.11.1",
            "mise current node",
            "mise ls --prunable --json",
            "mise prune -y",
            // The runner re-checks after the run to compute the version diff
            // because the before-check reported outdated items.
            "mise outdated",
            "mise current node",
        ]
    );
    let text = stdout(&output);
    assert!(
        text.contains("warning: mise --bump replaces lts/latest with a fixed version"),
        "missing bump warning: {text}"
    );
}

/// Runs the mise step with a Node version that goes from 20.0.0 to `after`.
///
/// `~/.default-npm-packages` lives in the stub home directory, so no test
/// touches the real one.
fn run_node_upgrade(tools: &StubTools, after: &str) -> Output {
    tools.set_responses(
        "mise",
        &mise_responses(&[
            // The `check()` query (`mise outdated`).
            StubOutput::new(),
            // The active version (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::stdout("20.18.0\n"),
            // The self-update (`mise self-update -y`).
            StubOutput::new(),
            // The active version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The upgrade (`mise upgrade`).
            StubOutput::new(),
            // The active version after the upgrade (`mise current node`).
            StubOutput::stdout(format!("{after}\n")),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ]),
    );
    run(tools, "mise")
}

fn write_default_npm_packages(tools: &StubTools, contents: &str) {
    let path = tools.home().join(".default-npm-packages");
    std::fs::write(&path, contents)
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

fn npm_calls(tools: &StubTools) -> Vec<String> {
    tools
        .step_calls()
        .into_iter()
        .filter(|call| call.starts_with("npm"))
        .collect()
}

#[test]
fn a_node_version_change_reinstalls_the_default_npm_packages() {
    let tools = StubTools::new(&["mise", "npm"]);
    write_default_npm_packages(&tools, "# global tools\ntypescript\n\n  left-pad  \n");

    let output = run_node_upgrade(&tools, "22.12.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "mise outdated",
            "mise current node",
            "mise ls-remote node@lts",
            "mise self-update -y",
            "mise current node",
            "mise upgrade",
            "mise current node",
            "npm install -g typescript left-pad",
            "mise ls --prunable --json",
            "mise prune -y",
        ]
    );
}

#[test]
fn an_unchanged_node_version_does_not_reinstall_the_default_npm_packages() {
    let tools = StubTools::new(&["mise", "npm"]);
    write_default_npm_packages(&tools, "typescript\n");

    let output = run_node_upgrade(&tools, "20.0.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        npm_calls(&tools).is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn a_missing_default_npm_packages_file_is_not_an_error() {
    let tools = StubTools::new(&["mise", "npm"]);

    let output = run_node_upgrade(&tools, "22.12.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        npm_calls(&tools).is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn a_default_npm_packages_file_with_only_comments_installs_nothing() {
    let tools = StubTools::new(&["mise", "npm"]);
    write_default_npm_packages(&tools, "# nothing yet\n\n   \n");

    let output = run_node_upgrade(&tools, "22.12.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        npm_calls(&tools).is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn a_failing_npm_reinstall_is_only_a_warning() {
    let tools = StubTools::new(&["mise", "npm"]);
    write_default_npm_packages(&tools, "typescript\n");
    tools.set_responses("npm", &[StubOutput::new().with_status(1)]);

    let output = run_node_upgrade(&tools, "22.12.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("warning: could not reinstall default npm packages"),
        "stdout: {}",
        stdout(&output)
    );
    assert!(
        tools.step_calls().contains(&"mise prune -y".to_owned()),
        "the prune must still run: {:?}",
        tools.step_calls()
    );
}

/// Runs the mise step with a Python version that goes from 3.11.0 to `after`,
/// probing both the Node and Python versions (the run always probes Node).
///
/// `~/.default-python-packages` lives in the stub home directory, so no test
/// touches the real one.
fn run_python_upgrade(tools: &StubTools, after: &str) -> Output {
    tools.set_responses(
        "mise",
        &mise_responses(&[
            // The `check()` query (`mise outdated`).
            StubOutput::new(),
            // The active version for the node LTS check (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::new(),
            // The self-update (`mise self-update -y`, brew is not stubbed).
            StubOutput::new(),
            // The active Node version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The active Python version before the upgrade (`mise current python`).
            StubOutput::stdout("3.11.0\n"),
            // The upgrade (`mise upgrade`).
            StubOutput::new(),
            // The active Node version after the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The active Python version after the upgrade (`mise current python`).
            StubOutput::stdout(format!("{after}\n")),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ]),
    );
    run(tools, "mise")
}

/// Runs the mise step when `~/.default-python-packages` does not exist, so no
/// Python version is probed at all.
fn run_python_upgrade_without_defaults(tools: &StubTools, _after: &str) -> Output {
    tools.set_responses(
        "mise",
        &mise_responses(&[
            // The `check()` query (`mise outdated`).
            StubOutput::new(),
            // The active version for the node LTS check (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The available LTS versions (`mise ls-remote node@lts`).
            StubOutput::new(),
            // The self-update (`mise self-update -y`, brew is not stubbed).
            StubOutput::new(),
            // The active Node version before the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The upgrade (`mise upgrade`).
            StubOutput::new(),
            // The active Node version after the upgrade (`mise current node`).
            StubOutput::stdout("20.0.0\n"),
            // The prune list (`mise ls --prunable --json`).
            StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            StubOutput::new(),
        ]),
    );
    run(tools, "mise")
}

fn write_default_python_packages(tools: &StubTools, contents: &str) {
    let path = tools.home().join(".default-python-packages");
    std::fs::write(&path, contents)
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

fn pip_calls(tools: &StubTools) -> Vec<String> {
    tools
        .step_calls()
        .into_iter()
        .filter(|call| call.starts_with("pip"))
        .collect()
}

#[test]
fn a_python_version_change_reinstalls_the_default_python_packages() {
    let tools = StubTools::new(&["mise", "pip"]);
    write_default_python_packages(&tools, "# default python\ntypescript-stubs\n\n  black  \n");

    let output = run_python_upgrade(&tools, "3.12.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec![
            "mise outdated",
            "mise current node",
            "mise ls-remote node@lts",
            "mise self-update -y",
            "mise current node",
            "mise current python",
            "mise upgrade",
            "mise current node",
            "mise current python",
            "pip install typescript-stubs black",
            "mise ls --prunable --json",
            "mise prune -y",
        ]
    );
}

#[test]
fn an_unchanged_python_version_does_not_reinstall_the_default_python_packages() {
    let tools = StubTools::new(&["mise", "pip"]);
    write_default_python_packages(&tools, "black\n");

    let output = run_python_upgrade(&tools, "3.11.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        pip_calls(&tools).is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn a_missing_default_python_packages_file_is_not_an_error() {
    let tools = StubTools::new(&["mise", "pip"]);

    let output = run_python_upgrade_without_defaults(&tools, "3.12.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        pip_calls(&tools).is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn a_default_python_packages_file_with_only_comments_installs_nothing() {
    let tools = StubTools::new(&["mise", "pip"]);
    write_default_python_packages(&tools, "# nothing yet\n\n   \n");

    let output = run_python_upgrade(&tools, "3.12.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        pip_calls(&tools).is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn a_failing_pip_reinstall_is_only_a_warning() {
    let tools = StubTools::new(&["mise", "pip"]);
    write_default_python_packages(&tools, "black\n");
    tools.set_responses("pip", &[StubOutput::new().with_status(1)]);

    let output = run_python_upgrade(&tools, "3.12.0");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("warning: could not reinstall default python packages"),
        "stdout: {}",
        stdout(&output)
    );
    assert!(
        tools.step_calls().contains(&"mise prune -y".to_owned()),
        "the prune must still run: {:?}",
        tools.step_calls()
    );
}
