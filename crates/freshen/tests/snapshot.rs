//! End-to-end checks for the pre-upgrade snapshot.
//!
//! Every test runs the built binary against stub package managers with
//! `FRESHEN_SNAPSHOT_DIR` pointing into the temporary home, so no test reads or
//! writes a real snapshot directory.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use freshen_lib::DEFAULT_SNAPSHOT_RETENTION;
use support::StubTools;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn snapshot_dir(tools: &StubTools) -> PathBuf {
    tools.home().join("snapshots")
}

fn snapshot_files(tools: &StubTools) -> Vec<PathBuf> {
    let directory = snapshot_dir(tools);
    match fs::read_dir(&directory) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!(
            "failed to read snapshot directory {}: {error}",
            directory.display()
        ),
    }
}

fn only_snapshot(tools: &StubTools) -> String {
    let files = snapshot_files(tools);
    assert_eq!(files.len(), 1, "expected one snapshot file: {files:?}");
    fs::read_to_string(&files[0])
        .unwrap_or_else(|error| panic!("failed to read snapshot {}: {error}", files[0].display()))
}

fn run(tools: &StubTools, args: &[&str]) -> Output {
    tools
        .command()
        .env("FRESHEN_SNAPSHOT_DIR", snapshot_dir(tools))
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn write_old_snapshot(tools: &StubTools, name: &str) -> PathBuf {
    let directory = snapshot_dir(tools);
    fs::create_dir_all(&directory).unwrap_or_else(|error| {
        panic!(
            "failed to create snapshot directory {}: {error}",
            directory.display()
        )
    });
    let path = directory.join(name);
    fs::write(&path, "# old snapshot\n")
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
    path
}

fn script_uv(tools: &StubTools) {
    tools.set_script(
        "uv",
        r#"case "$*" in
  "tool list")
    printf 'ruff v0.6.0\n'
    ;;
esac"#,
    );
}

fn snapshot_names(files: &[PathBuf]) -> Vec<String> {
    files
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .map(str::to_owned)
        .collect()
}

fn assert_pruned(path: &Path, expected: bool) {
    assert_eq!(
        path.exists(),
        expected,
        "{} existence should be {expected}",
        path.display()
    );
}

#[test]
fn records_installed_versions_for_each_step_in_a_timestamped_file() {
    let tools = StubTools::new(&["brew", "uv", "mise"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "list --versions")
    printf 'jq 1.7.1\n'
    ;;
  "bundle dump --force --file="*)
    for arg in "$@"; do
      case "$arg" in
        --file=*) printf 'brew "jq"\n' > "${arg#--file=}" ;;
      esac
    done
    ;;
esac"#,
    );
    tools.set_script(
        "uv",
        r#"case "$*" in
  "tool list")
    printf 'ruff v0.6.0\n'
    ;;
esac"#,
    );
    tools.set_script(
        "mise",
        r#"case "$*" in
  "ls")
    printf 'node 22.1.0\n'
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", "brew,uv,mise"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let files = snapshot_files(&tools);
    assert_eq!(files.len(), 1, "expected one snapshot file: {files:?}");

    let name = files[0]
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| panic!("snapshot file name should be UTF-8"));
    assert!(name.starts_with("snapshot-"), "name: {name}");
    assert!(name.ends_with(".txt"), "name: {name}");

    let text = only_snapshot(&tools);
    assert!(text.contains("# freshen pre-upgrade snapshot"), "{text}");
    assert!(text.contains("## brew"), "{text}");
    assert!(text.contains("$ brew list --versions"), "{text}");
    assert!(text.contains("jq 1.7.1"), "{text}");
    assert!(text.contains("$ brew bundle dump"), "{text}");
    assert!(text.contains("brew \"jq\""), "{text}");
    assert!(text.contains("## uv"), "{text}");
    assert!(text.contains("$ uv tool list"), "{text}");
    assert!(text.contains("ruff v0.6.0"), "{text}");
    assert!(text.contains("## mise"), "{text}");
    assert!(text.contains("$ mise ls"), "{text}");
    assert!(text.contains("node 22.1.0"), "{text}");
    assert!(
        stderr(&output).contains("pre-upgrade snapshot written to"),
        "stderr: {}",
        stderr(&output)
    );

    let summary = stdout(&output);
    assert!(summary.contains("Snapshot:"), "summary: {summary}");
    assert!(
        summary.contains("Rollback hints (previous versions from the snapshot):"),
        "summary: {summary}"
    );
    assert!(
        summary.contains("brew install jq@1.7.1"),
        "summary: {summary}"
    );
    assert!(
        summary.contains("uv tool install ruff==0.6.0"),
        "summary: {summary}"
    );
    assert!(
        summary.contains("mise install node@22.1.0"),
        "summary: {summary}"
    );
}

#[test]
fn captures_versions_before_the_upgrade_changes_them() {
    let tools = StubTools::new(&["uv"]);
    tools.set_script(
        "uv",
        r#"case "$*" in
  "tool list")
    if [ -f "$FRESHEN_STUB_STATE/upgraded" ]; then
      printf 'ruff v0.7.0\n'
    else
      printf 'ruff v0.6.0\n'
    fi
    ;;
  "tool upgrade --all")
    : > "$FRESHEN_STUB_STATE/upgraded"
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", "uv", "--cleanup"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = only_snapshot(&tools);
    assert!(text.contains("ruff v0.6.0"), "{text}");
    assert!(!text.contains("ruff v0.7.0"), "{text}");
    assert_eq!(
        tools.calls(),
        ["uv tool list", "uv tool upgrade --all", "uv cache prune"]
    );
}

#[test]
fn records_each_snapshot_group_only_once_per_run() {
    let tools = StubTools::new(&["brew"]);
    tools.set_script(
        "brew",
        r#"case "$*" in
  "list --versions")
    printf 'jq 1.7.1\n'
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", "brew,brew-formulae"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let calls = tools.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.as_str() == "brew list --versions")
            .count(),
        1,
        "calls: {calls:?}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.starts_with("brew bundle dump "))
            .count(),
        1,
        "calls: {calls:?}"
    );
    let text = only_snapshot(&tools);
    assert_eq!(text.matches("## brew").count(), 1, "{text}");
}

#[test]
fn dry_run_and_check_do_not_write_a_snapshot() {
    let tools = StubTools::new(&["brew"]);

    let dry_run = run(&tools, &["--dry-run", "--only", "brew"]);
    assert!(dry_run.status.success(), "stderr: {}", stderr(&dry_run));
    assert!(snapshot_files(&tools).is_empty());

    let check = run(&tools, &["--check", "--only", "brew"]);
    assert!(check.status.success(), "stderr: {}", stderr(&check));
    assert!(snapshot_files(&tools).is_empty());
}

#[test]
fn a_failed_snapshot_command_is_recorded_without_failing_the_step() {
    let tools = StubTools::new(&["uv"]);
    tools.set_script(
        "uv",
        r#"case "$*" in
  "tool list")
    printf 'no tools installed\n' >&2
    exit 1
    ;;
esac"#,
    );

    let output = run(&tools, &["--only", "uv"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("1 ok"), "{}", stdout(&output));
    let text = only_snapshot(&tools);
    assert!(text.contains("$ uv tool list"), "{text}");
    assert!(text.contains("# command failed"), "{text}");
    assert!(text.contains("no tools installed"), "{text}");
}

#[test]
fn prunes_old_snapshots_to_the_configured_retention() {
    let tools = StubTools::new(&["uv"]);
    script_uv(&tools);
    let oldest = write_old_snapshot(&tools, "snapshot-2020-01-01T00-00-00Z-1.txt");
    let older = write_old_snapshot(&tools, "snapshot-2021-01-01T00-00-00Z-1.txt");
    let recent = write_old_snapshot(&tools, "snapshot-2025-01-01T00-00-00Z-1.txt");

    let output = run(&tools, &["--only", "uv", "--snapshot-retention", "2"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let files = snapshot_files(&tools);
    assert_eq!(
        files.len(),
        2,
        "expected two snapshots: {:?}",
        snapshot_names(&files)
    );
    assert_pruned(&recent, true);
    assert_pruned(&older, false);
    assert_pruned(&oldest, false);
    assert!(
        files.iter().any(|path| path != &recent),
        "the new snapshot should be kept alongside the most recent old one"
    );
}

#[test]
fn default_retention_keeps_a_small_recent_window() {
    let tools = StubTools::new(&["uv"]);
    script_uv(&tools);
    let mut old = Vec::new();
    for day in 1..=DEFAULT_SNAPSHOT_RETENTION + 2 {
        old.push(write_old_snapshot(
            &tools,
            &format!("snapshot-2020-01-{day:02}T00-00-00Z-1.txt"),
        ));
    }

    let output = run(&tools, &["--only", "uv"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let files = snapshot_files(&tools);
    assert_eq!(
        files.len(),
        DEFAULT_SNAPSHOT_RETENTION,
        "expected the default retention window: {:?}",
        snapshot_names(&files)
    );
    assert_pruned(&old[0], false);
    assert_pruned(&old[1], false);
    assert_pruned(&old[2], false);
    assert_pruned(
        old.last()
            .unwrap_or_else(|| panic!("old snapshots should not be empty")),
        true,
    );
}
