//! End-to-end tests for the Chocolatey step.
//!
//! Every test runs the built binary against a stub `choco` executable and
//! asserts that Chocolatey itself is upgraded first, then every outdated
//! package is upgraded individually, with per-package failures recorded in
//! the summary. No real package manager is touched.

#![cfg(all(feature = "test-harness", target_os = "windows"))]

mod support;

use std::fs;
use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Reads the single snapshot file written by the run and returns its contents.
fn only_snapshot(tools: &StubTools) -> String {
    let directory = tools.home().join("snapshots");
    let files: Vec<_> = fs::read_dir(&directory)
        .unwrap_or_else(|error| {
            panic!(
                "failed to read snapshot directory {}: {error}",
                directory.display()
            )
        })
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .collect();
    assert_eq!(files.len(), 1, "expected one snapshot file: {files:?}");
    fs::read_to_string(&files[0])
        .unwrap_or_else(|error| panic!("failed to read snapshot {}: {error}", files[0].display()))
}

fn run(tools: &StubTools, upgrade_exit: i32) -> Output {
    let package_list = "git|2.45.2|2.46.0|\npython|3.11.0|3.11.1|\n";

    tools.set_responses(
        "choco",
        &[
            StubOutput::new(),                           // detect: choco --version
            StubOutput::new(),                           // snapshot: choco list --local-only -r
            StubOutput::stdout(package_list), // check before: choco outdated --limit-output
            StubOutput::new(),                // run: choco upgrade chocolatey
            StubOutput::stdout(package_list), // run: choco outdated --limit-output (package list)
            StubOutput::new().with_status(upgrade_exit), // run: choco upgrade git
            StubOutput::new(),                // run: choco upgrade python
            StubOutput::stdout(package_list), // check after: choco outdated --limit-output
        ],
    );
    tools
        .command()
        .args(["--only", "chocolatey"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

/// The commands the Chocolatey step issues, in order: detection, the read-only
/// check, the Chocolatey self-upgrade, the package list, one upgrade per
/// outdated package, and the check again for the version diff. The pre-upgrade
/// snapshot capture is excluded here because it is read-only and asserted
/// separately.
const CHOCO_CALLS: &[&str] = &[
    "choco --version",
    "choco outdated --limit-output",
    "choco upgrade chocolatey",
    "choco outdated --limit-output",
    "choco upgrade git",
    "choco upgrade python",
    "choco outdated --limit-output",
];

#[test]
fn chocolatey_reports_ok_when_all_upgrades_succeed() {
    let tools = StubTools::new(&["choco"]);
    let output = run(&tools, 0);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        CHOCO_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("1 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );

    // The pre-upgrade snapshot records `choco list --local-only -r`.
    let text = only_snapshot(&tools);
    assert!(
        text.contains("## chocolatey"),
        "snapshot should have a chocolatey section: {text}"
    );
    assert!(
        text.contains("$ choco list --local-only -r"),
        "snapshot should record the choco list command: {text}"
    );
}

#[test]
fn chocolatey_reports_failed_when_a_package_upgrade_fails() {
    let tools = StubTools::new(&["choco"]);
    let output = run(&tools, 42);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        CHOCO_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("git"),
        "failed package git should be named: {}",
        stdout(&output)
    );
}

/// `--list-steps` lists the Chocolatey step with its detection state.
#[test]
fn chocolatey_list_steps_lists_chocolatey() {
    let tools = StubTools::new(&["choco"]);
    tools.set_responses(
        "choco",
        &[StubOutput::new()], // detect: choco --version
    );
    let output = tools
        .command()
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("chocolatey"), "listing: {text}");
    assert!(text.contains("installed"), "listing: {text}");
}
