//! The Snap step: `snap refresh`, skipped when snap is absent.
//!
//! The step runs `snap refresh` whenever `snap` is available. It is skipped
//! entirely when snap is absent, so it is safe to register on every machine;
//! on macOS and Windows it is never detected.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
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
    use super::detect_snap;
    use crate::step::Detection;
    use std::env;
    use std::fs;
    use std::fs::Permissions;
    use std::path::{Path, PathBuf};

    /// Runs `f` with `PATH` replaced by `dir`, restoring the previous `PATH`
    /// afterwards so the test does not depend on what happens to be installed
    /// on the host machine.
    fn with_path<F>(dir: &Path, f: F)
    where
        F: FnOnce(),
    {
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
}
