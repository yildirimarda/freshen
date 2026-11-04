//! The Snap step: `snap refresh`, skipped when snap is absent.
//!
//! The step runs `snap refresh` whenever `snap` is available. It is skipped
//! entirely when snap is absent, so it is safe to register on every machine;
//! on macOS and Windows it is never detected.

#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::executable::command_exists;
use crate::options::RunOptions;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::output::human;
use crate::platform::Platform;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::process::{CommandSpec, ProcessError, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::wsl::inside_command;

/// The Snap package manager step.
pub const SNAP: StepId = "snap";

/// Snap runs on Linux natively and, on Windows, inside a selected WSL
/// distribution via `wsl -d <distro> snap ...`.
const SNAP_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// Registers the snap step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(SnapStep)
}

struct SnapStep;

impl SnapStep {
    /// Runs `snap refresh`. On Windows the command runs inside the selected
    /// WSL distribution via `wsl -d <distro>`.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn upgrade(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        human!("snap refresh");
        if let Err(error) = inside_command(
            options,
            CommandSpec::new("snap")
                .args(["refresh"])
                .retry_transient_network(NETWORK_RETRIES),
        )
        .run()
        {
            let ProcessError::Failed { output, .. } = &error else {
                failures.push(StepError::run_from_process(SNAP, &error));
                return;
            };
            let text = format!("{} {}", output.stdout(), output.stderr());
            if let Some(held) = held_refresh_window(&text) {
                human!("snap refresh is {held}; skipping the refresh");
                return;
            }
            failures.push(StepError::run_from_process(SNAP, &error));
        }
    }
}

impl Step for SnapStep {
    fn id(&self) -> StepId {
        SNAP
    }

    fn display_name(&self) -> &str {
        "snap"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        #[cfg(target_os = "linux")]
        return Ok(detect_snap());
        #[cfg(target_os = "windows")]
        return Ok(detect_wsl());
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        return Ok(Detection::NotInstalled);
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    #[allow(unused_variables)]
    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        #[cfg(target_os = "windows")]
        {
            let Some(_distro) = crate::wsl::selected_distro(options) else {
                human!("snap: no WSL distribution selected; pass --wsl-distro <distro>");
                return Ok(());
            };
            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);
            finish(failures)
        }

        #[cfg(target_os = "linux")]
        {
            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);
            finish(failures)
        }

        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            // snap is not available on macOS; nothing to run.
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &SNAP_PLATFORMS
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn held_refresh_window(text: &str) -> Option<String> {
    let lowered = text.to_ascii_lowercase();
    if !lowered.contains("held") {
        return None;
    }
    if let Some((_before, rest)) = lowered.split_once("held until") {
        let until = rest.split_whitespace().next()?;
        return Some(format!("held until {until}"));
    }
    Some("held".to_string())
}

/// Whether the `snap` binary is discoverable on `PATH`.
#[cfg(target_os = "linux")]
fn detect_snap() -> Detection {
    if command_exists("snap") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Whether the `wsl` binary is discoverable on `PATH` (Windows only).
#[cfg(target_os = "windows")]
fn detect_wsl() -> Detection {
    if command_exists("wsl") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::detect_snap;
    use super::held_refresh_window;
    use crate::step::Detection;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Runs `f` with `PATH` replaced by `dir`, restoring the previous `PATH`
    /// afterwards so the test does not depend on what happens to be installed
    /// on the host machine.
    fn with_path<F>(dir: &Path, f: F)
    where
        F: FnOnce(),
    {
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = env::var_os("PATH");
        env::set_var("PATH", dir);
        f();
        match previous {
            Some(value) => env::set_var("PATH", value),
            None => env::remove_var("PATH"),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "freshen-snap-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
        dir
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn detect_snap_reports_not_installed_when_absent_from_path() {
        let dir = temp_dir("absent");
        with_path(&dir, || {
            assert!(!detect_snap().is_installed());
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn detect_snap_reports_installed_when_on_path() {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("present");
        let program = dir.join("snap");
        fs::write(&program, "#!/bin/sh\nexit 0\n")
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", program.display()));
        fs::set_permissions(&program, Permissions::from_mode(0o755)).unwrap_or_else(|error| {
            panic!("failed to make {} executable: {error}", program.display())
        });

        with_path(&dir, || {
            assert!(detect_snap().is_installed());
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn held_refresh_window_parses_held_until() {
        let held =
            held_refresh_window("error: a snap refresh is held until 2026-03-01T00:00:00+01:00");
        assert_eq!(
            held.as_deref(),
            Some("held until 2026-03-01t00:00:00+01:00")
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn held_refresh_window_reports_held_without_time() {
        assert_eq!(
            held_refresh_window("error: a snap refresh is held"),
            Some("held".to_string())
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn held_refresh_window_returns_none_when_not_held() {
        assert!(held_refresh_window("All snaps up to date.").is_none());
        assert!(held_refresh_window("error: network is unreachable").is_none());
        assert!(held_refresh_window("").is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn held_refresh_window_is_case_insensitive() {
        let held = held_refresh_window("ERROR: A SNAP REFRESH IS HELD UNTIL 2026-01-01T00:00:00Z");
        assert_eq!(held.as_deref(), Some("held until 2026-01-01t00:00:00z"));
    }
}
