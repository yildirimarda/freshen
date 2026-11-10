//! The `git lfs` step: `git lfs install --skip-repo`, skipped when git-lfs is absent.
//!
//! The step runs `git lfs install --skip-repo` whenever `git lfs` is available.
//! Detection runs `git lfs version`, so a machine that has `git` but not
//! `git-lfs` is correctly reported as not having the tool. The step is skipped
//! entirely when git-lfs is absent, so it is safe to register on every
//! machine. `git lfs install --skip-repo` writes to the user's git
//! configuration, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The `git lfs` step.
pub const GIT_LFS: StepId = "git-lfs";

/// `git lfs` runs on every platform freshen supports.
const GIT_LFS_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the `git lfs` step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(GitLfsStep)
}

struct GitLfsStep;

impl GitLfsStep {
    /// Runs `git lfs install --skip-repo`.
    fn install(&self, failures: &mut Vec<StepError>) {
        human!("git lfs: install --skip-repo");
        if let Err(error) = CommandSpec::new("git")
            .args(["lfs", "install", "--skip-repo"])
            .run()
        {
            failures.push(StepError::run_from_process(GIT_LFS, &error));
        }
    }

    /// Whether `git lfs` is available, probed with `git lfs version`.
    fn detect_git_lfs() -> Detection {
        if CommandSpec::new("git")
            .args(["lfs", "version"])
            .run()
            .is_ok()
        {
            Detection::Installed
        } else {
            Detection::NotInstalled
        }
    }
}

impl Step for GitLfsStep {
    fn id(&self) -> StepId {
        GIT_LFS
    }

    fn display_name(&self) -> &str {
        "git lfs"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(Self::detect_git_lfs())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // `git lfs` has no read-only "what is outdated" query; report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.install(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &GIT_LFS_PLATFORMS
    }

    fn is_user_level(&self) -> bool {
        true
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::GitLfsStep;
    use crate::process::PATH_GUARD;
    use crate::step::Step;
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
            "freshen-git-lfs-{}-{}-{name}",
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

    #[cfg(unix)]
    fn write_executable(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join(name);
        fs::write(&path, body)
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap_or_else(|error| {
            panic!("failed to make {} executable: {error}", path.display())
        });
        path
    }

    #[test]
    fn detect_reports_not_installed_when_git_lfs_is_absent() {
        let dir = temp_dir("absent");
        with_path(&dir, || {
            assert!(!GitLfsStep::detect_git_lfs().is_installed());
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn detect_reports_installed_when_git_lfs_answers_version() {
        let dir = temp_dir("present");
        write_executable(
            &dir,
            "git",
            "#!/bin/sh\n\
             case \"$*\" in\n\
             *lfs\\ version*)\n\
               printf 'git-lfs/2.14.0\\n'\n\
               ;;\n\
             *)\n\
               exit 127\n\
               ;;\n\
             esac\n\
             exit 0\n",
        );
        with_path(&dir, || {
            assert!(GitLfsStep::detect_git_lfs().is_installed());
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn detect_reports_not_installed_when_git_lfs_version_fails() {
        let dir = temp_dir("fails");
        write_executable(&dir, "git", "#!/bin/sh\nexit 128\n");
        with_path(&dir, || {
            assert!(!GitLfsStep::detect_git_lfs().is_installed());
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_is_up_to_date_and_user_level() {
        let step = GitLfsStep;
        let report = step
            .check()
            .unwrap_or_else(|error| panic!("check failed: {error}"));
        assert_eq!(report.pending(), 0);
        assert!(step.is_user_level());
    }
}
