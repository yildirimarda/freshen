//! Stub-based tests for minikube profiles.
//!
//! Upgrading the `minikube` formula affects the minikube profiles it manages.
//! The runtime lists them with `minikube profile list -o json`, stops the
//! running profiles with `minikube stop -p <name>` before the `minikube`
//! formula is upgraded, and starts them again with `minikube start -p <name>`
//! afterwards. Profiles that were already stopped are left alone. No real
//! minikube is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::{
    brew_formula_upgrade, minikube_profile_list, minikube_start, minikube_stop, StubTools,
};

const STEPS: &str = "brew,brew-formulae,colima-stop,colima-start";

const MINIKUBE_PROFILES: &str =
    r#"[{"name":"minikube","status":"Running"},{"name":"work","status":"Running"}]"#;

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

fn install_minikube(tools: &StubTools) {
    tools.set_script(
        "minikube",
        r#"case "$*" in
  "profile list -o json")
    printf '%s' "$FRESHEN_MINIKUBE_PROFILES"
    ;;
  "stop -p "*)
    exit "${FRESHEN_MINIKUBE_STOP_STATUS:-0}"
    ;;
  "start -p "*)
    exit "${FRESHEN_MINIKUBE_START_STATUS:-0}"
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
        .env("FRESHEN_OUTDATED_FORMULAE", "minikube\n")
        .env("FRESHEN_MINIKUBE_PROFILES", MINIKUBE_PROFILES)
        .args(["--only", STEPS, "--container-runtime", "minikube"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn stops_minikube_profiles_around_the_minikube_upgrade() {
    let tools = StubTools::new(&["brew", "minikube"]);
    install_brew(&tools);
    install_minikube(&tools);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let list = index_of(&calls, minikube_profile_list()[0]);
    let stop_minikube = index_of(&calls, minikube_stop("minikube").as_str());
    let stop_work = index_of(&calls, minikube_stop("work").as_str());
    let upgrade = index_of(&calls, brew_formula_upgrade()[4]);
    let start_minikube = index_of(&calls, minikube_start("minikube").as_str());
    let start_work = index_of(&calls, minikube_start("work").as_str());

    assert!(list < stop_minikube, "calls: {calls:?}");
    assert!(stop_minikube < upgrade, "calls: {calls:?}");
    assert!(upgrade < start_minikube, "calls: {calls:?}");
    assert!(stop_minikube < stop_work, "calls: {calls:?}");
    assert!(start_minikube < start_work, "calls: {calls:?}");

    let text = stdout(&output);
    assert!(text.contains("minikube stop: minikube"), "output: {text}");
    assert!(text.contains("minikube stop: work"), "output: {text}");
    assert!(text.contains("minikube start: minikube"), "output: {text}");
    assert!(text.contains("minikube start: work"), "output: {text}");
}

#[test]
fn no_minikube_profiles_means_no_stop_or_start() {
    let tools = StubTools::new(&["brew", "minikube"]);
    install_brew(&tools);
    install_minikube(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "minikube\n")
        .env("FRESHEN_MINIKUBE_PROFILES", "[]")
        .args(["--only", STEPS, "--container-runtime", "minikube"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&minikube_profile_list()[0].to_string()),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("minikube stop")),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("minikube start")),
        "calls: {calls:?}"
    );
}

#[test]
fn a_failing_minikube_start_is_recorded() {
    let tools = StubTools::new(&["brew", "minikube"]);
    install_brew(&tools);
    install_minikube(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "minikube\n")
        .env("FRESHEN_MINIKUBE_PROFILES", MINIKUBE_PROFILES)
        .env("FRESHEN_MINIKUBE_START_STATUS", "1")
        .args(["--only", STEPS, "--container-runtime", "minikube"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&minikube_start("minikube")),
        "calls: {calls:?}"
    );
    let text = stdout(&output);
    assert!(
        text.contains("warning: minikube did not start; containers were NOT restarted"),
        "output: {text}"
    );
    assert!(text.contains("1 failed"), "summary: {text}");
}
