//! Stub-based tests for standalone Lima instances.
//!
//! Upgrading the `lima` formula also affects Lima VMs that Colima does not
//! manage. The runtime lists them with `limactl list --json`, stops the running
//! standalone instances with `limactl stop`, and starts them again with
//! `limactl start` after the upgrade. Colima-managed instances are left to
//! `colima stop` / `colima start`.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::{StubTools, brew_formula_upgrade, colima_stop, colima_start, limactl_stop, limactl_start, limactl_list_json};

const STEPS: &str = "brew,brew-formulae,colima-stop,colima-start";

const LIMA_INSTANCES: &str = concat!(
    r#"{"name":"colima","status":"Running"}"#,
    "\n",
    r#"{"name":"colima-dev","status":"Running"}"#,
    "\n",
    r#"{"name":"default","status":"Running"}"#,
    "\n",
    r#"{"name":"stopped","status":"Stopped"}"#,
    "\n",
);

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
  "outdated --formula --quiet"|"outdated --formula")
    printf '%s' "$FRESHEN_OUTDATED_FORMULAE"
    ;;
  "upgrade --formula")
    exit "${FRESHEN_BREW_UPGRADE_STATUS:-0}"
    ;;
esac"#,
    );
}

fn install_colima(tools: &StubTools) {
    tools.set_script(
        "colima",
        r#"case "$*" in
  "status")
    exit "${FRESHEN_COLIMA_STATUS:-0}"
    ;;
  "start")
    exit "${FRESHEN_COLIMA_START_STATUS:-0}"
    ;;
esac"#,
    );
}

fn install_limactl(tools: &StubTools) {
    tools.set_script(
        "limactl",
        r#"case "$*" in
  "list --json")
    printf '%s' "$FRESHEN_LIMA_INSTANCES"
    ;;
  "stop "*)
    exit "${FRESHEN_LIMA_STOP_STATUS:-0}"
    ;;
  "start "*)
    exit "${FRESHEN_LIMA_START_STATUS:-0}"
    ;;
esac"#,
    );
}

fn index_of(calls: &[String], call: &str) -> usize {
    calls
        .iter()
        .position(|recorded| recorded == call)
        .unwrap_or_else(|| panic!("missing call `{call}` in {calls:?}"))
}

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "lima\n")
        .env("FRESHEN_LIMA_INSTANCES", LIMA_INSTANCES)
        .args(["--only", STEPS])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn stops_standalone_lima_instances_around_the_lima_upgrade() {
    let tools = StubTools::new(&["brew", "colima", "limactl"]);
    install_brew(&tools);
    install_colima(&tools);
    install_limactl(&tools);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let upgrade = index_of(&calls, brew_formula_upgrade()[4]);
    let stop = index_of(&calls, limactl_stop("default").as_str());
    let start = index_of(&calls, limactl_start("default").as_str());

    assert!(
        index_of(&calls, limactl_list_json()[0]) < stop,
        "calls: {calls:?}"
    );
    assert!(stop < upgrade, "calls: {calls:?}");
    assert!(upgrade < start, "calls: {calls:?}");
    assert!(
        index_of(&calls, colima_stop()[0]) < upgrade,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, colima_start()[0]) > upgrade,
        "calls: {calls:?}"
    );

    for managed in ["colima", "colima-dev", "stopped"] {
        assert!(
            !calls
                .iter()
                .any(|call| call == &format!("limactl stop {managed}")
                    || call == &format!("limactl start {managed}")),
            "calls: {calls:?}"
        );
    }

    let text = stdout(&output);
    assert!(text.contains("limactl stop: default"), "output: {text}");
    assert!(text.contains("limactl start: default"), "output: {text}");
}

#[test]
fn standalone_lima_is_stopped_even_when_colima_is_not_installed() {
    let tools = StubTools::new(&["brew", "limactl"]);
    install_brew(&tools);
    install_limactl(&tools);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let upgrade = index_of(&calls, brew_formula_upgrade()[4]);

    assert!(
        index_of(&calls, limactl_stop("default").as_str()) < upgrade,
        "calls: {calls:?}"
    );
    assert!(
        index_of(&calls, limactl_start("default").as_str()) > upgrade,
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("colima ")),
        "calls: {calls:?}"
    );
}

#[test]
fn colima_managed_instances_are_not_stopped_with_limactl() {
    let tools = StubTools::new(&["brew", "colima", "limactl"]);
    install_brew(&tools);
    install_colima(&tools);
    install_limactl(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "lima\n")
        .env(
            "FRESHEN_LIMA_INSTANCES",
            concat!(
                r#"{"name":"colima","status":"Running"}"#,
                "\n",
                r#"{"name":"colima-dev","status":"Running"}"#,
                "\n",
            ),
        )
        .args(["--only", STEPS])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();

    assert!(
        !calls.iter().any(|call| call.starts_with("limactl stop")),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("limactl start")),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&colima_stop()[0].to_string()),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&colima_start()[0].to_string()),
        "calls: {calls:?}"
    );
}

#[test]
fn a_failing_lima_start_is_recorded() {
    let tools = StubTools::new(&["brew", "colima", "limactl"]);
    install_brew(&tools);
    install_colima(&tools);
    install_limactl(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "lima\n")
        .env("FRESHEN_LIMA_INSTANCES", LIMA_INSTANCES)
        .env("FRESHEN_LIMA_START_STATUS", "1")
        .args(["--only", STEPS])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&limactl_start("default")),
        "calls: {calls:?}"
    );
    assert!(
        stdout(&output).contains("1 failed"),
        "output: {}",
        stdout(&output)
    );
}
