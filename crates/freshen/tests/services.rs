//! Stub-based tests for the `--restart-services` flag.
//!
//! The apt, dnf and zypper steps record the services that need a restart
//! after an upgrade (reported by `needrestart`, `needs-restarting` or
//! `zypper ps -s`). With `--restart-services`, freshen restarts those
//! services after the run, capturing each service's previous enabled/running
//! state first so it can be restored on error or interrupt.
//!
//! No real package manager or system service is touched.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use support::{StubOutput, StubTools};

fn run(tools: &StubTools, args: &[&str]) -> Output {
    tools
        .command()
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn send_signal(child: &Child, signal: &str) {
    let status = Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .unwrap_or_else(|error| panic!("failed to run kill: {error}"));
    assert!(status.success(), "kill {signal} failed with {status}");
}

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

/// Unix-only fixture: the `systemctl` stub. `set_script` writes an executable
/// shell script (using `chmod`/`PermissionsExt`), so this helper and the
/// import it needs live in this `#[cfg(unix)]` submodule.
#[cfg(unix)]
mod unix {
    use super::StubTools;

    /// A `systemctl` stub that reports every service as disabled and
    /// inactive, then succeeds at restarting it.
    pub(super) fn install_enabled_restarting_systemctl(tools: &StubTools) {
        tools.set_script(
            "systemctl",
            "case \"$1\" in\n\
             is-enabled)\n\
               printf 'disabled\\n'\n\
               ;;\n\
             is-active)\n\
               printf 'inactive\\n'\n\
               ;;\n\
             restart)\n\
               exit 0\n\
               ;;\n\
             esac\n\
             exit 0\n",
        );
    }
}

#[test]
fn restart_services_restarts_detected_services() {
    let tools = StubTools::new(&["apt-get", "needrestart", "systemctl"]);
    unix::install_enabled_restarting_systemctl(&tools);
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
        ],
    );
    tools.set_responses(
        "needrestart",
        &[StubOutput::stdout("sshd.service\npostgresql.service\n")],
    );

    let output = run(&tools, &["--only", "apt", "--restart-services", "--json"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.calls();
    assert!(calls
        .iter()
        .any(|call| call == "systemctl is-enabled sshd.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl is-active sshd.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl restart sshd.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl is-enabled postgresql.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl is-active postgresql.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl restart postgresql.service"));

    // The previous state is captured before each service is restarted.
    let sshd_enabled = calls
        .iter()
        .position(|call| call == "systemctl is-enabled sshd.service")
        .unwrap_or_else(|| {
            panic!("missing `systemctl is-enabled sshd.service` in calls: {calls:?}")
        });
    let sshd_active = calls
        .iter()
        .position(|call| call == "systemctl is-active sshd.service")
        .unwrap_or_else(|| {
            panic!("missing `systemctl is-active sshd.service` in calls: {calls:?}")
        });
    let sshd_restart = calls
        .iter()
        .position(|call| call == "systemctl restart sshd.service")
        .unwrap_or_else(|| panic!("missing `systemctl restart sshd.service` in calls: {calls:?}"));
    assert!(
        sshd_enabled < sshd_active && sshd_active < sshd_restart,
        "capture must precede restart: {calls:?}"
    );

    let text = stdout(&output);
    let report: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("invalid JSON report: {error}: {text}"));
    assert_eq!(
        report["restarted_services"],
        serde_json::json!(["sshd.service", "postgresql.service"])
    );
    assert_eq!(report["was_interrupted"], false);
}

#[test]
fn restart_services_does_not_restart_without_the_flag() {
    let tools = StubTools::new(&["apt-get", "needrestart"]);
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
        ],
    );
    tools.set_responses("needrestart", &[StubOutput::stdout("sshd.service\n")]);

    let output = run(&tools, &["--only", "apt"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call.starts_with("systemctl")),
        "systemctl must not be called without --restart-services: {:?}",
        tools.calls()
    );
    assert!(
        !stdout(&output).contains("Restarted services"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn restart_services_restores_previous_state_on_restart_failure() {
    let tools = StubTools::new(&["apt-get", "needrestart", "systemctl"]);
    tools.set_script(
        "systemctl",
        "case \"$1\" in\n\
         is-enabled)\n\
           printf 'enabled\\n'\n\
           ;;\n\
         is-active)\n\
           printf 'active\\n'\n\
           ;;\n\
         restart)\n\
           exit 1\n\
           ;;\n\
         esac\n\
         exit 0\n",
    );
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
        ],
    );
    tools.set_responses("needrestart", &[StubOutput::stdout("sshd.service\n")]);

    let output = run(&tools, &["--only", "apt", "--restart-services"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.calls();
    assert!(calls
        .iter()
        .any(|call| call == "systemctl is-enabled sshd.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl is-active sshd.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl restart sshd.service"));

    // Previous state was enabled and active, so the restore re-enables and
    // starts the service after the restart failed.
    assert!(calls
        .iter()
        .any(|call| call == "systemctl enable sshd.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl start sshd.service"));
}

#[test]
fn restart_services_restores_on_interrupt() {
    let tools = StubTools::new(&["apt-get", "needrestart", "systemctl"]);
    tools.set_script(
        "systemctl",
        "case \"$1\" in\n\
         is-enabled)\n\
           printf 'disabled\\n'\n\
           ;;\n\
         is-active)\n\
           printf 'inactive\\n'\n\
           ;;\n\
         restart)\n\
           /bin/sleep 30\n\
           ;;\n\
         esac\n\
         exit 0\n",
    );
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
        ],
    );
    tools.set_responses("needrestart", &[StubOutput::stdout("sshd.service\n")]);

    let child = tools
        .command()
        .args(["--only", "apt", "--restart-services"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    wait_for_call(&tools, "systemctl restart sshd.service");
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
    let calls = tools.calls();
    assert!(calls
        .iter()
        .any(|call| call == "systemctl is-enabled sshd.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl is-active sshd.service"));

    // Previous state was disabled and inactive, so the restore disables and
    // stops the service after the restart was interrupted.
    assert!(calls
        .iter()
        .any(|call| call == "systemctl disable sshd.service"));
    assert!(calls
        .iter()
        .any(|call| call == "systemctl stop sshd.service"));
}
