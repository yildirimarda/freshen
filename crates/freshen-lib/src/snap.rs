//! The Snap step: `snap refresh`, skipped when snap is absent.
//!
//! The step runs `snap refresh` whenever `snap` is available. It is skipped
//! entirely when snap is absent, so it is safe to register on every machine;
//! on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, ProcessError, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Snap package manager step.
pub const SNAP: StepId = "snap";

/// Snap only runs on Linux.
const SNAP_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the snap step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(SnapStep)
}

struct SnapStep;

impl SnapStep {
    /// Runs `snap refresh`.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("snap refresh");
        if let Err(error) = CommandSpec::new("snap")
            .args(["refresh"])
            .retry_transient_network(NETWORK_RETRIES)
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
        Ok(detect_snap())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &SNAP_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

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
fn detect_snap() -> Detection {
    if command_exists("snap") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_snap, held_refresh_window};
    use crate::step::Detection;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// Serializes mutation of the global `PATH` env var so tests that replace
    /// it cannot observe each other's changes while cargo runs tests in
    /// parallel.
    static PATH_GUARD: Mutex<()> = Mutex::new(());

    /// Runs `f` with `PATH` replaced by `dir`, restoring the previous `PATH`
    /// afterwards so the test does not depend on what happens to be installed
    /// on the host machine.
    fn with_path<F>(dir: &Path, f: F)
    where
        F: FnOnce(),
    {
        let _guard = PATH_GUARD
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

    #[test]
    fn detect_snap_reports_not_installed_when_absent_from_path() {
        let dir = temp_dir("absent");
        with_path(&dir, || {
            assert!(!detect_snap().is_installed());
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
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

    #[test]
    fn held_refresh_window_parses_held_until() {
        let held =
            held_refresh_window("error: a snap refresh is held until 2026-03-01T00:00:00+01:00");
        assert_eq!(
            held.as_deref(),
            Some("held until 2026-03-01t00:00:00+01:00")
        );
    }

    #[test]
    fn held_refresh_window_reports_held_without_time() {
        assert_eq!(
            held_refresh_window("error: a snap refresh is held"),
            Some("held".to_string())
        );
    }

    #[test]
    fn held_refresh_window_returns_none_when_not_held() {
        assert!(held_refresh_window("All snaps up to date.").is_none());
        assert!(held_refresh_window("error: network is unreachable").is_none());
        assert!(held_refresh_window("").is_none());
    }

    #[test]
    fn held_refresh_window_is_case_insensitive() {
        let held = held_refresh_window("ERROR: A SNAP REFRESH IS HELD UNTIL 2026-01-01T00:00:00Z");
        assert_eq!(held.as_deref(), Some("held until 2026-01-01t00:00:00z"));
    }
}
