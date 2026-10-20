//! Interrupt handling: stop the running step, restore what was stopped and
//! print a partial summary.
//!
//! Each test starts the real binary against stubs, waits until a deliberately
//! slow step is running, then sends a real signal. No package manager or
//! container runtime is touched.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use support::StubTools;

const COMPOSE_JSON: &str = r#"[{"Name":"web","ConfigFiles":"/srv/web/compose.yml"}]"#;
const PS_OUTPUT: &str = "abc\tweb-1\tweb\ndef\tlonely\t\n";

fn wait_for_call(tools: &StubTools, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if tools.calls().iter().any(|call| call.contains(needle)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for `{needle}`; calls: {:?}",
            tools.calls()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn send_signal(child: &Child, signal: &str) {
    let status = Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .unwrap_or_else(|error| panic!("failed to run kill: {error}"));
    assert!(status.success(), "kill {signal} failed with {status}");
}

fn spawn(tools: &StubTools, steps: &str, args: &[&str]) -> Child {
    tools
        .command()
        .env("FRESHEN_TEST_STEPS", steps)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn install_container_stubs(tools: &StubTools) {
    tools.set_script(
        "brew",
        r#"case "$*" in
  "outdated --formula --quiet")
    printf 'colima\n'
    ;;
  "upgrade --formula")
    /bin/sleep 30
    ;;
esac"#,
    );
    tools.set_script(
        "colima",
        r#"case "$*" in
  "status")
    exit 0
    ;;
esac"#,
    );
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

#[test]
fn sigint_stops_the_running_step_and_prints_a_partial_summary() {
    let tools = StubTools::new(&["slow", "fast"]);
    tools.set_script("slow", "/bin/sleep 30");

    let child = spawn(&tools, "slow;fast", &[]);
    wait_for_call(&tools, "slow");
    send_signal(&child, "-INT");

    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("failed to wait for freshen: {error}"));

    assert_eq!(
        output.status.code(),
        Some(130),
        "stderr: {}",
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("Summary:"), "summary: {text}");
    assert!(text.contains("interrupted"), "summary: {text}");
    assert!(text.contains("slow"), "summary: {text}");
    assert!(
        !tools.calls().iter().any(|call| call == "fast"),
        "the next step must not start: {:?}",
        tools.calls()
    );
}

#[test]
fn sigterm_stops_the_running_step_too() {
    let tools = StubTools::new(&["slow"]);
    tools.set_script("slow", "/bin/sleep 30");

    let child = spawn(&tools, "slow", &[]);
    wait_for_call(&tools, "slow");
    send_signal(&child, "-TERM");

    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("failed to wait for freshen: {error}"));

    assert_eq!(
        output.status.code(),
        Some(130),
        "stderr: {}",
        stderr(&output)
    );
    assert!(stdout(&output).contains("interrupted"));
}

#[test]
fn an_interrupt_restores_colima_and_the_containers_it_stopped() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_container_stubs(&tools);

    let child = tools
        .command()
        .env("FRESHEN_COMPOSE_JSON", COMPOSE_JSON)
        .env("FRESHEN_PS_OUTPUT", PS_OUTPUT)
        .args(["--only", "brew,brew-formulae,colima-stop,colima-start"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    wait_for_call(&tools, "brew upgrade --formula");
    send_signal(&child, "-INT");

    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("failed to wait for freshen: {error}"));

    assert_eq!(
        output.status.code(),
        Some(130),
        "stderr: {}",
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("interrupted"), "summary: {text}");
    assert!(
        text.contains("Restoring colima and containers"),
        "restore output: {text}"
    );

    let calls = tools.calls();
    let position = |needle: &str| {
        calls
            .iter()
            .position(|call| call.contains(needle))
            .unwrap_or_else(|| panic!("missing `{needle}` in calls: {calls:?}"))
    };

    let upgrade = position("brew upgrade --formula");
    assert!(position("colima stop") < upgrade);
    assert!(position("docker stop def") < upgrade);
    assert!(position("colima start") > upgrade);
    assert!(position("docker start def") > upgrade);
    assert!(position("docker compose -p web -f /srv/web/compose.yml up -d") > upgrade);
}
