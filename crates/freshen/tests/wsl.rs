//! End-to-end tests for the WSL step.
//!
//! Every test runs the built binary against a stub `wsl` executable and
//! asserts that `wsl --update` runs only when `--install-wsl-updates` is
//! given, because the update shuts down every running WSL distribution. No
//! real WSL installation is touched.

#![cfg(all(feature = "test-harness", target_os = "windows"))]

mod support;

use std::process::Output;

use support::{StubOutput, StubTools};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run(tools: &StubTools, opt_in: bool) -> Output {
    tools.set_responses(
        "wsl",
        &[
            StubOutput::new(), // detect: wsl --version
            StubOutput::new(), // list: wsl --list --running (no distributions running)
            StubOutput::new(), // run: wsl --update (only when opted in)
        ],
    );
    let mut command = tools.command();
    command.args(["--only", "wsl"]);
    if opt_in {
        command.arg("--install-wsl-updates");
    }
    command
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

/// The commands the wsl step issues when opted in: detection, the running
/// distribution listing, then the update.
const WSL_UPDATE_CALLS: &[&str] = &["wsl --version", "wsl --list --running", "wsl --update"];

/// The commands the wsl step issues without the opt-in: detection only.
const WSL_NO_UPDATE_CALLS: &[&str] = &["wsl --version"];

#[test]
fn wsl_updates_the_kernel_when_opted_in() {
    let tools = StubTools::new(&["wsl"]);
    let output = run(&tools, true);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WSL_UPDATE_CALLS,
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
fn wsl_does_not_update_without_the_opt_in() {
    let tools = StubTools::new(&["wsl"]);
    let output = run(&tools, false);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WSL_NO_UPDATE_CALLS,
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
fn wsl_reports_failed_when_the_update_fails() {
    let tools = StubTools::new(&["wsl"]);
    tools.set_responses(
        "wsl",
        &[
            StubOutput::new(),                 // detect: wsl --version
            StubOutput::new(),                 // list: wsl --list --running (none running)
            StubOutput::new().with_status(42), // run: wsl --update
        ],
    );
    let output = tools
        .command()
        .args(["--only", "wsl", "--install-wsl-updates"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        WSL_UPDATE_CALLS,
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 0 skipped, 1 failed"),
        "summary: {}",
        stdout(&output)
    );
}

/// `--yes` skips the confirmation prompt: the running distributions are listed
/// and `wsl --update` still runs.
#[test]
fn wsl_updates_with_yes_when_distributions_are_running() {
    let tools = StubTools::new(&["wsl"]);
    tools.set_responses(
        "wsl",
        &[
            StubOutput::new(), // detect: wsl --version
            StubOutput::stdout(
                "NAME            STATE           VERSION\n\
                 * Ubuntu        Running         2\n\
                 docker-desktop  Running         2\n",
            ), // list: wsl --list --running
            StubOutput::new(), // run: wsl --update
        ],
    );
    let output = tools
        .command()
        .args(["--only", "wsl", "--install-wsl-updates", "--yes"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        &["wsl --version", "wsl --list --running", "wsl --update",],
        "calls: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(text.contains("- Ubuntu"), "listing: {text}");
    assert!(text.contains("- docker-desktop"), "listing: {text}");
}

/// Without `--yes` the user must confirm; answering `n` skips the update so
/// `wsl --update` is never invoked.
#[test]
fn wsl_skips_update_when_the_user_declines() {
    let tools = StubTools::new(&["wsl"]);
    tools.set_responses(
        "wsl",
        &[
            StubOutput::new(), // detect: wsl --version
            StubOutput::stdout(
                "NAME            STATE           VERSION\n\
                 * Ubuntu        Running         2\n",
            ), // list: wsl --list --running
                               // wsl --update must not be called
        ],
    );
    let mut command = tools.command();
    command.args(["--only", "wsl", "--install-wsl-updates"]);
    let output = tools.run_with_stdin(command, "n\n");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        &["wsl --version", "wsl --list --running"],
        "calls: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(text.contains("- Ubuntu"), "listing: {text}");
    assert!(
        !text.contains("wsl: install"),
        "update ran despite declining: {text}"
    );
}

/// Answering `y` to the confirmation proceeds with the update.
#[test]
fn wsl_updates_when_the_user_confirms() {
    let tools = StubTools::new(&["wsl"]);
    tools.set_responses(
        "wsl",
        &[
            StubOutput::new(), // detect: wsl --version
            StubOutput::stdout(
                "NAME            STATE           VERSION\n\
                 * Ubuntu        Running         2\n",
            ), // list: wsl --list --running
            StubOutput::new(), // run: wsl --update
        ],
    );
    let mut command = tools.command();
    command.args(["--only", "wsl", "--install-wsl-updates"]);
    let output = tools.run_with_stdin(command, "y\n");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        &["wsl --version", "wsl --list --running", "wsl --update",],
        "calls: {:?}",
        tools.step_calls()
    );
    let text = stdout(&output);
    assert!(text.contains("wsl: install"), "update did not run: {text}");
}

#[test]
fn wsl_is_skipped_when_wsl_is_absent() {
    let tools = StubTools::new(&[]);
    // On Windows the stub harness puts System32 on PATH so `.cmd` stubs can
    // run via `cmd.exe`, which would leak a real `wsl.exe` into the stub-only
    // environment. Restrict PATH to the stub directory so `wsl` is genuinely
    // absent and the step is skipped.
    let mut command = tools.command();
    command.env("PATH", tools.stub_dir());
    let output = command
        .args(["--only", "wsl", "--install-wsl-updates"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools.step_calls().is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

/// `--list-steps` lists the wsl step with its detection state.
#[test]
fn wsl_list_steps_lists_wsl() {
    let tools = StubTools::new(&["wsl"]);
    tools.set_responses("wsl", &[StubOutput::new()]); // detect: wsl --version
    let output = tools
        .command()
        .args(["--list-steps"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("wsl"), "listing: {text}");
    assert!(text.contains("installed"), "listing: {text}");
}

/// apt runs inside the selected WSL distribution via `wsl -d <distro> --exec`.
#[test]
fn apt_runs_inside_the_selected_wsl_distribution() {
    let tools = StubTools::new(&["wsl"]);
    tools.set_responses(
        "wsl",
        &[
            StubOutput::new(), // check: wsl -d Ubuntu --exec apt-get -s upgrade
            StubOutput::new(), // run: wsl -d Ubuntu --exec apt-get update
            StubOutput::new(), // run: wsl -d Ubuntu --exec apt-get upgrade -y
        ],
    );
    let output = tools
        .command()
        .args(["--only", "apt", "--wsl-distro", "Ubuntu"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        tools.step_calls(),
        &[
            "wsl -d Ubuntu --exec apt-get -s upgrade",
            "wsl -d Ubuntu --exec apt-get update",
            "wsl -d Ubuntu --exec apt-get upgrade -y",
        ],
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("1 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

/// apt is a no-op on Windows when no WSL distribution is selected.
#[test]
fn apt_is_a_noop_without_a_selected_wsl_distribution() {
    let tools = StubTools::new(&["wsl"]);
    let output = tools
        .command()
        .args(["--only", "apt"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools.step_calls().is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("1 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

/// apt is skipped on Windows when WSL itself is absent.
#[test]
fn apt_is_skipped_when_wsl_is_absent() {
    let tools = StubTools::new(&[]);
    let mut command = tools.command();
    command.env("PATH", tools.stub_dir());
    let output = command
        .args(["--only", "apt", "--wsl-distro", "Ubuntu"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        tools.step_calls().is_empty(),
        "calls: {:?}",
        tools.step_calls()
    );
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 1 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}
