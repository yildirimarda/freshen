//! End-to-end checks for the single-run lock.
//!
//! The lock is held by the test process itself, so a real second run can be
//! started while it is held without spawning a long-running first update.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::process::Output;

use freshen_lib::RunLock;
use support::StubTools;

const EVERY_BUILTIN_STEP: &str =
    "brew,brew-formulae,brew-casks,brew-autoremove,brew-cleanup,brew-reopen,colima-stop,colima-start,docker,uv,mise";

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_run_while_another_holds_the_lock_names_the_holder_pid() {
    let tools = StubTools::new(&[]);
    let lock = RunLock::acquire_in(tools.home().join("lock"))
        .unwrap_or_else(|error| panic!("failed to take the lock: {error}"));

    let output = tools
        .command()
        .args(["--skip", EVERY_BUILTIN_STEP])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let text = stderr(&output);
    assert!(
        text.contains("another freshen run is in progress"),
        "{text}"
    );
    assert!(
        text.contains(&format!("PID {}", std::process::id())),
        "the message must name the holder PID: {text}"
    );

    drop(lock);
}

#[test]
fn a_completed_run_releases_the_lock_for_the_next_run() {
    let tools = StubTools::new(&[]);

    let first = tools
        .command()
        .args(["--skip", EVERY_BUILTIN_STEP])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(first.status.success(), "stderr: {}", stderr(&first));

    let second = tools
        .command()
        .args(["--skip", EVERY_BUILTIN_STEP])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(second.status.success(), "stderr: {}", stderr(&second));
}

#[test]
fn a_dry_run_does_not_take_the_lock() {
    let tools = StubTools::new(&[]);
    let _lock = RunLock::acquire_in(tools.home().join("lock"))
        .unwrap_or_else(|error| panic!("failed to take the lock: {error}"));

    let output = tools
        .command()
        .args(["--dry-run", "--skip", EVERY_BUILTIN_STEP])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
}
