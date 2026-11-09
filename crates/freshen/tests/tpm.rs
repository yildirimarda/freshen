//! Parity tests for the tmux plugin manager step.
//!
//! Every test runs the built binary against a stub `~/.tmux/plugins/tpm/bin/
//! update_plugins` script and asserts the exact command transcript of the
//! `update_plugins all` block. No real tool is touched.

#![cfg(feature = "test-harness")]
#![cfg(unix)]

mod support;

use std::fs;
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::process::Output;

use support::StubTools;

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .args(["--only", "tpm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Writes a stub `~/.tmux/plugins/tpm/bin/update_plugins` that logs its
/// invocation to `FRESHEN_STUB_LOG` and exits with `status`.
fn write_tpm_stub(tools: &StubTools, status: i32) {
    let path = tools.home().join(".tmux/plugins/tpm/bin/update_plugins");
    let parent = path
        .parent()
        .unwrap_or_else(|| panic!("the tpm script has a parent directory"));
    fs::create_dir_all(parent).unwrap_or_else(|error| {
        panic!(
            "failed to create tpm plugin directory {}: {error}",
            parent.display()
        )
    });
    let script = format!(
        "#!/bin/sh\n\
         {{ printf '%s' 'tpm'; for arg in \"$@\"; do printf ' %s' \"$arg\"; done; printf '\\n'; }} >> \"${{FRESHEN_STUB_LOG:?FRESHEN_STUB_LOG is not set}}\"\n\
         exit {status}\n"
    );
    fs::write(&path, script)
        .unwrap_or_else(|error| panic!("failed to write tpm stub {}: {error}", path.display()));
    fs::set_permissions(&path, Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("failed to make tpm stub executable: {error}"));
}

#[test]
fn tpm_updates_plugins() {
    let tools = StubTools::new(&[]);
    write_tpm_stub(&tools, 0);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::tpm_transcript());
}

#[test]
fn tpm_skips_when_tpm_is_absent() {
    let tools = StubTools::new(&[]);

    let output = run(&tools);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn a_failing_tpm_update_fails_the_step() {
    let tools = StubTools::new(&[]);
    write_tpm_stub(&tools, 1);

    let output = run(&tools);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(tools.step_calls(), support::tpm_transcript());
    assert!(
        stdout(&output).contains("1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_tpm_up_to_date_without_calling_anything() {
    let tools = StubTools::new(&[]);
    write_tpm_stub(&tools, 0);

    let output = tools
        .command()
        .args(["--check", "--only", "tpm"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(tools.step_calls().is_empty());
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}
