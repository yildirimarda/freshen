//! Stub-based tests for restarting the desktop applications that were stopped
//! before an upgrade, reusing the recorded snapshot of what was stopped.
//!
//! The apt step stops running desktop applications and records them. With
//! `--restart-apps`, freshen restarts those applications with `gtk-launch`
//! after the upgrade. No real package manager or desktop application is
//! touched; `gtk-launch` is a stub.
//!
//! The stopped application is reaped by the test process while freshen runs,
//! so that freshen's stop policy observes it as stopped instead of waiting out
//! the stop timeout.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use support::{retry_text_file_busy, StubOutput, StubTools};

/// A unique id for the `sleep` process each test starts, so parallel tests do
/// not collide when matching processes by executable name in `/proc`.
static SLEEP_ID: AtomicU64 = AtomicU64::new(0);

/// Locates the system `sleep` binary so it can be copied under a unique name.
fn locate_sleep_binary() -> PathBuf {
    if let Ok(path) = env::var("PATH") {
        for dir in env::split_paths(&path) {
            let candidate = dir.join("sleep");
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
    }
    PathBuf::from("/usr/bin/sleep")
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Unix-only fixture: the `gtk-launch` stub. `set_script` writes an executable
/// shell script (using `chmod`/`PermissionsExt`), so this helper and the
/// import it needs live in this `#[cfg(unix)]` submodule.
#[cfg(unix)]
mod unix {
    use super::StubTools;

    /// A `gtk-launch` stub that succeeds and detaches the application.
    pub(super) fn install_gtk_launch(tools: &StubTools) {
        tools.set_script("gtk-launch", "exit 0\n");
    }
}

/// Spawns a desktop application to be stopped and writes its `.desktop` file.
///
/// Returns the application process, which the caller reaps while freshen runs
/// (see [`run_with_stopped_app`]) so freshen observes it as stopped.
fn stop_a_desktop_application(tools: &StubTools) -> std::process::Child {
    // Copy the system `sleep` binary under a unique name so the process table
    // entry used to match the desktop application does not collide with other
    // tests running in parallel.
    let id = SLEEP_ID.fetch_add(1, Ordering::Relaxed);
    let unique_name = format!("freshen-sleep-{id}");
    let sleep_path = tools.home().join(unique_name);
    fs::copy(locate_sleep_binary(), &sleep_path)
        .unwrap_or_else(|error| panic!("failed to copy sleep binary to {sleep_path:?}: {error}"));
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&sleep_path, fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("failed to make {sleep_path:?} executable: {error}"));

    let sleep = retry_text_file_busy(|| Command::new(&sleep_path).arg("30").spawn())
        .unwrap_or_else(|error| panic!("failed to spawn {sleep_path:?}: {error}"));
    thread::sleep(Duration::from_millis(100));

    let desktop_dir = tools.home().join("desktop-applications");
    std::fs::create_dir_all(&desktop_dir)
        .unwrap_or_else(|error| panic!("failed to create {desktop_dir:?}: {error}"));
    std::fs::write(
        desktop_dir.join("sleep.desktop"),
        format!("Desktop Entry\nName=Sleep\nExec={}\n", sleep_path.display()),
    )
    .unwrap_or_else(|error| panic!("failed to write desktop file: {error}"));

    sleep
}

/// Runs freshen with `args` while reaping `sleep` whenever it exits, so that
/// freshen's stop policy observes the application as stopped instead of
/// waiting out the stop timeout.
///
/// freshen runs on a thread so the test process can reap the stopped
/// application; `std::process::Command::output` would block the test process
/// and leave `sleep` as a zombie that `is_alive` still reports as running.
fn run_with_stopped_app(
    tools: &StubTools,
    mut sleep: std::process::Child,
    args: &[&str],
) -> Output {
    let mut command = tools.command();
    command.args(args);
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let output = command.output();
        let _ = tx.send(output);
    });

    let mut output = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // Reap the application if freshen has stopped it, so `is_alive`
        // reports it as stopped on the next poll.
        let _ = sleep.try_wait();
        match rx.try_recv() {
            Ok(out) => {
                output = Some(out.unwrap_or_else(|error| panic!("freshen failed to run: {error}")));
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {
                if Instant::now() >= deadline {
                    panic!("freshen did not finish in time");
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(mpsc::TryRecvError::Disconnected) => break,
        }
    }

    let output = output.unwrap_or_else(|| panic!("freshen did not produce output"));

    // Reap the application if it is still running (for example when it was
    // never stopped), so it does not outlive the test.
    let _ = sleep.kill();
    let _ = sleep.wait();

    let _ = thread.join();
    output
}

#[test]
fn restart_apps_restarts_the_stopped_desktop_applications() {
    let tools = StubTools::new(&["apt-get", "gtk-launch"]);
    unix::install_gtk_launch(&tools);
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check, up to date)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
            StubOutput::new(), // apt-get autoremove -y
            StubOutput::new(), // apt-get autoclean
        ],
    );
    let sleep = stop_a_desktop_application(&tools);

    let output = run_with_stopped_app(
        &tools,
        sleep,
        &["--only", "apt", "--restart-apps", "--json"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.calls();
    assert!(
        calls.iter().any(|call| call == "gtk-launch sleep.desktop"),
        "gtk-launch sleep.desktop was not called: {calls:?}",
    );

    let report: serde_json::Value = serde_json::from_str(&stdout(&output))
        .unwrap_or_else(|error| panic!("invalid JSON report: {error}: {}", stdout(&output)));
    assert_eq!(
        report["restarted_apps"],
        serde_json::json!(["Sleep"]),
        "report: {}",
        stdout(&output)
    );
}

#[test]
fn restart_apps_does_not_launch_without_the_flag() {
    let tools = StubTools::new(&["apt-get", "gtk-launch"]);
    unix::install_gtk_launch(&tools);
    tools.set_responses(
        "apt-get",
        &[
            StubOutput::new(), // apt-get -s upgrade (check, up to date)
            StubOutput::new(), // apt-get update
            StubOutput::new(), // apt-get upgrade -y
            StubOutput::new(), // apt-get autoremove -y
            StubOutput::new(), // apt-get autoclean
        ],
    );
    let sleep = stop_a_desktop_application(&tools);

    let output = run_with_stopped_app(&tools, sleep, &["--only", "apt", "--json"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call.starts_with("gtk-launch")),
        "gtk-launch must not be called without --restart-apps: {:?}",
        tools.calls()
    );
    let report: serde_json::Value = serde_json::from_str(&stdout(&output))
        .unwrap_or_else(|error| panic!("invalid JSON report: {error}"));
    assert_eq!(
        report["restarted_apps"].as_array().map(|array| array.len()),
        Some(0)
    );
}
