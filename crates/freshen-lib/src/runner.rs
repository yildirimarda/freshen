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

use std::collections::HashMap;
use std::fmt;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::check::{CheckResult, CheckSummary};
use crate::options::{RunOptions, SelectionFlag};
use crate::snapshot::SnapshotRecorder;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

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
    /// The step does not support the platform of the run.
    UnsupportedPlatform,
    /// `--only` or `--skip` excluded the step from the run.
    NotSelected(SelectionFlag),
    /// The step conflicts with an earlier step that is part of the run.
    Conflict(StepId),
}

impl SkipReason {
    /// Human-readable explanation shown in the summary.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not installed",
            Self::UnsupportedPlatform => "not supported on this platform",
            Self::NotSelected(SelectionFlag::Only) => "not selected by --only",
            Self::NotSelected(SelectionFlag::Skip) => "excluded by --skip",
            Self::Conflict(_) => "conflicts with another step",
        }
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(conflict) => write!(f, "conflicts with `{conflict}`"),
            other => f.write_str(other.as_str()),
        }
    }
}

/// One package, tool or artifact whose installed version changed during a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionChange {
    name: String,
    before: Option<String>,
    after: Option<String>,
}

impl VersionChange {
    /// Creates a change for `name` from `before` to `after`.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        before: Option<impl Into<String>>,
        after: Option<impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            before: before.map(Into::into),
            after: after.map(Into::into),
        }
    }

    /// Name of the package, tool or artifact.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Version before the step ran, when known.
    #[must_use]
    pub fn before(&self) -> Option<&str> {
        self.before.as_deref()
    }

    /// Version after the step ran, when known.
    #[must_use]
    pub fn after(&self) -> Option<&str> {
        self.after.as_deref()
    }
}

impl fmt::Display for VersionChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)?;
        match (&self.before, &self.after) {
            (Some(before), Some(after)) => write!(f, " {before} -> {after}"),
            (Some(before), None) => write!(f, " {before} -> ?"),
            (None, Some(after)) => write!(f, " ? -> {after}"),
            (None, None) => Ok(()),
        }
    }
}

/// The collected result of one step: status, duration, version changes and
/// failure detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    id: StepId,
    name: String,
    status: StepStatus,
    duration: Duration,
    changes: Vec<VersionChange>,
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
            changes: Vec::new(),
            skip_reason: None,
            error: None,
        }
    }

    fn with_changes(mut self, changes: Vec<VersionChange>) -> Self {
        self.changes = changes;
        self
    }

    fn ok(step: &dyn Step, duration: Duration) -> Self {
        Self::new(step, StepStatus::Ok, duration)
    }

    fn planned(step: &dyn Step, duration: Duration) -> Self {
        Self::new(step, StepStatus::Planned, duration)
    }

    fn skipped(step: &dyn Step, duration: Duration) -> Self {
        Self::skipped_because(step, SkipReason::NotInstalled, duration)
    }

    fn skipped_because(step: &dyn Step, reason: SkipReason, duration: Duration) -> Self {
        Self {
            skip_reason: Some(reason),
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

    /// Versions that changed while the step ran, in report order.
    #[must_use]
    pub fn changes(&self) -> &[VersionChange] {
        &self.changes
    }

    /// Returns `true` when at least one version changed during the step.
    #[must_use]
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
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

    /// Renders the summary as a JSON report.
    #[must_use]
    pub fn to_json(&self) -> String {
        crate::report::run_json(self)
    }

    /// Renders the summary as a Markdown report.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        crate::report::run_markdown(self)
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
            for change in result.changes() {
                writeln!(f, "          - {change}")?;
            }
        }
        Ok(())
    }
}

pub(crate) fn format_duration(duration: Duration) -> String {
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
///
/// When two steps that are part of the run declare a conflict, the later one
/// is skipped without being detected or run, and its [`SkipReason::Conflict`]
/// names the earlier step that won.
#[derive(Clone)]
pub struct Runner<'a> {
    steps: Vec<PlannedStep<'a>>,
    options: RunOptions,
}

#[derive(Clone, Copy)]
enum PlannedStep<'a> {
    Run(&'a dyn Step),
    Skip {
        step: &'a dyn Step,
        reason: SkipReason,
    },
}

impl<'a> PlannedStep<'a> {
    fn running(self) -> Option<&'a dyn Step> {
        match self {
            Self::Run(step) => Some(step),
            Self::Skip { .. } => None,
        }
    }
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
        let mut planned = Vec::new();

        for step in steps
            .into_iter()
            .filter(|step| selection.includes(step.id()))
        {
            let conflict = planned
                .iter()
                .copied()
                .filter_map(PlannedStep::running)
                .find(|earlier| {
                    earlier.conflicts().contains(&step.id())
                        || step.conflicts().contains(&earlier.id())
                })
                .map(|earlier| earlier.id());

            match conflict {
                Some(earlier) => planned.push(PlannedStep::Skip {
                    step,
                    reason: SkipReason::Conflict(earlier),
                }),
                None => planned.push(PlannedStep::Run(step)),
            }
        }

        Self {
            steps: planned,
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
        let mut snapshots = SnapshotRecorder::new();

        RunSummary::from_results(
            self.steps
                .iter()
                .copied()
                .map(|planned| match planned {
                    PlannedStep::Run(step) => run_step(step, &self.options, &mut snapshots),
                    PlannedStep::Skip { step, reason } => {
                        StepResult::skipped_because(step, reason, Duration::ZERO)
                    }
                })
                .collect(),
        )
    }

    /// Detects every step and asks each installed one what is outdated,
    /// without running or cleaning up anything.
    #[must_use]
    pub fn check(&self) -> CheckSummary {
        CheckSummary::from_results(
            self.steps
                .iter()
                .copied()
                .map(|planned| match planned {
                    PlannedStep::Run(step) => check_step(step),
                    PlannedStep::Skip { step, reason } => {
                        CheckResult::skipped(step, reason, Duration::ZERO)
                    }
                })
                .collect(),
        )
    }
}

fn check_step(step: &dyn Step) -> CheckResult {
    let started = Instant::now();
    match step.detect() {
        Ok(Detection::NotInstalled) => {
            CheckResult::skipped(step, SkipReason::NotInstalled, started.elapsed())
        }
        Ok(Detection::Installed) => match step.check() {
            Ok(report) if report.is_up_to_date() => {
                CheckResult::up_to_date(step, started.elapsed())
            }
            Ok(report) => CheckResult::outdated(step, report, started.elapsed()),
            Err(error) => CheckResult::failed(step, error, started.elapsed()),
        },
        Err(error) => CheckResult::failed(step, error, started.elapsed()),
    }
}

fn run_step(step: &dyn Step, options: &RunOptions, snapshots: &mut SnapshotRecorder) -> StepResult {
    let started = Instant::now();
    match step.detect() {
        Ok(Detection::NotInstalled) => StepResult::skipped(step, started.elapsed()),
        Ok(Detection::Installed) if options.is_dry_run() => {
            StepResult::planned(step, started.elapsed())
        }
        Ok(Detection::Installed) => {
            capture_snapshot(step, snapshots);
            let before = step.check().ok();
            let result = match step.run_with_options(options) {
                Ok(()) => match step.cleanup() {
                    Ok(()) => StepResult::ok(step, started.elapsed()),
                    Err(error) => StepResult::failed(step, started.elapsed(), error),
                },
                Err(error) => StepResult::failed(step, started.elapsed(), error),
            };

            let changes = match before {
                Some(before) if !before.is_up_to_date() => match step.check() {
                    Ok(after) => diff_reports(&before, &after),
                    Err(_) => Vec::new(),
                },
                _ => Vec::new(),
            };

            result.with_changes(changes)
        }
        Err(error) => StepResult::failed(step, started.elapsed(), error),
    }
}

fn capture_snapshot(step: &dyn Step, snapshots: &mut SnapshotRecorder) {
    let Some(section) = step.snapshot() else {
        return;
    };

    match snapshots.record(section) {
        Ok(true) => {
            if let Some(path) = snapshots.path() {
                eprintln!(
                    "freshen: pre-upgrade snapshot written to {}",
                    path.display()
                );
            }
        }
        Ok(false) => {}
        Err(error) => eprintln!("freshen: could not write pre-upgrade snapshot: {error}"),
    }
}

/// Compares the outdated items a step reported before and after it ran.
///
/// An item that disappeared from the report was upgraded to the version that
/// was pending before the run. An item that is still outdated but now has a
/// different installed version changed to that version. Items that were not
/// outdated before are ignored: they did not change because of this step.
fn diff_reports(before: &CheckReport, after: &CheckReport) -> Vec<VersionChange> {
    let after_by_name: HashMap<&str, &OutdatedItem> = after
        .items()
        .iter()
        .map(|item| (item.name(), item))
        .collect();

    before
        .items()
        .iter()
        .filter_map(|item| {
            let after_item = after_by_name.get(item.name());

            if let Some(after_item) = after_item {
                if after_item.current() == item.current() && after_item.latest() == item.latest() {
                    return None;
                }
            }

            let after_version = match after_item {
                Some(after_item) => after_item.current(),
                None => item.latest(),
            };

            if item.current() == after_version {
                return None;
            }
            if item.current().is_none() && after_version.is_none() {
                return None;
            }

            Some(VersionChange::new(
                item.name(),
                item.current().map(str::to_owned),
                after_version.map(str::to_owned),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{RunSummary, Runner, SkipReason, StepStatus};
    use crate::check::CheckStatus;
    use crate::options::{RunOptions, StepSelection};
    use crate::platform::Platform;
    use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};
    use std::cell::Cell;
    use std::process::ExitCode;
    use std::rc::Rc;
    use std::time::Duration;

    #[derive(Default)]
    struct Calls {
        detect: Cell<usize>,
        check: Cell<usize>,
        run: Cell<usize>,
        cleanup: Cell<usize>,
    }

    struct FakeStep {
        id: StepId,
        name: &'static str,
        detection: Detection,
        report: CheckReport,
        fail_detect: bool,
        fail_check: bool,
        fail_run: bool,
        fail_cleanup: bool,
        conflicts: &'static [StepId],
        sleep: Duration,
        calls: Rc<Calls>,
    }

    impl FakeStep {
        fn new(id: StepId, name: &'static str) -> Self {
            Self {
                id,
                name,
                detection: Detection::Installed,
                report: CheckReport::up_to_date(),
                fail_detect: false,
                fail_check: false,
                fail_run: false,
                fail_cleanup: false,
                conflicts: &[],
                sleep: Duration::ZERO,
                calls: Rc::new(Calls::default()),
            }
        }

        fn detection(mut self, detection: Detection) -> Self {
            self.detection = detection;
            self
        }

        fn report(mut self, report: CheckReport) -> Self {
            self.report = report;
            self
        }

        fn conflicting(mut self, conflicts: &'static [StepId]) -> Self {
            self.conflicts = conflicts;
            self
        }

        fn fail_detect(mut self) -> Self {
            self.fail_detect = true;
            self
        }

        fn fail_check(mut self) -> Self {
            self.fail_check = true;
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
            self.calls.check.set(self.calls.check.get() + 1);
            if self.fail_check {
                return Err(StepError::check(self.id, "cannot check"));
            }
            Ok(self.report.clone())
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

        fn conflicts(&self) -> &'static [StepId] {
            self.conflicts
        }

        fn platforms(&self) -> &'static [Platform] {
            &Platform::ALL
        }
    }

    fn run(steps: &[&dyn Step]) -> RunSummary {
        Runner::new(steps.iter().copied()).run()
    }

    fn assert_exit_code(actual: ExitCode, expected: ExitCode) {
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
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
        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);

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
        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);

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
        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);
        assert!(summary.to_string().contains("1 planned"));

        assert_eq!(calls.detect.get(), 1);
        assert_eq!(calls.run.get(), 0);
        assert_eq!(calls.cleanup.get(), 0);
    }

    #[test]
    fn check_queries_installed_steps_without_running_or_cleaning_up() {
        let outdated =
            FakeStep::new("brew", "Homebrew").report(CheckReport::from_items([OutdatedItem::new(
                "jq",
            )
            .with_current("1.7.1")
            .with_latest("1.8.0")]));
        let current = FakeStep::new("uv", "uv");
        let missing = FakeStep::new("mise", "mise").detection(Detection::NotInstalled);
        let outdated_calls = outdated.calls();
        let current_calls = current.calls();
        let missing_calls = missing.calls();
        let steps: [&dyn Step; 3] = [&outdated, &current, &missing];

        let summary = Runner::new(steps).check();

        assert_eq!(summary.steps()[0].status(), CheckStatus::Outdated);
        assert_eq!(summary.steps()[1].status(), CheckStatus::UpToDate);
        assert_eq!(summary.steps()[2].status(), CheckStatus::Skipped);
        assert_eq!(summary.outdated_steps(), 1);
        assert_eq!(summary.up_to_date_count(), 1);
        assert_eq!(summary.skipped_count(), 1);
        assert_eq!(summary.pending(), 1);
        assert_eq!(
            summary.steps()[2].skip_reason(),
            Some(SkipReason::NotInstalled)
        );
        assert_exit_code(summary.exit_code(), ExitCode::from(2));
        assert_eq!(
            summary.to_string(),
            "Homebrew:\n  jq 1.7.1 -> 1.8.0\n1 update(s) pending across 1 step(s).\n"
        );

        assert_eq!(outdated_calls.detect.get(), 1);
        assert_eq!(outdated_calls.check.get(), 1);
        assert_eq!(outdated_calls.run.get(), 0);
        assert_eq!(outdated_calls.cleanup.get(), 0);
        assert_eq!(current_calls.check.get(), 1);
        assert_eq!(current_calls.run.get(), 0);
        assert_eq!(missing_calls.check.get(), 0);
    }

    #[test]
    fn check_exits_zero_when_every_installed_step_is_current() {
        let step = FakeStep::new("uv", "uv");

        let summary = Runner::new([&step as &dyn Step]).check();

        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);
        assert_eq!(summary.to_string(), "Everything is up to date.\n");
    }

    #[test]
    fn check_failures_take_precedence_over_pending_updates() {
        let outdated = FakeStep::new("brew", "Homebrew")
            .report(CheckReport::from_items([OutdatedItem::new("jq")]));
        let failing = FakeStep::new("uv", "uv").fail_check();
        let steps: [&dyn Step; 2] = [&outdated, &failing];

        let summary = Runner::new(steps).check();

        assert!(summary.has_updates());
        assert!(summary.has_failures());
        assert_exit_code(summary.exit_code(), ExitCode::FAILURE);
        assert!(summary.to_string().contains("cannot check"));
    }

    #[test]
    fn a_detection_failure_is_reported_by_check() {
        let step = FakeStep::new("brew", "Homebrew").fail_detect();
        let calls = step.calls();

        let summary = Runner::new([&step as &dyn Step]).check();

        assert_eq!(summary.steps()[0].status(), CheckStatus::Failed);
        assert_exit_code(summary.exit_code(), ExitCode::FAILURE);
        assert_eq!(calls.check.get(), 0);
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
    fn a_later_conflicting_step_is_skipped_with_the_reason() {
        let earlier = FakeStep::new("brew", "Homebrew");
        let later = FakeStep::new("mas", "Mac App Store").conflicting(&["brew"]);
        let earlier_calls = earlier.calls();
        let later_calls = later.calls();

        let summary = run(&[&earlier, &later]);

        assert_eq!(summary.steps().len(), 2);
        assert_eq!(summary.steps()[0].status(), StepStatus::Ok);
        assert_eq!(summary.steps()[1].status(), StepStatus::Skipped);
        assert_eq!(
            summary.steps()[1].skip_reason(),
            Some(SkipReason::Conflict("brew"))
        );
        assert_eq!(summary.ok_count(), 1);
        assert_eq!(summary.skipped_count(), 1);
        assert_eq!(summary.failed_count(), 0);
        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);
        assert!(summary.to_string().contains("conflicts with `brew`"));

        assert_eq!(earlier_calls.run.get(), 1);
        assert_eq!(later_calls.detect.get(), 0);
        assert_eq!(later_calls.run.get(), 0);
        assert_eq!(later_calls.cleanup.get(), 0);
    }

    #[test]
    fn the_earlier_step_wins_even_when_it_declares_the_conflict() {
        let earlier = FakeStep::new("brew", "Homebrew").conflicting(&["mas"]);
        let later = FakeStep::new("mas", "Mac App Store");

        let summary = run(&[&earlier, &later]);

        assert_eq!(summary.steps()[0].status(), StepStatus::Ok);
        assert_eq!(summary.steps()[1].status(), StepStatus::Skipped);
        assert_eq!(
            summary.steps()[1].skip_reason(),
            Some(SkipReason::Conflict("brew"))
        );
    }

    #[test]
    fn an_excluded_step_does_not_conflict_with_the_rest_of_the_run() {
        let brew = FakeStep::new("brew", "Homebrew");
        let mas = FakeStep::new("mas", "Mac App Store").conflicting(&["brew"]);
        let options = RunOptions::new().with_selection(StepSelection::new().with_skip(["brew"]));
        let steps: [&dyn Step; 2] = [&brew, &mas];

        let summary = Runner::with_options(steps, options).run();

        assert_eq!(summary.steps().len(), 1);
        assert_eq!(summary.steps()[0].id(), "mas");
        assert_eq!(summary.steps()[0].status(), StepStatus::Ok);
        assert_eq!(summary.skipped_count(), 0);
    }

    #[test]
    fn a_conflict_skipped_step_does_not_hide_the_steps_that_follow() {
        let first = FakeStep::new("brew", "Homebrew");
        let conflicting = FakeStep::new("mas", "Mac App Store").conflicting(&["brew"]);
        let last = FakeStep::new("uv", "uv");

        let summary = run(&[&first, &conflicting, &last]);

        assert_eq!(summary.steps().len(), 3);
        assert_eq!(summary.steps()[0].status(), StepStatus::Ok);
        assert_eq!(summary.steps()[1].status(), StepStatus::Skipped);
        assert_eq!(summary.steps()[2].status(), StepStatus::Ok);
        assert_eq!(summary.ok_count(), 2);
        assert_eq!(summary.skipped_count(), 1);
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
        assert_exit_code(summary.exit_code(), ExitCode::FAILURE);
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
        assert_exit_code(summary.exit_code(), ExitCode::FAILURE);
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
        assert_exit_code(summary.exit_code(), ExitCode::FAILURE);
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
        assert_exit_code(summary.exit_code(), ExitCode::FAILURE);
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
        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);
    }

    #[test]
    fn an_empty_run_succeeds() {
        let summary = Runner::new(std::iter::empty()).run();

        assert!(summary.steps().is_empty());
        assert!(!summary.has_failures());
        assert_eq!(summary.total_duration(), Duration::ZERO);
        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);
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
