//! Stub-based tests for kind clusters.
//!
//! Upgrading the `kind` formula affects the kind clusters it manages. The runtime
//! lists them with `kind get clusters`, stops the running clusters with
//! `kind cluster stop` before the `kind` formula is upgraded, and starts them
//! again with `kind cluster start` afterwards. Clusters that were already
//! stopped are left alone. No real kind is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use support::{
    brew_formula_upgrade, kind_cluster_start, kind_cluster_stop, kind_get_clusters, StubTools,
};

const STEPS: &str = "brew,brew-formulae,colima-stop,colima-start";

const KIND_CLUSTERS: &str = "kind-default\nkind-work\n";

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

fn install_kind(tools: &StubTools) {
    tools.set_script(
        "kind",
        r#"case "$*" in
  "get clusters")
    printf '%s' "$FRESHEN_KIND_CLUSTERS"
    ;;
  "cluster stop "*)
    exit "${FRESHEN_KIND_STOP_STATUS:-0}"
    ;;
  "cluster start "*)
    exit "${FRESHEN_KIND_START_STATUS:-0}"
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
        .env("FRESHEN_OUTDATED_FORMULAE", "kind\n")
        .env("FRESHEN_KIND_CLUSTERS", KIND_CLUSTERS)
        .args(["--only", STEPS, "--container-runtime", "kind"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn stops_kind_clusters_around_the_kind_upgrade() {
    let tools = StubTools::new(&["brew", "kind"]);
    install_brew(&tools);
    install_kind(&tools);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    let list = index_of(&calls, kind_get_clusters()[0]);
    let stop_default = index_of(&calls, kind_cluster_stop("kind-default").as_str());
    let stop_work = index_of(&calls, kind_cluster_stop("kind-work").as_str());
    let upgrade = index_of(&calls, brew_formula_upgrade()[4]);
    let start_default = index_of(&calls, kind_cluster_start("kind-default").as_str());
    let start_work = index_of(&calls, kind_cluster_start("kind-work").as_str());

    assert!(list < stop_default, "calls: {calls:?}");
    assert!(stop_default < upgrade, "calls: {calls:?}");
    assert!(upgrade < start_default, "calls: {calls:?}");
    assert!(stop_default < stop_work, "calls: {calls:?}");
    assert!(start_default < start_work, "calls: {calls:?}");

    let text = stdout(&output);
    assert!(
        text.contains("kind cluster stop: kind-default"),
        "output: {text}"
    );
    assert!(
        text.contains("kind cluster stop: kind-work"),
        "output: {text}"
    );
    assert!(
        text.contains("kind cluster start: kind-default"),
        "output: {text}"
    );
    assert!(
        text.contains("kind cluster start: kind-work"),
        "output: {text}"
    );
}

#[test]
fn no_kind_clusters_means_no_stop_or_start() {
    let tools = StubTools::new(&["brew", "kind"]);
    install_brew(&tools);
    install_kind(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "kind\n")
        .env("FRESHEN_KIND_CLUSTERS", "")
        .args(["--only", STEPS, "--container-runtime", "kind"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&kind_get_clusters()[0].to_string()),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("kind cluster stop")),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("kind cluster start")),
        "calls: {calls:?}"
    );
}

#[test]
fn a_failing_kind_start_is_recorded() {
    let tools = StubTools::new(&["brew", "kind"]);
    install_brew(&tools);
    install_kind(&tools);

    let output = tools
        .command()
        .env("FRESHEN_OUTDATED_FORMULAE", "kind\n")
        .env("FRESHEN_KIND_CLUSTERS", KIND_CLUSTERS)
        .env("FRESHEN_KIND_START_STATUS", "1")
        .args(["--only", STEPS, "--container-runtime", "kind"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.contains(&kind_cluster_start("kind-default")),
        "calls: {calls:?}"
    );
    let text = stdout(&output);
    assert!(
        text.contains("warning: kind did not start; containers were NOT restarted"),
        "output: {text}"
    );
    assert!(text.contains("1 failed"), "summary: {text}");
}
