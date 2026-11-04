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

/// Detection of the PSWindowsUpdate module.
const WINDOWS_UPDATE_DETECT: &str = "pwsh -NoProfile -Command Get-Module -ListAvailable PSWindowsUpdate | Select-Object -ExpandProperty Name";

/// The `Get-WindowsUpdate` query.
const WINDOWS_UPDATE_CHECK: &str = "pwsh -NoProfile -Command Import-Module PSWindowsUpdate; Get-WindowsUpdate | Select-Object Title, KB, Version | ConvertTo-Json";

/// The PowerShell invocations of the run, without the quotes the Windows `.cmd`
/// stub logs around an argument that contains spaces.
fn pwsh_calls(tools: &StubTools) -> Vec<String> {
    tools
        .step_calls()
        .iter()
        .map(|call| call.replace('"', ""))
        .collect()
}

/// Detection comes first, then exactly one `Get-WindowsUpdate` query: the
/// runner checks before the step and the step reports through a check of its
/// own, but both reuse the cached listing, so a run costs one slow
/// PSWindowsUpdate scan. Nothing is ever installed.
fn assert_windows_update_calls(calls: &[String]) {
    assert_eq!(
        calls.first().map(String::as_str),
        Some(WINDOWS_UPDATE_DETECT),
        "calls: {calls:?}"
    );
    assert_eq!(
        calls.len(),
        2,
        "expected detect + one Get-WindowsUpdate call, got {calls:?}"
    );
    assert_eq!(calls[1], WINDOWS_UPDATE_CHECK, "calls: {calls:?}");
}

#[test]
fn windows_update_reports_available_updates() {
    let tools = StubTools::new(&["pwsh"]);
    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_windows_update_calls(&pwsh_calls(&tools));
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
    assert_windows_update_calls(&pwsh_calls(&tools));
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
