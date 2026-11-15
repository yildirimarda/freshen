//! The `sample` step: a worked example used by the "Writing a step"
//! contributor guide (`docs/writing-a-step.md`). It is not registered in the
//! built-in registry and exists only to prove the step template compiles and
//! behaves as the guide describes.
//!
//! This module is compiled under `#[cfg(test)]` only.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::snapshot::{SnapshotCommand, SnapshotSection};
use crate::step::{CheckReport, Detection, Operation, OutdatedItem, Step, StepError, StepId};

/// The sample tool step.
pub const SAMPLE: StepId = "sample";

/// sample runs on every platform freshen supports.
const SAMPLE_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the sample step (not wired into the built-in registry).
#[must_use]
#[allow(dead_code)]
pub fn register(registry: Registry) -> Registry {
    registry.with(SampleStep)
}

struct SampleStep;

impl SampleStep {
    /// Upgrades every outdated sample package, recording the first failure.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("sample: upgrade");
        if let Err(error) = CommandSpec::new("sample")
            .args(["upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(SAMPLE, &error));
        }
    }
}

impl Step for SampleStep {
    fn id(&self) -> StepId {
        SAMPLE
    }

    fn display_name(&self) -> &str {
        "sample"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_sample())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("sample")
            .args(["--outdated"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(SAMPLE, &error))?;
        Ok(CheckReport::from_items(parse_sample_outdated(
            output.stdout(),
        )))
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn cleanup(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        human!("sample: cache prune");
        if let Err(error) = CommandSpec::new("sample")
            .args(["cache", "prune"])
            .neutral_locale()
            .run()
        {
            failures.push(StepError::from_process(SAMPLE, Operation::Cleanup, &error));
        }
        finish(failures)
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(SnapshotSection::new(
            SAMPLE,
            vec![SnapshotCommand::new("sample", ["list", "--versions"])],
        ))
    }

    fn platforms(&self) -> &'static [Platform] {
        &SAMPLE_PLATFORMS
    }

    /// sample manages user-level packages; running it as root installs into
    /// root's environment instead of the user's.
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

fn detect_sample() -> Detection {
    if crate::executable::command_exists("sample") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses `sample --outdated`: one package per line, `name current latest`.
fn parse_sample_outdated(output: &str) -> Vec<OutdatedItem> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let current = parts.next();
            let latest = parts.next();
            let mut item = OutdatedItem::new(name);
            if let Some(current) = current {
                item = item.with_current(current);
            }
            if let Some(latest) = latest {
                item = item.with_latest(latest);
            }
            Some(item)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_display_name() {
        let step = SampleStep;
        assert_eq!(step.id(), SAMPLE);
        assert_eq!(step.display_name(), "sample");
    }

    #[test]
    fn platforms_and_privilege_flags() {
        let step = SampleStep;
        assert_eq!(step.platforms(), &SAMPLE_PLATFORMS);
        assert!(step.is_user_level());
        assert!(!step.needs_root());
        assert!(!step.needs_administrator());
    }

    #[test]
    fn no_prerequisites_or_conflicts_by_default() {
        let step = SampleStep;
        assert!(step.prerequisites().is_empty());
        assert!(step.conflicts().is_empty());
    }

    #[test]
    fn restore_is_a_no_op_by_default() {
        let step = SampleStep;
        assert!(step.restore().is_ok());
    }

    #[test]
    fn detect_reports_not_installed_when_sample_is_absent() {
        // `sample` is not installed in the test environment, so detection
        // reports NotInstalled and the step is skipped gracefully instead of
        // failing. Run detection under `with_path` with a stub `PATH` that
        // contains no `sample`: the ambient `PATH` cannot be relied on to
        // lack `sample` (macOS ships a real `/usr/bin/sample`), and while a
        // parallel test holds `PATH_GUARD` its stub directory is this test's
        // `PATH`.
        let empty = std::env::temp_dir().join("freshen-sample-absent");
        std::fs::create_dir_all(&empty).unwrap_or_else(|error| {
            panic!(
                "failed to create stub directory {}: {error}",
                empty.display()
            )
        });
        let step = SampleStep;
        assert!(crate::process::with_path(&empty, || step.detect())
            .is_ok_and(|detection| !detection.is_installed()));
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn check_reports_outdated_items() {
        let output = "jq 1.7.1 1.8.0\nripgrep\n";
        let report = CheckReport::from_items(parse_sample_outdated(output));
        assert!(!report.is_up_to_date());
        assert_eq!(report.pending(), 2);

        let mut items = report.items().iter();
        let jq = match items.next() {
            Some(item) => item,
            None => panic!("expected jq in the check report"),
        };
        assert_eq!(jq.name(), "jq");
        assert_eq!(jq.current(), Some("1.7.1"));
        assert_eq!(jq.latest(), Some("1.8.0"));

        let ripgrep = match items.next() {
            Some(item) => item,
            None => panic!("expected ripgrep in the check report"),
        };
        assert_eq!(ripgrep.name(), "ripgrep");
        assert_eq!(ripgrep.current(), None);
        assert_eq!(ripgrep.latest(), None);
    }

    #[test]
    fn empty_output_is_up_to_date() {
        let report = CheckReport::from_items(parse_sample_outdated(""));
        assert!(report.is_up_to_date());
        assert_eq!(report.pending(), 0);
    }

    #[test]
    fn check_maps_a_missing_command_to_a_check_error() {
        // `sample` is not on PATH, so the read-only query fails with a typed
        // StepError tagged with the step and the Check operation.
        let step = SampleStep;
        let error = match step.check() {
            Ok(_) => panic!("check should fail when `sample` is missing"),
            Err(error) => error,
        };
        assert_eq!(error.step(), SAMPLE);
        assert_eq!(error.operation(), Operation::Check);
        assert!(
            error.to_string().starts_with("step `sample`"),
            "unexpected error message: {error}"
        );
    }

    #[test]
    fn run_maps_a_missing_command_to_a_run_error() {
        let step = SampleStep;
        let error = match step.run() {
            Ok(_) => panic!("run should fail when `sample` is missing"),
            Err(error) => error,
        };
        assert_eq!(error.step(), SAMPLE);
        assert_eq!(error.operation(), Operation::Run);
    }

    #[test]
    fn cleanup_maps_a_missing_command_to_a_cleanup_error() {
        let step = SampleStep;
        let error = match step.cleanup() {
            Ok(_) => panic!("cleanup should fail when `sample` is missing"),
            Err(error) => error,
        };
        assert_eq!(error.step(), SAMPLE);
        assert_eq!(error.operation(), Operation::Cleanup);
    }

    #[test]
    fn errors_carry_step_and_operation() {
        let error = StepError::run(SAMPLE, "upgrade failed");
        assert_eq!(error.step(), SAMPLE);
        assert_eq!(error.operation(), Operation::Run);
        assert_eq!(error.message(), "upgrade failed");
        assert_eq!(
            error.to_string(),
            "step `sample` failed during run: upgrade failed"
        );
    }
}
