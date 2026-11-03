//! The winget step: detection, a read-only check and the upgrade for Windows
//! package updates.
//!
//! `winget` is the Windows Package Manager. Detection runs `winget --version`;
//! `check()` runs `winget upgrade` and parses the table of packages with
//! updates available. `run()` refreshes the package index with `winget source
//! update` and then upgrades every package with
//! `winget upgrade --all --silent --accept-package-agreements
//! --accept-source-agreements`. Neither `detect` nor `check` changes anything,
//! so the step is safe in `--check` mode. The step is only available on
//! Windows.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, ProcessError};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, SkipReason, Step, StepError, StepId};

/// The winget package manager step.
pub const WINGET: StepId = "winget";

/// Exit code `winget upgrade` returns when no package upgrades are
/// applicable, so freshen reports the step as skipped instead of failed.
///
/// This is a freshen convention; confirm it against real `winget` output
/// before shipping on Windows.
pub const EXIT_UPDATE_NOT_APPLICABLE: i32 = 1;

/// winget only runs on Windows.
const WINGET_PLATFORMS: [Platform; 1] = [Platform::Windows];

/// Registers the winget step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(WingetStep)
}

struct WingetStep;

impl WingetStep {
    /// Reports the packages `winget upgrade` lists as outdated.
    fn check(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("winget")
            .args(["upgrade"])
            .neutral_locale()
            .run()
            .map_err(|error| StepError::check_from_process(WINGET, &error))?;
        let items = parse_winget_upgrade(output.stdout());
        Ok(CheckReport::from_items(items))
    }
}

impl Step for WingetStep {
    fn id(&self) -> StepId {
        WINGET
    }

    fn display_name(&self) -> &str {
        "winget"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        if CommandSpec::new("winget").args(["--version"]).run().is_ok() {
            Ok(Detection::Installed)
        } else {
            Ok(Detection::NotInstalled)
        }
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check()
    }

    /// Refreshes the package index with `winget source update` and then
    /// upgrades every package with `winget upgrade --all --silent
    /// --accept-package-agreements --accept-source-agreements`. A successful
    /// source update is required, so its failure is reported first. The
    /// upgrade's exit code is mapped as follows: `0` is ok, the update-not
    /// applicable code is skipped (nothing to upgrade), and any other non-zero
    /// code is a failure.
    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        let mut not_applicable = false;

        human!("winget source update");
        if let Err(error) = CommandSpec::new("winget")
            .args(["source", "update"])
            .neutral_locale()
            .run()
        {
            failures.push(StepError::run_from_process(WINGET, &error));
        }

        human!(
            "winget upgrade --all --silent --accept-package-agreements --accept-source-agreements"
        );
        let upgrade = CommandSpec::new("winget")
            .args([
                "upgrade",
                "--all",
                "--silent",
                "--accept-package-agreements",
                "--accept-source-agreements",
            ])
            .neutral_locale()
            .run();
        if let Err(error) = &upgrade {
            if exit_code(error) == Some(EXIT_UPDATE_NOT_APPLICABLE) {
                not_applicable = true;
            } else {
                failures.push(StepError::run_from_process(WINGET, error));
            }
        }

        if let Some(error) = failures.into_iter().next() {
            return Err(error);
        }
        if not_applicable {
            return Err(StepError::run(WINGET, "no applicable package upgrades")
                .with_skip_reason(SkipReason::NothingToDo));
        }
        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &WINGET_PLATFORMS
    }
}

/// Returns the exit code of a failed process, when one was captured.
fn exit_code(error: &ProcessError) -> Option<i32> {
    error
        .output()
        .and_then(|output| output.status())
        .and_then(|status| status.code())
}

/// Parses the output of `winget upgrade`.
///
/// Each row of the upgrade table is `Name Id Version Source`, where `Name`
/// may contain spaces. The last three whitespace-separated tokens are
/// `Source`, `Version` and `Id`; everything before `Id` is the package name.
/// The announcement, the column titles and the separator line are skipped.
/// `winget` does not report the currently installed version, so only the
/// available version is recorded.
fn parse_winget_upgrade(output: &str) -> Vec<OutdatedItem> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            // Skip the "Found the following packages available for upgrade:"
            // announcement and the "nothing to upgrade" message.
            if line.starts_with("Found the following packages")
                || line.starts_with("No applicable package upgrades found")
            {
                return None;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() < 4 {
                return None;
            }
            // Skip the column titles: `Name Id Version Source`.
            if tokens == ["Name", "Id", "Version", "Source"] {
                return None;
            }
            // Skip the separator line: runs of dashes separated by spaces.
            if tokens
                .iter()
                .all(|token| token.bytes().all(|byte| byte == b'-'))
            {
                return None;
            }
            let version = tokens.get(tokens.len() - 2).copied().unwrap_or("");
            let id = tokens.get(tokens.len() - 3).copied().unwrap_or("");
            let name = tokens[..tokens.len() - 3].join(" ");
            if name.is_empty() || id.is_empty() || version.is_empty() {
                return None;
            }
            Some(OutdatedItem::new(name).with_latest(version))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_winget_upgrade, WingetStep, EXIT_UPDATE_NOT_APPLICABLE};
    use crate::step::{SkipReason, Step, StepError};

    #[test]
    fn parses_the_upgrade_table() {
        let items = parse_winget_upgrade(
            "Found the following packages available for upgrade:\n\
             Name                Id                          Version        Source\n\
             --------------------------------------------------------------------------------\n\
             7zip                7zip.7zip                   24.08        winget\n\
             Git                 Git.Git                     2.45.2       winget\n\
             Microsoft Visual C++ Redistributable  Microsoft.VC++.Redist  14.42.34512  winget\n",
        );

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name(), "7zip");
        assert_eq!(items[0].current(), None);
        assert_eq!(items[0].latest(), Some("24.08"));
        assert_eq!(items[1].name(), "Git");
        assert_eq!(items[1].latest(), Some("2.45.2"));
        assert_eq!(items[2].name(), "Microsoft Visual C++ Redistributable");
        assert_eq!(items[2].latest(), Some("14.42.34512"));
    }

    #[test]
    fn reports_nothing_when_no_upgrades_are_available() {
        let items = parse_winget_upgrade("No applicable package upgrades found.\n");
        assert!(items.is_empty());
    }

    #[test]
    fn skips_the_header_and_separator_lines() {
        let items = parse_winget_upgrade(
            "Found the following packages available for upgrade:\n\
             Name Id Version Source\n\
             ----------------------\n",
        );
        assert!(items.is_empty());
    }

    #[test]
    fn ignores_short_lines_that_do_not_form_a_row() {
        let items = parse_winget_upgrade("7zip\nwinget\n");
        assert!(items.is_empty());
    }

    /// Runs `winget source update` before `winget upgrade`, recording each
    /// invocation against a stub `winget` placed on `PATH`.
    #[test]
    fn run_updates_the_source_before_upgrading() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-winget-run-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        let log = dir.join("calls.log");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script = "#!/bin/sh\n\
                 {\n\
                   printf '%s' winget\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 } >> \"$FRESHEN_WINGET_LOG\"\n\
                 exit 0\n";
            let stub = dir.join("winget");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n>>\"%FRESHEN_WINGET_LOG%\" echo winget %*\r\nexit /b 0\r\n";
            let stub = dir.join("winget.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINGET_LOG").ok();

        // Serialize with the other tests that replace the global `PATH` (snap
        // replaces it entirely) so this test's stub `winget` is not shadowed
        // while `run()` spawns it twice.
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINGET_LOG", &log);

        let result = WingetStep.run();

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINGET_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_LOG");
        }

        assert!(result.is_ok(), "winget run should succeed");

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);

        let calls: Vec<&str> = calls.lines().map(str::trim).collect();
        assert_eq!(
            calls,
            vec![
                "winget --disable-interactivity source update",
                "winget --disable-interactivity upgrade --all --silent --accept-package-agreements --accept-source-agreements",
            ],
            "calls: {calls:?}"
        );
    }

    /// The upgrade exit code maps to ok / skipped / failed: exit 0 is ok, the
    /// update-not-applicable code is skipped (nothing to upgrade) and any
    /// other non-zero code is a failure.
    #[test]
    fn run_maps_upgrade_exit_codes_to_ok_skipped_and_failed() {
        for (exit_code, expect_ok, expect_skipped) in [
            (0, true, false),
            (EXIT_UPDATE_NOT_APPLICABLE, false, true),
            (42, false, false),
        ] {
            let (result, calls) = run_winget_with_upgrade_exit(exit_code);

            if expect_ok {
                assert!(result.is_ok(), "exit {exit_code} should be ok");
            } else {
                let error = match result {
                    Err(error) => error,
                    Ok(()) => panic!("exit {exit_code} should have failed, got Ok"),
                };
                let expected_skip = if expect_skipped {
                    Some(SkipReason::NothingToDo)
                } else {
                    None
                };
                assert_eq!(
                    error.skip_reason(),
                    expected_skip,
                    "exit {exit_code} skip reason"
                );
            }

            assert_eq!(
                calls,
                &[
                    "winget --disable-interactivity source update",
                    "winget --disable-interactivity upgrade --all --silent --accept-package-agreements --accept-source-agreements",
                ],
                "calls for exit {exit_code}: {calls:?}"
            );
        }
    }

    /// Runs `winget source update` (always exit 0) followed by `winget
    /// upgrade --all ...` which exits with `exit_code`, recording both
    /// invocations against a stub `winget` on `PATH`.
    fn run_winget_with_upgrade_exit(exit_code: i32) -> (Result<(), StepError>, Vec<String>) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-winget-exit-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        let log = dir.join("calls.log");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script = "#!/bin/sh\n\
                 {\n\
                   printf '%s' winget\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 } >> \"$FRESHEN_WINGET_LOG\"\n\
                 case \"$2\" in\n\
                   source) exit 0 ;;\n\
                   *) exit \"${FRESHEN_WINGET_EXIT_CODE:-0}\" ;;\n\
                 esac\n";
            let stub = dir.join("winget");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n\
                 >>\"%FRESHEN_WINGET_LOG%\" echo winget %*\r\n\
                 set \"code=%FRESHEN_WINGET_EXIT_CODE%\r\n\
                 if not defined code set code=0\r\n\
                 if \"%~2\"==\"source\" (\r\n\
                   exit /b 0\r\n\
                 ) else (\r\n\
                   exit /b %code%\r\n\
                 )\r\n";
            let stub = dir.join("winget.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        let previous_path = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WINGET_LOG").ok();
        let previous_exit_code = std::env::var("FRESHEN_WINGET_EXIT_CODE").ok();

        // Serialize with the other tests that replace the global `PATH`
        // while `run()` spawns the stub twice.
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var("PATH", &dir);
        std::env::set_var("FRESHEN_WINGET_LOG", &log);
        std::env::set_var("FRESHEN_WINGET_EXIT_CODE", exit_code.to_string());

        let result = WingetStep.run();

        if let Some(previous) = previous_path {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WINGET_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_LOG");
        }
        if let Some(previous) = previous_exit_code {
            std::env::set_var("FRESHEN_WINGET_EXIT_CODE", previous);
        } else {
            std::env::remove_var("FRESHEN_WINGET_EXIT_CODE");
        }

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);

        let calls: Vec<String> = calls.lines().map(str::trim).map(String::from).collect();
        (result, calls)
    }
}
