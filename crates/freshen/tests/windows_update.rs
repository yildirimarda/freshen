//! End-to-end tests for the Windows Update step.
//!
//! Every test runs the built binary against a stub `pwsh` executable and
//! asserts that the PowerShell invocations (detect the PSWindowsUpdate module,
//! then query `Get-WindowsUpdate`) are issued in order and that the step
//! reports available updates without installing them. No real PowerShell or
//! Windows Update is touched.

#![cfg(all(feature = "test-harness", target_os = "windows"))]

mod support;

use std::process::Output;

use freshen_lib::windows_update::WINDOWS_UPDATE;
use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools) -> Output {
    tools.set_responses(
        "pwsh",
        &[
            StubOutput::stdout("PSWindowsUpdate\n"), // detect: module is installed
            StubOutput::stdout(
                r##"[
  {
    "Title": "Security Update for Windows",
    "KB": "KB5012345",
    "Version": "10.0.19041.1234"
  },
  {
    "Title": "Cumulative Update for Windows",
    "KB": "KB5012346",
    "Version": "10.0.19041.1235"
  }
]"##,
            ), // check: two updates available
        ],
    );
    tools
        .command()
        .args(["--only", WINDOWS_UPDATE])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

/// The commands the Windows Update step issues, in order: detection of the
/// PSWindowsUpdate module and the `Get-WindowsUpdate` query.
const WINDOWS_UPDATE_CALLS: &[&str] = &[
    "pwsh -NoProfile -Command Get-Module -ListAvailable PSWindowsUpdate | Select-Object -ExpandProperty Name",
    "pwsh -NoProfile -Command Import-Module PSWindowsUpdate; Get-WindowsUpdate | Select-Object Title, KB, Version | ConvertTo-Json",
];

#[test]
fn windows_update_reports_available_updates() {
    let tools = StubTools::new(&["pwsh"]);
    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WINDOWS_UPDATE_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(
        text.contains("Security Update for Windows"),
        "update listed: {text}"
    );
    assert!(
        text.contains("Cumulative Update for Windows"),
        "update listed: {text}"
    );
}

#[test]
fn windows_update_reports_none_when_no_updates_available() {
    let tools = StubTools::new(&["pwsh"]);
    tools.set_responses(
        "pwsh",
        &[
            StubOutput::stdout("PSWindowsUpdate\n"), // detect: module is installed
            StubOutput::stdout("[]"),                // check: no updates
        ],
    );
    let output = tools
        .command()
        .args(["--only", WINDOWS_UPDATE])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WINDOWS_UPDATE_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(
        text.contains("no Windows Updates available"),
        "summary: {text}"
    );
}

#[test]
fn windows_update_skipped_when_pswindowsupdate_missing() {
    let tools = StubTools::new(&["pwsh"]);
    tools.set_responses(
        "pwsh",
        &[
            StubOutput::new(), // detect: no module output
        ],
    );
    let output = tools
        .command()
        .args(["--only", WINDOWS_UPDATE])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert_eq!(calls.len(), 1, "only detect runs: {calls:?}");
    assert!(calls[0].contains("ListAvailable"), "detect call: {calls:?}");
    let text = stdout(&output);
    assert!(
        text.contains("not installed"),
        "not installed in summary: {text}"
    );
}

/// `--list-steps` lists the Windows Update step with its detection state.
#[test]
fn windows_update_list_steps_lists_windows_update() {
    let tools = StubTools::new(&["pwsh"]);
    tools.set_responses(
        "pwsh",
        &[StubOutput::stdout("PSWindowsUpdate\n")], // detect: module is installed
    );
    let output = tools
        .command()
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("windows-update"), "listing: {text}");
    assert!(text.contains("installed"), "listing: {text}");
}
