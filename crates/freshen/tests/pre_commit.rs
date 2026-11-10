//! End-to-end tests for the `pre-commit` step.
//!
//! Every test runs the built binary against a stub `pre-commit` executable
//! and asserts the exact command transcript. freshen never scans the disk for
//! repositories: only repositories listed in the configuration file are
//! touched, and no real pre-commit is invoked.

#![cfg(all(feature = "test-harness", target_os = "linux"))]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use support::StubTools;

/// A marker the stub `pre-commit` leaves in the directory it ran in, so a
/// test can tell which repository `pre-commit autoupdate` ran inside.
const MARKER: &str = ".pre-commit-autoupdate-ran";

fn run(tools: &StubTools, config: &str) -> Output {
    write_config(&tools.config_path(), config);
    tools
        .command()
        .args(["--only", "pre-commit"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn write_config(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", parent.display()));
    }
    fs::write(path, contents)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn repositories(tools: &StubTools) -> (PathBuf, PathBuf, PathBuf) {
    let base = tools.stub_dir().to_path_buf();
    let repo1 = base.join("repo1");
    let repo2 = base.join("repo2");
    let decoy = base.join("decoy");
    for directory in [&repo1, &repo2, &decoy] {
        fs::create_dir_all(directory)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", directory.display()));
    }
    // The decoy looks like a repository but is not listed in the config, so
    // freshen must never touch it.
    fs::write(decoy.join(".pre-commit-config.yaml"), "repos: []\n").unwrap_or_else(|error| {
        panic!(
            "failed to write {}: {error}",
            decoy.join(".pre-commit-config.yaml").display()
        )
    });
    (repo1, repo2, decoy)
}

fn config_text(repo1: &Path, repo2: &Path) -> String {
    format!(
        r#"[pre_commit]
repositories = ["{}", "{}"]
"#,
        repo1.display(),
        repo2.display()
    )
}

#[test]
fn pre_commit_updates_each_listed_repository() {
    let tools = StubTools::new(&["pre-commit"]);
    tools.set_script("pre-commit", ": > .pre-commit-autoupdate-ran\n");
    let (repo1, repo2, decoy) = repositories(&tools);

    let output = run(&tools, &config_text(&repo1, &repo2));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        vec!["pre-commit autoupdate", "pre-commit autoupdate"],
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        repo1.join(MARKER).is_file(),
        "pre-commit autoupdate did not run in {}",
        repo1.display()
    );
    assert!(
        repo2.join(MARKER).is_file(),
        "pre-commit autoupdate did not run in {}",
        repo2.display()
    );
    assert!(
        !decoy.join(MARKER).is_file(),
        "freshen scanned the disk and touched the unlisted repository {}",
        decoy.display()
    );
}

#[test]
fn pre_commit_skips_when_pre_commit_is_missing() {
    let tools = StubTools::new(&[]);
    let (repo1, repo2, _decoy) = repositories(&tools);

    let output = run(&tools, &config_text(&repo1, &repo2));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn pre_commit_does_nothing_without_repositories() {
    let tools = StubTools::new(&["pre-commit"]);
    tools.set_script("pre-commit", ": > .pre-commit-autoupdate-ran\n");
    let (repo1, _repo2, _decoy) = repositories(&tools);

    let output = run(&tools, "");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools.step_calls().is_empty(),
        "ran a command with no repositories configured: {:?}",
        tools.step_calls()
    );
    assert!(
        !repo1.join(MARKER).is_file(),
        "ran pre-commit autoupdate with no repositories configured"
    );
}

#[test]
fn check_reports_pre_commit_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&["pre-commit"]);
    tools.set_script("pre-commit", ": > .pre-commit-autoupdate-ran\n");
    let (repo1, _repo2, _decoy) = repositories(&tools);

    let output = tools
        .command()
        .args(["--check", "--only", "pre-commit"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "Everything is up to date.\n");
    assert!(tools.step_calls().is_empty());
    assert!(
        !repo1.join(MARKER).is_file(),
        "ran pre-commit autoupdate during --check"
    );
}
