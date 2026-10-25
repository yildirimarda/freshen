//! End-to-end checks for the per-step flags and the include/exclude lists.
//!
//! Every test runs the built binary against stub package managers with the
//! home and configuration directories pointing into the temporary directory, so
//! no test reads the configuration file of the machine it runs on.

#![cfg(feature = "test-harness")]

mod support;

use std::fs;
use std::path::Path;
use std::process::Output;

use support::StubTools;

// The constants below are only read by the tests that drive the Unix tools,
// so they are not compiled on Windows.
#[cfg(unix)]
const CASK_INFO: &str = include_str!("../../freshen-lib/tests/fixtures/brew-info-casks.json");
#[cfg(unix)]
const HOMEBREW_STEPS: &str =
    "brew,brew-formulae,brew-services,brew-casks,brew-autoremove,brew-cleanup,brew-reopen";
#[cfg(unix)]
const CONTAINER_STEPS: &str = "brew,brew-formulae,brew-services,brew-casks,brew-autoremove,brew-cleanup,brew-reopen,colima-stop,colima-start,docker";

#[cfg(unix)]
const COMPOSE_JSON: &str = r#"[
    {"Name":"web","ConfigFiles":"/srv/web/compose.yml,/srv/web/override.yml"},
    {"Name":"db","ConfigFiles":"/srv/db/compose.yml"}
]"#;

#[cfg(unix)]
const PS_OUTPUT: &str = "abc\tweb-1\tweb\ndef\tlonely\t\nghi\tlonely2\t\n";

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn write_config(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", parent.display()));
    }
    fs::write(path, contents)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

/// A stub `brew` that reports the given outdated formulae and casks.
#[cfg(unix)]
fn install_brew(tools: &StubTools, formulae: &str, casks: &str) {
    tools.set_script(
        "brew",
        &format!(
            r#"case "$*" in
  "outdated --formula")
    printf '%s' '{formulae}'
    ;;
  "outdated --formula --quiet")
    printf '%s' '{formulae}'
    ;;
  "outdated --cask --greedy")
    printf '%s' '{casks}'
    ;;
  "outdated --cask --greedy --quiet")
    printf '%s' '{casks}'
    ;;
  "info --cask --json=v2 "*)
    printf '%s' '{cask_info}'
    ;;
esac"#,
            formulae = formulae,
            casks = casks,
            cask_info = CASK_INFO,
        ),
    );
}

/// A stub `docker` that reports the given compose projects and containers.
#[cfg(unix)]
fn install_docker(tools: &StubTools) {
    tools.set_script(
        "docker",
        r#"case "$*" in
  "compose ls --format json")
    printf '%s' "$FRESHEN_COMPOSE_JSON"
    ;;
  "ps --format "*)
    printf '%s' "$FRESHEN_PS_OUTPUT"
    ;;
esac"#,
    );
}

/// A stub `colima` that reports a running colima.
#[cfg(unix)]
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

#[test]
fn a_step_disabled_in_the_configuration_file_is_skipped_and_reported() {
    let tools = StubTools::new(&["uv"]);
    write_config(&tools.config_path(), "[steps]\nprobe-0 = false\n");

    let output = tools
        .command()
        .env("FRESHEN_TEST_STEPS", "uv")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.calls().is_empty(), "the disabled step must not run");

    let text = stdout(&output);
    assert!(
        text.contains("disabled in the configuration file"),
        "output: {text}"
    );
    assert!(text.contains("1 skipped"), "output: {text}");
}

#[test]
fn a_step_enabled_in_the_configuration_file_still_runs() {
    let tools = StubTools::new(&["uv"]);
    write_config(&tools.config_path(), "[steps]\nprobe-0 = true\n");

    let output = tools
        .command()
        .env("FRESHEN_TEST_STEPS", "uv")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.calls(), ["uv"]);
    assert!(
        !stdout(&output).contains("disabled in the configuration file"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn a_disabled_step_is_listed_with_its_reason() {
    let tools = StubTools::new(&["uv"]);
    write_config(&tools.config_path(), "[steps]\nprobe-0 = false\n");

    let output = tools
        .command()
        .env("FRESHEN_TEST_STEPS", "uv")
        .arg("--list-steps")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("disabled in the configuration file"),
        "output: {}",
        stdout(&output)
    );
}

#[test]
fn an_unknown_step_in_the_configuration_file_is_an_error() {
    let tools = StubTools::new(&["uv"]);
    write_config(&tools.config_path(), "[steps]\nprobe-9 = false\n");

    let output = tools
        .command()
        .env("FRESHEN_TEST_STEPS", "uv")
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("unknown step `probe-9`"),
        "stderr: {}",
        stderr(&output)
    );
}

#[cfg(unix)]
#[test]
fn the_cask_exclude_list_limits_the_cask_upgrade() {
    let tools = StubTools::new(&["brew"]);
    install_brew(&tools, "", "firefox\ngoogle-chrome\n");
    write_config(
        &tools.config_path(),
        "[exclude]\ncasks = [\"google-chrome\"]\n",
    );

    let output = tools
        .command()
        .args(["--only", HOMEBREW_STEPS])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let mut expected: Vec<String> = Vec::new();
    expected.extend(support::brew_update().iter().map(|&s| s.to_string()));
    expected.extend_from_slice(&[
        "brew outdated --formula".to_string(),
        "brew outdated --formula --quiet".to_string(),
        "brew outdated --cask --greedy".to_string(),
        "brew list --pinned --cask".to_string(),
        "brew info --cask --json=v2 firefox".to_string(),
        "brew outdated --cask --greedy --quiet".to_string(),
        "brew list --pinned --cask".to_string(),
        "brew info --cask --json=v2 firefox".to_string(),
        "brew upgrade --cask firefox".to_string(),
        "brew outdated --cask --greedy".to_string(),
        "brew list --pinned --cask".to_string(),
        "brew info --cask --json=v2 firefox".to_string(),
    ]);
    assert_eq!(tools.step_calls(), expected);
    assert!(
        stdout(&output).contains("Casks to upgrade: firefox"),
        "output: {}",
        stdout(&output)
    );
}

#[cfg(unix)]
#[test]
fn the_cask_include_list_keeps_every_other_cask_out_of_the_upgrade() {
    let tools = StubTools::new(&["brew"]);
    install_brew(&tools, "", "firefox\ngoogle-chrome\n");
    write_config(
        &tools.config_path(),
        "[include]\ncasks = [\"google-chrome\"]\n",
    );

    let output = tools
        .command()
        .args(["--only", HOMEBREW_STEPS])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools
            .step_calls()
            .contains(&"brew upgrade --cask google-chrome".to_owned()),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        !tools
            .step_calls()
            .contains(&"brew upgrade --cask firefox".to_owned()),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[test]
fn the_package_exclude_list_limits_the_uv_tool_upgrade() {
    let tools = StubTools::new(&["uv"]);
    let outdated = "ruff v0.1.0 [latest: 0.2.0]\nblack v24.0.0 [latest: 24.1.0]\n";
    tools.set_responses(
        "uv",
        &[
            // The pre-upgrade snapshot reads the installed tools first.
            support::StubOutput::new(),
            support::StubOutput::stdout(outdated),
            support::StubOutput::stdout(outdated),
            support::StubOutput::stdout(outdated),
        ],
    );
    write_config(&tools.config_path(), "[exclude]\npackages = [\"black\"]\n");

    let output = tools
        .command()
        .args(["--only", "uv"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let calls = tools.step_calls();
    assert!(
        calls.contains(&"uv tool upgrade ruff".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls.contains(&"uv tool upgrade black".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls.contains(&"uv tool upgrade --all".to_owned()),
        "calls: {calls:?}"
    );
}

#[test]
fn the_package_exclude_list_limits_the_mise_tool_upgrade() {
    let tools = StubTools::new(&["brew", "mise"]);
    tools.set_responses("brew", &[support::StubOutput::new().with_status(1)]);
    let outdated = "node    20         20.0.0   20.1.0\npython  3.11       3.11.0   3.11.1\n";
    tools.set_responses(
        "mise",
        &[
            // The snapshot (`mise ls`) consumes the first response.
            support::StubOutput::new(),
            // The `check()` query (`mise outdated`).
            support::StubOutput::new(),
            // The self-update (`mise self-update -y`).
            support::StubOutput::new(),
            // The upgrade query (`mise outdated`) when a filter is set.
            support::StubOutput::stdout(outdated),
            // Per-tool upgrade (`mise upgrade node`).
            support::StubOutput::new(),
            // The prune list (`mise ls --prunable --json`).
            support::StubOutput::stdout("{}"),
            // The prune (`mise prune -y`).
            support::StubOutput::new(),
        ],
    );
    write_config(&tools.config_path(), "[exclude]\npackages = [\"python\"]\n");

    let output = tools
        .command()
        .args(["--only", "mise", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(
        output.status.success(),
        "stderr: {} stdout: {}",
        stderr(&output),
        stdout(&output)
    );

    let calls = tools.step_calls();
    assert!(
        calls.contains(&"mise outdated".to_owned()),
        "calls: {calls:?} stdout: {}",
        stdout(&output)
    );
    assert!(
        calls.contains(&"mise upgrade node".to_owned()),
        "calls: {calls:?} stdout: {} stderr: {}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        !calls.contains(&"mise upgrade python".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls.contains(&"mise upgrade".to_owned()),
        "calls: {calls:?}"
    );
}

#[cfg(unix)]
#[test]
fn an_excluded_cask_is_not_reported_as_outdated() {
    let tools = StubTools::new(&["brew"]);
    install_brew(&tools, "", "firefox\ngoogle-chrome\n");
    write_config(
        &tools.config_path(),
        "[exclude]\ncasks = [\"google-chrome\"]\n",
    );

    let output = tools
        .command()
        .args(["--only", HOMEBREW_STEPS, "--check"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    let text = stdout(&output);
    assert!(text.contains("firefox"), "output: {text}");
    assert!(!text.contains("google-chrome"), "output: {text}");
}

#[cfg(unix)]
#[test]
fn the_package_exclude_list_limits_the_formula_upgrade() {
    let tools = StubTools::new(&["brew"]);
    install_brew(&tools, "jq\nnode\n", "");
    write_config(&tools.config_path(), "[exclude]\npackages = [\"node\"]\n");

    let output = tools
        .command()
        .args(["--only", HOMEBREW_STEPS])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools
            .step_calls()
            .contains(&"brew upgrade --formula jq".to_owned()),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        !tools
            .step_calls()
            .contains(&"brew upgrade --formula node".to_owned()),
        "calls: {:?}",
        tools.step_calls()
    );
}

#[cfg(unix)]
#[test]
fn the_project_exclude_list_keeps_a_project_out_of_the_docker_step() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools, "colima\n", "");
    install_colima(&tools);
    install_docker(&tools);
    write_config(&tools.config_path(), "[exclude]\nprojects = [\"db\"]\n");

    let output = tools
        .command()
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .args(["--only", CONTAINER_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let calls = tools.step_calls();
    assert!(
        calls.iter().any(
            |call| call.starts_with("docker compose -p web ") && call.ends_with("pull --quiet")
        ),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose -p db ")),
        "calls: {calls:?}"
    );
}

#[cfg(unix)]
#[test]
fn the_container_exclude_list_keeps_a_container_running() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_brew(&tools, "colima\n", "");
    install_colima(&tools);
    install_docker(&tools);
    write_config(
        &tools.config_path(),
        "[exclude]\ncontainers = [\"lonely2\"]\n",
    );

    let output = tools
        .command()
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .args(["--only", CONTAINER_STEPS, "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let calls = tools.step_calls();
    assert!(
        calls.contains(&"docker stop def".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls.contains(&"docker stop ghi".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&"docker start def".to_owned()),
        "calls: {calls:?}"
    );
    assert!(
        !calls.contains(&"docker start ghi".to_owned()),
        "calls: {calls:?}"
    );
}
