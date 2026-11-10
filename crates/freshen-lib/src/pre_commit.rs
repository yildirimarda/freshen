//! The `pre-commit` step: `pre-commit autoupdate` in the repositories listed
//! in the configuration file.
//!
//! The step runs `pre-commit autoupdate` in every repository configured
//! under `[pre-commit] repositories`. It never scans the disk for
//! repositories: only the listed ones are touched, so the step does nothing
//! until a repository is listed. It is opt-in in exactly that sense: with no
//! repositories configured it runs no command at all.
//!
//! `pre-commit autoupdate` has no read-only "what is outdated" query, so
//! `check()` reports the step up to date, like the `git lfs` step.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The `pre-commit` step.
pub const PRE_COMMIT: StepId = "pre-commit";

/// `pre-commit` runs on every platform freshen supports.
const PRE_COMMIT_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the `pre-commit` step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(PreCommitStep)
}

struct PreCommitStep;

impl PreCommitStep {
    /// Whether `pre-commit` is available, probed by looking for it on `PATH`.
    fn detect_pre_commit() -> Detection {
        if command_exists("pre-commit") {
            Detection::Installed
        } else {
            Detection::NotInstalled
        }
    }
}

impl Step for PreCommitStep {
    fn id(&self) -> StepId {
        PRE_COMMIT
    }

    fn display_name(&self) -> &str {
        "pre-commit"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(Self::detect_pre_commit())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // `pre-commit autoupdate` has no read-only "what is outdated" query;
        // report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        // `run_with_options` is what the runner calls; `run` has no access to
        // the configured repositories, so it is intentionally a no-op here.
        Ok(())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        for repository in options.pre_commit_repositories() {
            human!("pre-commit: autoupdate in {}", repository.display());
            if let Err(error) = CommandSpec::new("pre-commit")
                .args(["autoupdate"])
                .current_dir(repository)
                .run()
            {
                failures.push(StepError::run_from_process(PRE_COMMIT, &error));
            }
        }
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &PRE_COMMIT_PLATFORMS
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
    use super::{PreCommitStep, PRE_COMMIT};
    use crate::platform::Platform;
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
        let dir =
            env::temp_dir().join(format!("freshen-pre-commit-{}-{name}", std::process::id(),));
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
    fn detect_reports_not_installed_when_pre_commit_is_absent() {
        let dir = temp_dir("absent");
        with_path(&dir, || {
            assert!(!PreCommitStep::detect_pre_commit().is_installed());
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn detect_reports_installed_when_pre_commit_answers_version() {
        let dir = temp_dir("present");
        write_executable(
            &dir,
            "pre-commit",
            "#!/bin/sh\n\
             case \"$*\" in\n\
             *--version*)\n\
               printf 'pre-commit 3.8.0\\n'\n\
               ;;\n\
             *)\n\
               exit 127\n\
               ;;\n\
             esac\n\
             exit 0\n",
        );
        with_path(&dir, || {
            assert!(PreCommitStep::detect_pre_commit().is_installed());
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn id_and_platforms() {
        let step = PreCommitStep;
        assert_eq!(step.id(), PRE_COMMIT);
        assert!(step.platforms().contains(&Platform::Linux));
        assert!(step.platforms().contains(&Platform::Windows));
        assert!(step.platforms().contains(&Platform::Macos));
    }

    #[test]
    fn check_is_up_to_date_and_user_level() {
        let step = PreCommitStep;
        let report = step
            .check()
            .unwrap_or_else(|error| panic!("check failed: {error}"));
        assert_eq!(report.pending(), 0);
        assert!(step.is_user_level());
    }
}
