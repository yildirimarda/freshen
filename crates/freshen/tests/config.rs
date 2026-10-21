//! End-to-end checks for the configuration file.
//!
//! Every test runs the built binary against stub package managers with the
//! home, configuration and application data directories pointing into the
//! temporary directory, so no test reads the configuration file of the
//! machine it runs on. The preflight facts are scripted, so a test never
//! looks at the real disk, battery or network.

#![cfg(feature = "test-harness")]

mod support;

use std::fs;
use std::path::Path;
use std::process::Output;

use support::StubTools;

/// The free disk space the scripted preflight probe reports: 10 GiB.
const FREE_DISK_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// A configuration that asks for more disk space than the scripted probe
/// reports, so a run that reads it stops with the preflight exit code before
/// it touches anything.
const DISK_SPACE_CONFIG: &str = r#"
[preflight]
network = false
battery = false
metered_connection = false
min_free_disk_gib = 1000000

[package_manager]
enabled = false
"#;

/// A configuration that fits in the disk space the scripted probe reports.
const ENOUGH_DISK_SPACE_CONFIG: &str = r#"
[preflight]
network = false
battery = false
metered_connection = false
min_free_disk_gib = 1

[package_manager]
enabled = false
"#;

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn write_config(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", parent.display()));
    }
    fs::write(path, contents)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

fn run(tools: &StubTools, args: &[&str]) -> Output {
    tools
        .command()
        .env("FRESHEN_SKIP_PREFLIGHT", "0")
        .env("FRESHEN_TEST_STEPS", "uv")
        .env("FRESHEN_TEST_PREFLIGHT", format!("disk={FREE_DISK_BYTES}"))
        .env("FRESHEN_TEST_PACKAGE_MANAGERS", "")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

#[test]
fn the_configuration_in_the_default_location_is_used() {
    let tools = StubTools::new(&["uv"]);
    write_config(&tools.config_path(), DISK_SPACE_CONFIG);

    let output = run(&tools, &[]);

    assert_eq!(output.status.code(), Some(3), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("disk space"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn a_configuration_given_with_the_config_flag_is_used() {
    let tools = StubTools::new(&["uv"]);
    let path = tools.home().join("elsewhere.toml");
    write_config(&path, DISK_SPACE_CONFIG);

    let path = path.to_string_lossy().into_owned();
    let output = run(&tools, &["--config", &path]);

    assert_eq!(output.status.code(), Some(3), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("disk space"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn the_command_line_wins_over_the_configuration_file() {
    let tools = StubTools::new(&["uv"]);
    write_config(&tools.config_path(), ENOUGH_DISK_SPACE_CONFIG);

    let output = run(&tools, &["--min-free-disk-gib", "1000000"]);

    assert_eq!(output.status.code(), Some(3), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("disk space"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn a_configuration_that_fits_lets_the_run_start() {
    let tools = StubTools::new(&["uv"]);
    write_config(&tools.config_path(), ENOUGH_DISK_SPACE_CONFIG);

    let output = run(&tools, &[]);

    assert!(
        output.status.success(),
        "stderr: {}\nstdout: {}",
        stderr(&output),
        stdout(&output)
    );
    assert_eq!(tools.calls(), ["uv"]);
}

#[test]
fn a_missing_configuration_file_is_an_error() {
    let tools = StubTools::new(&["uv"]);
    let path = tools.home().join("missing.toml");

    let path = path.to_string_lossy().into_owned();
    let output = run(&tools, &["--config", &path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("does not exist"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn an_invalid_configuration_file_points_at_the_offending_line() {
    let tools = StubTools::new(&["uv"]);
    let path = tools.home().join("broken.toml");
    write_config(&path, "[preflight]\nnetwork = true\nnetwrok = false\n");

    let path = path.to_string_lossy().into_owned();
    let output = run(&tools, &["--config", &path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let stderr = stderr(&output);
    assert!(stderr.contains("broken.toml:3:"), "stderr: {stderr}");
    assert!(
        stderr.contains("unknown field `netwrok`"),
        "stderr: {stderr}"
    );
}

#[test]
fn an_out_of_range_value_points_at_the_offending_line() {
    let tools = StubTools::new(&["uv"]);
    let path = tools.home().join("range.toml");
    write_config(&path, "[preflight]\nbattery_threshold = 200\n");

    let path = path.to_string_lossy().into_owned();
    let output = run(&tools, &["--config", &path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let stderr = stderr(&output);
    assert!(stderr.contains("range.toml:2:"), "stderr: {stderr}");
    assert!(
        stderr.contains("battery_threshold must be between 0 and 100"),
        "stderr: {stderr}"
    );
}

#[test]
fn no_configuration_file_is_not_an_error() {
    let tools = StubTools::new(&["uv"]);

    let output = run(&tools, &[]);

    assert!(
        output.status.success(),
        "stderr: {}\nstdout: {}",
        stderr(&output),
        stdout(&output)
    );
}

#[test]
fn an_invalid_configuration_file_stops_a_listing() {
    let tools = StubTools::new(&["uv"]);
    let path = tools.home().join("broken.toml");
    write_config(&path, "[preflight]\nnetwork = true\nnetwrok = false\n");

    let path = path.to_string_lossy().into_owned();
    let output = run(&tools, &["--list-steps", "--config", &path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("unknown field `netwrok`"),
        "stderr: {}",
        stderr(&output)
    );
}
