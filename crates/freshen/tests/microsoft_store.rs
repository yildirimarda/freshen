//! End-to-end tests for the Microsoft Store step.
//!
//! Every test runs the built binary against a stub `winget` executable and
//! asserts that the upgrade exit codes (`0`, the update-not-applicable code
//! and other non-zero codes) map to ok, skipped and failed respectively. No
//! real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "windows"))]

mod support;

use std::process::Output;

use freshen_lib::microsoft_store::EXIT_UPDATE_NOT_APPLICABLE;
use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools, upgrade_exit: i32) -> Output {
    tools.set_responses(
        "winget",
        &[
            StubOutput::new(),                           // detect: winget --version
            StubOutput::new(),                           // check: winget upgrade --source msstore
            StubOutput::new(),                           // run: winget source update
            StubOutput::new().with_status(upgrade_exit), // run: winget upgrade --all --source msstore ...
        ],
    );
    tools
        .command()
        .args(["--only", "microsoft-store"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

/// The commands the Microsoft Store step issues, in order: detection, the
/// read-only check, the source refresh and the upgrade itself.
const MICROSOFT_STORE_CALLS: &[&str] = &[
    "winget --version",
    "winget --disable-interactivity upgrade --source msstore",
    "winget --disable-interactivity source update",
    "winget --disable-interactivity upgrade --all --source msstore --silent --accept-package-agreements --accept-source-agreements",
];

#[test]
fn microsoft_store_reports_ok_when_the_upgrade_succeeds() {
    let tools = StubTools::new(&["winget"]);
    let output = run(&tools, 0);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        MICROSOFT_STORE_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("1 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn microsoft_store_reports_skipped_when_no_upgrades_are_applicable() {
    let tools = StubTools::new(&["winget"]);
    let output = run(&tools, EXIT_UPDATE_NOT_APPLICABLE);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        MICROSOFT_STORE_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn microsoft_store_reports_failed_when_the_upgrade_fails() {
    let tools = StubTools::new(&["winget"]);
    let output = run(&tools, 42);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        MICROSOFT_STORE_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

/// With a package exclude list, held-back apps are not upgraded and are
/// reported in the run summary.
#[test]
fn microsoft_store_holds_back_excluded_apps() {
    let tools = StubTools::new(&["winget"]);

    let package_list =
        "Name Id Version Source\n------ -- -- ------- ------\nMicrosoft Teams Microsoft.Teams 2.0.0 msstore\nWhatsApp WhatsApp.LLC 2.2345.12.0 msstore\n";

    tools.set_responses(
        "winget",
        &[
            StubOutput::new(),                // detect: winget --version
            StubOutput::stdout(package_list), // check before: winget upgrade --source msstore
            StubOutput::new(),                // run: winget source update
            StubOutput::stdout(package_list), // run: winget upgrade --source msstore (package list)
            StubOutput::new(), // run: winget upgrade --id Microsoft.Teams --source msstore ...
            StubOutput::stdout(package_list), // check after: winget upgrade --source msstore (diff)
        ],
    );

    let config_path = tools.home().join("microsoft-store-config.toml");
    std::fs::write(&config_path, "[exclude]\npackages = [\"WhatsApp\"]\n")
        .unwrap_or_else(|error| panic!("failed to write config: {error}"));

    let output = tools
        .command()
        .arg("--only")
        .arg("microsoft-store")
        .arg("--config")
        .arg(&config_path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.step_calls();
    assert!(
        calls.iter().any(|call| {
            call == "winget --disable-interactivity upgrade --id Microsoft.Teams --source msstore --silent --accept-package-agreements --accept-source-agreements"
        }),
        "Microsoft.Teams should be upgraded by id: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.contains("WhatsApp.LLC")),
        "WhatsApp should not be upgraded: {calls:?}"
    );
    let text = stdout(&output);
    assert!(
        text.contains("held back: WhatsApp"),
        "held back WhatsApp in summary: {text}"
    );
}

/// `--list-steps` lists the Microsoft Store step with its detection state.
#[test]
fn microsoft_store_list_steps_lists_microsoft_store() {
    let tools = StubTools::new(&["winget"]);
    tools.set_responses(
        "winget",
        &[StubOutput::new()], // detect: winget --version
    );
    let output = tools
        .command()
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("microsoft-store"), "listing: {text}");
    assert!(text.contains("installed"), "listing: {text}");
}
