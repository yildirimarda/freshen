//! End-to-end checks for the `schedule install` / `schedule remove`
//! subcommands on macOS and Linux.
//!
//! Scheduling is supported on macOS (launchd agent) and Linux (systemd user
//! timer): the run cases are gated per platform, and every other platform
//! checks that `schedule` reports that scheduling is not supported. The run
//! cases never touch the real system -- they run the binary against stub
//! executables on `PATH`.

#![cfg(feature = "test-harness")]

mod support;

use std::process::Output;

use support::StubTools;

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[cfg(target_os = "macos")]
const AGENT_PLIST: &str = "com.freshen.schedule.plist";

/// The directory the agent property list is written into, relative to home.
#[cfg(target_os = "macos")]
fn plist_dir(home: &std::path::Path) -> std::path::PathBuf {
    home.join("Library").join("LaunchAgents")
}

#[cfg(target_os = "macos")]
fn plist_path(home: &std::path::Path) -> std::path::PathBuf {
    plist_dir(home).join(AGENT_PLIST)
}

#[cfg(target_os = "macos")]
#[test]
fn schedule_install_writes_and_loads_the_agent() {
    let tools = StubTools::new(&["launchctl"]);

    let output = tools
        .command()
        .args(["schedule", "install"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let calls = tools.calls();
    assert!(
        calls.iter().any(|call| call.starts_with("launchctl load ")),
        "expected a launchctl load call, got {calls:?}"
    );

    let home = tools.home();
    let plist = plist_path(home);
    assert!(
        plist.is_file(),
        "agent property list was not written: {plist:?}"
    );

    let content = std::fs::read_to_string(&plist)
        .unwrap_or_else(|error| panic!("failed to read agent property list {plist:?}: {error}"));
    assert!(
        content.contains("com.freshen.schedule"),
        "plist missing label: {content}"
    );
    assert!(
        content.contains("--check"),
        "plist missing --check: {content}"
    );
    assert!(
        content.contains("<key>StartInterval</key>"),
        "plist missing interval: {content}"
    );
    assert!(
        content.contains("<key>RunAtLoad</key>") && content.contains("<false/>"),
        "plist should not run at load: {content}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn schedule_install_runs_every_configured_interval() {
    let tools = StubTools::new(&["launchctl"]);

    let output = tools
        .command()
        .args(["schedule", "install", "--every", "6"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let home = tools.home();
    let content = std::fs::read_to_string(plist_path(home))
        .unwrap_or_else(|error| panic!("failed to read agent property list: {error}"));
    assert!(
        content.contains("<integer>21600</integer>"),
        "plist should run every 6 hours: {content}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn schedule_remove_unloads_and_deletes_the_agent() {
    let tools = StubTools::new(&["launchctl"]);

    let install_output = tools
        .command()
        .args(["schedule", "install"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(install_output.status.success());

    let home = tools.home();
    let plist = plist_path(home);
    assert!(plist.is_file());

    let output = tools
        .command()
        .args(["schedule", "remove"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let calls = tools.calls();
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("launchctl unload ")),
        "expected a launchctl unload call, got {calls:?}"
    );
    assert!(
        !plist.is_file(),
        "agent property list was not removed: {plist:?}"
    );
}

#[cfg(target_os = "linux")]
const TIMER_UNIT: &str = "com.freshen.schedule.timer";

/// The directory the timer unit is written into, relative to home.
#[cfg(target_os = "linux")]
fn timer_dir(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".config").join("systemd").join("user")
}

/// The path of the timer unit on Linux.
#[cfg(target_os = "linux")]
fn timer_path(home: &std::path::Path) -> std::path::PathBuf {
    timer_dir(home).join(TIMER_UNIT)
}

/// The path of the service unit the timer runs on Linux.
#[cfg(target_os = "linux")]
fn service_path(home: &std::path::Path) -> std::path::PathBuf {
    timer_dir(home).join("com.freshen.schedule.service")
}

#[cfg(target_os = "linux")]
#[test]
fn schedule_install_writes_and_starts_the_timer() {
    let tools = StubTools::new(&["systemctl"]);

    let output = tools
        .command()
        .args(["schedule", "install"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let calls = tools.calls();
    assert!(
        calls
            .iter()
            .any(|call| call == "systemctl --user daemon-reload"),
        "expected a systemctl --user daemon-reload call, got {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|call| { call == "systemctl --user enable --now com.freshen.schedule.timer" }),
        "expected a systemctl --user enable --now call, got {calls:?}"
    );

    let home = tools.home();
    let timer = timer_path(home);
    assert!(timer.is_file(), "timer unit was not written: {timer:?}");

    let service = service_path(home);
    assert!(
        service.is_file(),
        "service unit was not written: {service:?}"
    );

    let timer_content = std::fs::read_to_string(&timer)
        .unwrap_or_else(|error| panic!("failed to read timer unit {timer:?}: {error}"));
    assert!(
        timer_content.contains("com.freshen.schedule"),
        "timer missing label: {timer_content}"
    );
    assert!(
        timer_content.contains("OnUnitActiveSec"),
        "timer missing interval: {timer_content}"
    );
    assert!(
        timer_content.contains("WantedBy=timers.target"),
        "timer missing install section: {timer_content}"
    );

    let service_content = std::fs::read_to_string(&service)
        .unwrap_or_else(|error| panic!("failed to read service unit {service:?}: {error}"));
    assert!(
        service_content.contains("--check"),
        "service missing --check: {service_content}"
    );
    assert!(
        service_content.contains("Type=oneshot"),
        "service missing Type=oneshot: {service_content}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn schedule_install_runs_every_configured_interval() {
    let tools = StubTools::new(&["systemctl"]);

    let output = tools
        .command()
        .args(["schedule", "install", "--every", "6"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let home = tools.home();
    let content = std::fs::read_to_string(timer_path(home))
        .unwrap_or_else(|error| panic!("failed to read timer unit: {error}"));
    assert!(
        content.contains("OnUnitActiveSec=21600"),
        "timer should run every 6 hours: {content}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn schedule_remove_stops_and_deletes_the_timer() {
    let tools = StubTools::new(&["systemctl"]);

    let install_output = tools
        .command()
        .args(["schedule", "install"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(install_output.status.success());

    let home = tools.home();
    let timer = timer_path(home);
    assert!(timer.is_file());

    let output = tools
        .command()
        .args(["schedule", "remove"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let calls = tools.calls();
    assert!(
        calls
            .iter()
            .any(|call| { call == "systemctl --user disable --now com.freshen.schedule.timer" }),
        "expected a systemctl --user disable --now call, got {calls:?}"
    );
    assert!(!timer.is_file(), "timer unit was not removed: {timer:?}");
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[test]
fn schedule_reports_not_supported_on_other_platforms() {
    let tools = StubTools::new(&[]);

    let output = tools
        .command()
        .args(["schedule", "install"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("scheduling is not supported"),
        "stderr: {text}"
    );

    let remove_output = tools
        .command()
        .args(["schedule", "remove"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(!remove_output.status.success());
    let text = stderr(&remove_output);
    assert!(
        text.contains("scheduling is not supported"),
        "stderr: {text}"
    );
}

#[cfg(test)]
mod cli_parsing {
    use super::stderr;
    use crate::support::StubTools;

    #[test]
    fn schedule_install_parses_with_default_interval() {
        let tools = StubTools::new(&[]);
        let output = tools
            .command()
            .args(["schedule", "install"])
            .output()
            .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
        // On non-macOS this fails with "not supported"; on macOS it would
        // install. Either way the subcommand parses and runs past argument
        // parsing, so the exit is not an argument error.
        let text = stderr(&output);
        assert!(
            !text.contains("error:") && !text.contains("usage:"),
            "schedule install should parse: {text}"
        );
    }

    #[test]
    fn schedule_remove_parses() {
        let tools = StubTools::new(&[]);
        let output = tools
            .command()
            .args(["schedule", "remove"])
            .output()
            .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
        let text = stderr(&output);
        assert!(
            !text.contains("error:") && !text.contains("usage:"),
            "schedule remove should parse: {text}"
        );
    }
}
