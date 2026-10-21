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
  "ls --prunable --json")
    printf '{}'
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
        .args(["--only", steps, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn uv_upgrades_tools_and_prunes_the_cache() {
    let tools = StubTools::new(&["uv"]);
    install_uv(&tools);

    let output = run(&tools, "uv");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "uv tool list --outdated",
            "uv tool upgrade --all",
            "uv cache prune"
        ]
    );
}

#[test]
fn a_failing_uv_upgrade_still_prunes_the_cache_and_fails_the_step() {
    let tools = StubTools::new(&["uv"]);
    install_uv(&tools);

    let output = tools
        .command()
        .env("FRESHEN_UV_UPGRADE_STATUS", "1")
        .args(["--only", "uv", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "uv tool list --outdated",
            "uv tool upgrade --all",
            "uv cache prune"
        ]
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
    install_brew(&tools);
    install_mise(&tools);

    let output = run(&tools, "mise");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "mise outdated",
            "brew list --formula mise",
            "mise self-update -y",
            "mise upgrade",
            "mise ls --prunable --json",
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
        .args(["--only", "mise", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "mise outdated",
            "brew list --formula mise",
            "mise upgrade",
            "mise ls --prunable --json",
            "mise prune -y",
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
        tools.step_calls(),
        [
            "mise outdated",
            "mise self-update -y",
            "mise upgrade",
            "mise ls --prunable --json",
            "mise prune -y"
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
        .args(["--only", "mise", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "mise outdated",
            "mise self-update -y",
            "mise upgrade",
            "mise ls --prunable --json",
            "mise prune -y"
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
        .args(["--only", "mise", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        [
            "mise outdated",
            "mise self-update -y",
            "mise upgrade",
            "mise ls --prunable --json",
            "mise prune -y"
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
    assert!(tools.step_calls().is_empty());
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
    assert_eq!(
        tools.step_calls(),
        [
            "mise outdated",
            "mise self-update -y",
            "mise upgrade",
            "mise ls --prunable --json",
            "mise prune -y",
            "ps -axo pid=,ppid=,command=",
            "ps -o comm= -p 1",
        ]
    );
}

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
    tools.set_script(
        "uv",
        r#"case "$*" in
  "tool list --outdated")
    printf 'black v23.1.0 [latest: 26.10.1]\n- black\n- blackd\nruff v0.6.0 [latest: 0.7.0]\n- ruff\n'
    ;;
esac"#,
    );

    let output = tools
        .command()
        .args(["--check", "--only", "uv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), ["uv tool list --outdated"]);
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
    tools.set_script(
        "mise",
        r#"case "$*" in
  "outdated")
    printf 'Plugin  Requested  Current  Latest\n'
    printf 'node    20         20.0.0   20.1.0\n'
    printf 'python  3.11       3.11.0   3.11.1\n'
    ;;
esac"#,
    );

    let output = tools
        .command()
        .args(["--check", "--only", "mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), ["mise outdated"]);
    let text = stdout(&output);
    assert!(text.contains("mise:"), "output: {text}");
    assert!(text.contains("node 20.0.0 -> 20.1.0"), "output: {text}");
    assert!(text.contains("python 3.11.0 -> 3.11.1"), "output: {text}");
    assert!(
        text.contains("2 update(s) pending across 1 step(s)."),
        "output: {text}"
    );
}

#[test]
fn check_reports_uv_and_mise_up_to_date_without_running_them() {
    let tools = StubTools::new(&["uv", "mise"]);
    tools.set_script(
        "uv",
        r#"case "$*" in
  "tool list --outdated")
    printf 'No tools installed\n'
    ;;
esac"#,
    );
    tools.set_script(
        "mise",
        r#"case "$*" in
  "outdated")
    printf 'All tools are up to date\n' >&2
    ;;
esac"#,
    );

    let output = tools
        .command()
        .args(["--check", "--only", "uv,mise"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        ["uv tool list --outdated", "mise outdated"]
    );
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn check_reports_a_failed_tool_query_with_exit_1() {
    let tools = StubTools::new(&["uv"]);
    tools.set_script(
        "uv",
        r#"case "$*" in
  "tool list --outdated")
    exit 1
    ;;
esac"#,
    );

    let output = tools
        .command()
        .args(["--check", "--only", "uv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), ["uv tool list --outdated"]);
    assert!(
        stdout(&output).contains("failed during check"),
        "output: {}",
        stdout(&output)
    );
}
