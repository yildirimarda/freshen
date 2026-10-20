//! Executing a run and summarising what happened.
//!
//! The zsh implementation records a failed command, warns about it and keeps
//! going, so one broken package manager never prevents the rest of the machine
//! from being updated. This module keeps that behaviour: every step runs in
//! order, failures are collected instead of aborting the run, and the caller
//! receives one status and duration per step.
//!
//! Exit-code semantics match `update-all.sh`: `0` when no step failed and `1`
//! when at least one did. A skipped step (its tool is not installed) is not a
//! failure and therefore does not change the exit code.

use std::fmt;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::options::RunOptions;
use crate::step::{Detection, Step, StepError, StepId};

/// The outcome of one step in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepStatus {
    /// The step ran (including its cleanup) successfully.
    Ok,
    /// The tool is installed but `--dry-run` prevented the upgrade.
    Planned,
    /// The step did not run because its tool is not installed.
    Skipped,
    /// Detection, the upgrade or the cleanup failed.
    Failed,
}

impl StepStatus {
    /// Lowercase name used in the summary.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Planned => "planned",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    /// Returns `true` for [`StepStatus::Failed`].
    #[must_use]
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed)
    }
}

impl fmt::Display for StepStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a step was skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// The tool managed by the step is not installed on this machine.
    NotInstalled,
}

impl SkipReason {
    /// Human-readable explanation shown in the summary.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not installed",
        }
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The collected result of one step: status, duration and failure detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    id: StepId,
    name: String,
    status: StepStatus,
    duration: Duration,
    skip_reason: Option<SkipReason>,
    error: Option<StepError>,
}

impl StepResult {
    fn new(step: &dyn Step, status: StepStatus, duration: Duration) -> Self {
        Self {
            id: step.id(),
            name: step.display_name().to_owned(),
            status,
            duration,
            skip_reason: None,
            error: None,
        }
    }

    fn ok(step: &dyn Step, duration: Duration) -> Self {
        Self::new(step, StepStatus::Ok, duration)
    }

    fn planned(step: &dyn Step, duration: Duration) -> Self {
        Self::new(step, StepStatus::Planned, duration)
    }

    fn skipped(step: &dyn Step, duration: Duration) -> Self {
        Self {
            skip_reason: Some(SkipReason::NotInstalled),
            ..Self::new(step, StepStatus::Skipped, duration)
        }
    }

    fn failed(step: &dyn Step, duration: Duration, error: StepError) -> Self {
        Self {
            error: Some(error),
            ..Self::new(step, StepStatus::Failed, duration)
        }
    }

    /// Stable id of the step.
    #[must_use]
    pub fn id(&self) -> StepId {
        self.id
    }

    /// Name shown to the user.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Outcome of the step.
    #[must_use]
    pub fn status(&self) -> StepStatus {
        self.status
    }

    /// Wall-clock time spent detecting, running and cleaning up the step.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Why the step was skipped, when it was.
    #[must_use]
    pub fn skip_reason(&self) -> Option<SkipReason> {
        self.skip_reason
    }

    /// The failure that stopped the step, when it failed.
    #[must_use]
    pub fn error(&self) -> Option<&StepError> {
        self.error.as_ref()
    }
}

/// The final summary of a run: one [`StepResult`] per step, in run order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunSummary {
    steps: Vec<StepResult>,
}

impl RunSummary {
    fn from_results(steps: Vec<StepResult>) -> Self {
        Self { steps }
    }

    /// Results of every step that was part of the run, in execution order.
    #[must_use]
    pub fn steps(&self) -> &[StepResult] {
        &self.steps
    }

    /// Number of steps with the given status.
    #[must_use]
    pub fn count(&self, status: StepStatus) -> usize {
        self.steps
            .iter()
            .filter(|result| result.status() == status)
            .count()
    }

    /// Number of steps that completed successfully.
    #[must_use]
    pub fn ok_count(&self) -> usize {
        self.count(StepStatus::Ok)
    }

    /// Number of steps that `--dry-run` planned but did not execute.
    #[must_use]
    pub fn planned_count(&self) -> usize {
        self.count(StepStatus::Planned)
    }

    /// Number of steps whose tool was not installed.
    #[must_use]
    pub fn skipped_count(&self) -> usize {
        self.count(StepStatus::Skipped)
    }

    /// Number of steps that failed.
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.count(StepStatus::Failed)
    }

    /// Returns `true` when at least one step failed.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.failed_count() > 0
    }

    /// Results of the steps that failed.
    pub fn failures(&self) -> impl Iterator<Item = &StepResult> {
        self.steps
            .iter()
            .filter(|result| result.status().is_failure())
    }

    /// Sum of the per-step durations.
    #[must_use]
    pub fn total_duration(&self) -> Duration {
        self.steps.iter().map(StepResult::duration).sum()
    }

    /// Process exit code matching the zsh script: `0` unless a step failed.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        if self.has_failures() {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }
}

impl fmt::Display for RunSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Summary: {} ok, {} planned, {} skipped, {} failed ({})",
            self.ok_count(),
            self.planned_count(),
            self.skipped_count(),
            self.failed_count(),
            format_duration(self.total_duration())
        )?;
        for result in &self.steps {
            write!(
                f,
                "  {:<7} {} ({})",
                result.status(),
                result.name(),
                format_duration(result.duration())
            )?;
            if let Some(reason) = result.skip_reason() {
                write!(f, " - {reason}")?;
            }
            if let Some(error) = result.error() {
                write!(f, " - {error}")?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

fn format_duration(duration: Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{:.2}s", duration.as_secs_f64())
    } else if duration.as_millis() > 0 {
        format!("{}ms", duration.as_millis())
    } else if duration.as_micros() > 0 {
        format!("{}µs", duration.as_micros())
    } else {
        format!("{}ns", duration.as_nanos())
    }
}

/// Runs an ordered list of steps, collecting failures instead of aborting.
///
/// Each step is detected first. A missing tool skips the step; an installed
/// tool is run and, when that succeeds, cleaned up. A failure in any of those
/// phases is recorded and the next step still runs. With `--dry-run`, an
/// installed tool is detected and recorded as planned, but never run or
/// cleaned up.
#[derive(Clone)]
pub struct Runner<'a> {
    steps: Vec<&'a dyn Step>,
    options: RunOptions,
}

impl<'a> Runner<'a> {
    /// Creates a runner for steps that are already in execution order.
    pub fn new(steps: impl IntoIterator<Item = &'a dyn Step>) -> Self {
        Self::with_options(steps, RunOptions::new())
    }

    /// Creates a runner that applies `options`, including `--dry-run` and the
    /// `--only`/`--skip` selection.
    pub fn with_options(
        steps: impl IntoIterator<Item = &'a dyn Step>,
        options: RunOptions,
    ) -> Self {
        let selection = options.selection();
        Self {
            steps: steps
                .into_iter()
                .filter(|step| selection.includes(step.id()))
                .collect(),
            options,
        }
    }

    /// Options that control this run.
    #[must_use]
    pub fn options(&self) -> &RunOptions {
        &self.options
    }

    /// Runs every step and returns the collected results.
    #[must_use]
    pub fn run(&self) -> RunSummary {
        let dry_run = self.options.is_dry_run();
        RunSummary::from_results(
            self.steps
                .iter()
                .map(|step| run_step(*step, dry_run))
                .collect(),
        )
    }
}

fn run_step(step: &dyn Step, dry_run: bool) -> StepResult {
    let started = Instant::now();
    match step.detect() {
        Ok(Detection::NotInstalled) => StepResult::skipped(step, started.elapsed()),
        Ok(Detection::Installed) if dry_run => StepResult::planned(step, started.elapsed()),
        Ok(Detection::Installed) => match step.run() {
            Ok(()) => match step.cleanup() {
                Ok(()) => StepResult::ok(step, started.elapsed()),
                Err(error) => StepResult::failed(step, started.elapsed(), error),
            },
            Err(error) => StepResult::failed(step, started.elapsed(), error),
        },
        Err(error) => StepResult::failed(step, started.elapsed(), error),
    }
}

#[cfg(test)]
mod tests {
    use super::{RunSummary, Runner, SkipReason, StepStatus};
    use crate::options::{RunOptions, StepSelection};
    use crate::platform::Platform;
    use crate::step::{CheckReport, Detection, Step, StepError, StepId};
    use std::cell::Cell;
    use std::process::ExitCode;
    use std::rc::Rc;
    use std::time::Duration;

    #[derive(Default)]
    struct Calls {
        detect: Cell<usize>,
        run: Cell<usize>,
        cleanup: Cell<usize>,
    }

    struct FakeStep {
        id: StepId,
        name: &'static str,
        detection: Detection,
        fail_detect: bool,
        fail_run: bool,
        fail_cleanup: bool,
        sleep: Duration,
        calls: Rc<Calls>,
    }

    impl FakeStep {
        fn new(id: StepId, name: &'static str) -> Self {
            Self {
                id,
                name,
                detection: Detection::Installed,
                fail_detect: false,
                fail_run: false,
                fail_cleanup: false,
                sleep: Duration::ZERO,
                calls: Rc::new(Calls::default()),
            }
        }

        fn detection(mut self, detection: Detection) -> Self {
            self.detection = detection;
            self
        }

        fn fail_detect(mut self) -> Self {
            self.fail_detect = true;
            self
        }

        fn fail_run(mut self) -> Self {
            self.fail_run = true;
            self
        }

        fn fail_cleanup(mut self) -> Self {
            self.fail_cleanup = true;
            self
        }

        fn sleep(mut self, duration: Duration) -> Self {
            self.sleep = duration;
            self
        }

        fn calls(&self) -> Rc<Calls> {
            Rc::clone(&self.calls)
        }
    }

    impl Step for FakeStep {
        fn id(&self) -> StepId {
            self.id
        }

        fn display_name(&self) -> &str {
            self.name
        }

        fn detect(&self) -> Result<Detection, StepError> {
            self.calls.detect.set(self.calls.detect.get() + 1);
            if self.fail_detect {
                return Err(StepError::detect(self.id, "cannot detect"));
            }
            Ok(self.detection)
        }

        fn check(&self) -> Result<CheckReport, StepError> {
            Ok(CheckReport::up_to_date())
        }

        fn run(&self) -> Result<(), StepError> {
            self.calls.run.set(self.calls.run.get() + 1);
            if !self.sleep.is_zero() {
                std::thread::sleep(self.sleep);
            }
            if self.fail_run {
                return Err(StepError::run(self.id, "upgrade failed"));
            }
            Ok(())
        }

        fn cleanup(&self) -> Result<(), StepError> {
            self.calls.cleanup.set(self.calls.cleanup.get() + 1);
            if self.fail_cleanup {
                return Err(StepError::cleanup(self.id, "cleanup failed"));
            }
            Ok(())
        }

        fn platforms(&self) -> &'static [Platform] {
            &Platform::ALL
        }
    }

    fn run(steps: &[&dyn Step]) -> RunSummary {
        Runner::new(steps.iter().copied()).run()
    }

    #[test]
    fn runs_installed_steps_in_order_and_cleans_up() {
        let first = FakeStep::new("brew", "Homebrew");
        let second = FakeStep::new("uv", "uv");
        let first_calls = first.calls();
        let second_calls = second.calls();

        let summary = run(&[&first, &second]);

        assert_eq!(summary.steps().len(), 2);
        assert_eq!(summary.steps()[0].id(), "brew");
        assert_eq!(summary.steps()[0].name(), "Homebrew");
        assert_eq!(summary.steps()[0].status(), StepStatus::Ok);
        assert_eq!(summary.steps()[1].id(), "uv");
        assert_eq!(summary.ok_count(), 2);
        assert_eq!(summary.skipped_count(), 0);
        assert_eq!(summary.failed_count(), 0);
        assert!(!summary.has_failures());
        assert_eq!(summary.exit_code(), ExitCode::SUCCESS);

        assert_eq!(first_calls.detect.get(), 1);
        assert_eq!(first_calls.run.get(), 1);
        assert_eq!(first_calls.cleanup.get(), 1);
        assert_eq!(second_calls.run.get(), 1);
        assert_eq!(second_calls.cleanup.get(), 1);
    }

    #[test]
    fn a_missing_tool_is_skipped_and_never_run() {
        let step = FakeStep::new("uv", "uv").detection(Detection::NotInstalled);
        let calls = step.calls();

        let summary = run(&[&step]);

        let result = &summary.steps()[0];
        assert_eq!(result.status(), StepStatus::Skipped);
        assert_eq!(result.skip_reason(), Some(SkipReason::NotInstalled));
        assert_eq!(result.error(), None);
        assert_eq!(summary.skipped_count(), 1);
        assert_eq!(summary.ok_count(), 0);
        assert_eq!(summary.failed_count(), 0);
        assert_eq!(summary.exit_code(), ExitCode::SUCCESS);

        assert_eq!(calls.detect.get(), 1);
        assert_eq!(calls.run.get(), 0);
        assert_eq!(calls.cleanup.get(), 0);
    }

    #[test]
    fn dry_run_detects_but_never_runs_or_cleans_up() {
        let installed = FakeStep::new("brew", "Homebrew");
        let missing = FakeStep::new("uv", "uv").detection(Detection::NotInstalled);
        let calls = installed.calls();
        let steps: [&dyn Step; 2] = [&installed, &missing];

        let summary = Runner::with_options(steps, RunOptions::new().with_dry_run(true)).run();

        assert_eq!(summary.steps()[0].status(), StepStatus::Planned);
        assert_eq!(summary.planned_count(), 1);
        assert_eq!(summary.ok_count(), 0);
        assert_eq!(summary.steps()[1].status(), StepStatus::Skipped);
        assert_eq!(summary.skipped_count(), 1);
        assert_eq!(summary.failed_count(), 0);
        assert_eq!(summary.exit_code(), ExitCode::SUCCESS);
        assert!(summary.to_string().contains("1 planned"));

        assert_eq!(calls.detect.get(), 1);
        assert_eq!(calls.run.get(), 0);
        assert_eq!(calls.cleanup.get(), 0);
    }

    #[test]
    fn runner_applies_only_and_skip_from_options() {
        let brew = FakeStep::new("brew", "Homebrew");
        let uv = FakeStep::new("uv", "uv");
        let mise = FakeStep::new("mise", "mise");
        let steps: [&dyn Step; 3] = [&brew, &uv, &mise];
        let options = RunOptions::new().with_selection(
            StepSelection::new()
                .with_only(["brew", "uv", "mise"])
                .with_skip(["uv"]),
        );

        let summary = Runner::with_options(steps, options).run();

        assert_eq!(summary.steps().len(), 2);
        assert_eq!(summary.steps()[0].id(), "brew");
        assert_eq!(summary.steps()[1].id(), "mise");
    }

    #[test]
    fn dry_run_still_reports_a_detection_failure() {
        let failing = FakeStep::new("mise", "mise").fail_detect();

        let summary = Runner::with_options(
            [&failing as &dyn Step],
            RunOptions::new().with_dry_run(true),
        )
        .run();

        assert_eq!(summary.steps()[0].status(), StepStatus::Failed);
        assert_eq!(summary.planned_count(), 0);
        assert_eq!(summary.exit_code(), ExitCode::FAILURE);
    }

    #[test]
    fn a_failing_step_is_recorded_and_does_not_abort_the_run() {
        let failing = FakeStep::new("brew", "Homebrew").fail_run();
        let healthy = FakeStep::new("uv", "uv");
        let failing_calls = failing.calls();
        let healthy_calls = healthy.calls();

        let summary = run(&[&failing, &healthy]);

        assert_eq!(summary.steps().len(), 2);
        assert_eq!(summary.failed_count(), 1);
        assert_eq!(summary.ok_count(), 1);
        assert!(summary.has_failures());
        assert_eq!(summary.exit_code(), ExitCode::FAILURE);
        assert_eq!(summary.failures().count(), 1);

        let result = &summary.steps()[0];
        assert_eq!(result.status(), StepStatus::Failed);
        assert_eq!(
            result.error().map(StepError::operation),
            Some(crate::step::Operation::Run)
        );
        assert_eq!(failing_calls.cleanup.get(), 0);

        assert_eq!(healthy_calls.run.get(), 1);
        assert_eq!(healthy_calls.cleanup.get(), 1);
        assert_eq!(summary.steps()[1].status(), StepStatus::Ok);
    }

    #[test]
    fn a_detection_error_is_a_failure_and_the_step_does_not_run() {
        let step = FakeStep::new("mise", "mise").fail_detect();
        let calls = step.calls();

        let summary = run(&[&step]);

        let result = &summary.steps()[0];
        assert_eq!(result.status(), StepStatus::Failed);
        assert_eq!(
            result.error().map(StepError::operation),
            Some(crate::step::Operation::Detect)
        );
        assert_eq!(calls.detect.get(), 1);
        assert_eq!(calls.run.get(), 0);
        assert_eq!(calls.cleanup.get(), 0);
        assert_eq!(summary.exit_code(), ExitCode::FAILURE);
    }

    #[test]
    fn a_cleanup_error_is_a_failure() {
        let step = FakeStep::new("brew", "Homebrew").fail_cleanup();
        let calls = step.calls();

        let summary = run(&[&step]);

        let result = &summary.steps()[0];
        assert_eq!(result.status(), StepStatus::Failed);
        assert_eq!(
            result.error().map(StepError::operation),
            Some(crate::step::Operation::Cleanup)
        );
        assert_eq!(calls.run.get(), 1);
        assert_eq!(calls.cleanup.get(), 1);
        assert_eq!(summary.exit_code(), ExitCode::FAILURE);
    }

    #[test]
    fn records_a_duration_per_step_and_sums_them() {
        let slow = FakeStep::new("brew", "Homebrew").sleep(Duration::from_millis(5));
        let skipped = FakeStep::new("uv", "uv").detection(Detection::NotInstalled);

        let summary = run(&[&slow, &skipped]);

        assert!(summary.steps()[0].duration() >= Duration::from_millis(1));
        let summed: Duration = summary.steps().iter().map(|step| step.duration()).sum();
        assert_eq!(summary.total_duration(), summed);
    }

    #[test]
    fn skipped_steps_alone_do_not_fail_the_run() {
        let first = FakeStep::new("uv", "uv").detection(Detection::NotInstalled);
        let second = FakeStep::new("mise", "mise").detection(Detection::NotInstalled);

        let summary = run(&[&first, &second]);

        assert_eq!(summary.skipped_count(), 2);
        assert!(!summary.has_failures());
        assert_eq!(summary.exit_code(), ExitCode::SUCCESS);
    }

    #[test]
    fn an_empty_run_succeeds() {
        let summary = Runner::new(std::iter::empty()).run();

        assert!(summary.steps().is_empty());
        assert!(!summary.has_failures());
        assert_eq!(summary.total_duration(), Duration::ZERO);
        assert_eq!(summary.exit_code(), ExitCode::SUCCESS);
    }

    #[test]
    fn the_summary_lists_counts_statuses_and_details() {
        let ok = FakeStep::new("brew", "Homebrew");
        let skipped = FakeStep::new("uv", "uv").detection(Detection::NotInstalled);
        let failed = FakeStep::new("mise", "mise").fail_run();

        let summary = run(&[&ok, &skipped, &failed]);
        let text = summary.to_string();

        assert!(text.contains("1 ok, 0 planned, 1 skipped, 1 failed"));
        assert!(text.contains("ok"));
        assert!(text.contains("Homebrew"));
        assert!(text.contains("skipped"));
        assert!(text.contains("uv"));
        assert!(text.contains("not installed"));
        assert!(text.contains("failed"));
        assert!(text.contains("mise"));
        assert!(text.contains("upgrade failed"));
    }
}
