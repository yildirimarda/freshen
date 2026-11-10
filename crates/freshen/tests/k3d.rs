//! Stub-based tests for k3d clusters.
//!
//! Upgrading the `k3d` formula affects the k3d clusters it manages. The runtime
//! lists them with `k3d cluster list -o json`, stops the running clusters with
//! `k3d cluster stop` before the `k3d` formula is upgraded, and starts them
//! again with `k3d cluster start` afterwards. Clusters that were already
//! stopped are left alone. No real k3d is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::{
    brew_formula_upgrade, k3d_cluster_list_json, k3d_cluster_start, k3d_cluster_stop, StubTools,
};

const STEPS: &str = "brew,brew-formulae,colima-stop,colima-start";

const K3D_CLUSTERS: &str =
    r#"[{"name":"k3d-default","state":"running"},{"name":"k3d-work","state":"stopped"}]"#;

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

fn install_k3d(tools: &StubTools) {
    tools.set_script(
        "k3d",
        r#"case "$*" in
  "cluster list -o json")
    printf '%s' "$FRESHEN_K3D_CLUSTERS"
    ;;
  "cluster stop "*)
    exit "${FRESHEN_K3D_STOP_STATUS:-0}"
    ;;
  "cluster start "*)
    exit "${FRESHEN_K3D_START_STATUS:-0}"
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
        .env("FRESHEN_OUTDATED_FORMULAE", "k3d\n")
        .env("FRESHEN_K3D_CLUSTERS", K3D_CLUSTERS)
        .args(["--only", STEPS, "--container-runtime", "k3d"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn stops_k3d_clusters_around_the_k3d_upgrade() {
    let tools = StubTools::new(&["brew", "k3d"]);
    install_brew(&tools);
    install_k3d(&tools);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let list = index_of(&calls, k3d_cluster_list_json()[0]);
    let stop = index_of(&calls, k3d_cluster_stop("k3d-default").as_str());
    let upgrade = index_of(&calls, brew_formula_upgrade()[4]);
    let start = index_of(&calls, k3d_cluster_start("k3d-default").as_str());

    assert!(list < stop, "calls: {calls:?}");
    assert!(stop < upgrade, "calls: {calls:?}");
    assert!(upgrade < start, "calls: {calls:?}");

    // The cluster that was already stopped must not be started again.
    assert!(
        !calls
            .iter()
            .any(|call| call == &k3d_cluster_start("k3d-work")),
        "calls: {calls:?}"
    );

    let text = stdout(&output);
    assert!(
        text.contains("k3d cluster stop: k3d-default"),
        "output: {text}"
    );
    assert!(
        text.contains("k3d cluster start: k3d-default"),
        "output: {text}"
    );
}

#[test]
fn k3d_clusters_that_were_stopped_are_not_started() {
    let tools = StubTools::new(&["brew", "k3d"]);
    install_brew(&tools);
    install_k3d(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "k3d\n")
        .env(
            "FRESHEN_K3D_CLUSTERS",
            r#"[{"name":"k3d-default","state":"stopped"}]"#,
        )
        .args(["--only", STEPS, "--container-runtime", "k3d"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        index_of(&calls, k3d_cluster_list_json()[0]) < index_of(&calls, brew_formula_upgrade()[4]),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("k3d cluster stop")),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("k3d cluster start")),
        "calls: {calls:?}"
    );
}

#[test]
fn no_k3d_clusters_means_no_stop_or_start() {
    let tools = StubTools::new(&["brew", "k3d"]);
    install_brew(&tools);
    install_k3d(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "k3d\n")
        .env("FRESHEN_K3D_CLUSTERS", "[]")
        .args(["--only", STEPS, "--container-runtime", "k3d"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&k3d_cluster_list_json()[0].to_string()),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("k3d cluster stop")),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("k3d cluster start")),
        "calls: {calls:?}"
    );
}

#[test]
fn a_failing_k3d_start_is_recorded() {
    let tools = StubTools::new(&["brew", "k3d"]);
    install_brew(&tools);
    install_k3d(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "k3d\n")
        .env("FRESHEN_K3D_CLUSTERS", K3D_CLUSTERS)
        .env("FRESHEN_K3D_START_STATUS", "1")
        .args(["--only", STEPS, "--container-runtime", "k3d"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&k3d_cluster_start("k3d-default")),
        "calls: {calls:?}"
    );
    let text = stdout(&output);
    assert!(
        text.contains("warning: k3d did not start; containers were NOT restarted"),
        "output: {text}"
    );
    assert!(text.contains("1 failed"), "summary: {text}");
}
