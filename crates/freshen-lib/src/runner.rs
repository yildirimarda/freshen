//! Executing a run and summarising what happened.
//!
//! The zsh implementation records a failed command, warns about it and keeps
//! going, so one broken package manager never prevents the rest of the machine
//! from being updated. This module keeps that behaviour: every step runs in
//! order, failures are collected instead of aborting the run, and the caller
//! receives one status and duration per step.
//!
//! Exit-code semantics match `update-all.sh`: `0` when no step failed or
//! timed out and `1` when at least one did. A skipped step (its tool is not
//! installed) is not a failure and therefore does not change the exit code.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::check::{CheckResult, CheckSummary};
use crate::cleanup::{self, CleanupReport};
use crate::containers::{ActiveContainerRuntime, COLIMA_STOP, DOCKER};
use crate::deadline::{self, Deadline};
use crate::options::RunOptions;
use crate::snapshot::{RollbackCommand, SnapshotRecorder};
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

pub use crate::step::SkipReason;

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
    /// The step was stopped because its own timeout or the run's global
    /// timeout elapsed before it finished.
    TimedOut,
    /// The run was interrupted while this step was running or before it could
    /// start.
    Interrupted,
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
            Self::TimedOut => "timed out",
            Self::Interrupted => "interrupted",
        }
    }

    /// Returns `true` when the step did not complete because it failed or
    /// timed out.
    #[must_use]
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::TimedOut)
    }
}

impl fmt::Display for StepStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
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

/// The collected result of one step: status, duration, version changes,
/// cleanup reports and failure detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    id: StepId,
    name: String,
    status: StepStatus,
    duration: Duration,
    changes: Vec<VersionChange>,
    cleanups: Vec<CleanupReport>,
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
            cleanups: Vec::new(),
            skip_reason: None,
            error: None,
        }
    }

    fn with_changes(mut self, changes: Vec<VersionChange>) -> Self {
        self.changes = changes;
        self
    }

    fn with_cleanups(mut self, cleanups: Vec<CleanupReport>) -> Self {
        self.cleanups = cleanups;
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

    fn interrupted(step: &dyn Step, duration: Duration) -> Self {
        Self::new(step, StepStatus::Interrupted, duration)
    }

    fn timed_out(step: &dyn Step, duration: Duration) -> Self {
        Self::new(step, StepStatus::TimedOut, duration)
    }

    fn into_timed_out(mut self) -> Self {
        self.status = StepStatus::TimedOut;
        self.error = None;
        self.changes.clear();
        self
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

    /// Destructive cleanup results produced by the step, in run order.
    #[must_use]
    pub fn cleanups(&self) -> &[CleanupReport] {
        &self.cleanups
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
    snapshot: Option<PathBuf>,
    rollback: Vec<RollbackCommand>,
    interrupted: bool,
    restore_failures: Vec<String>,
    container_runtime: Option<ActiveContainerRuntime>,
}

impl RunSummary {
    fn from_results(steps: Vec<StepResult>) -> Self {
        Self {
            steps,
            snapshot: None,
            rollback: Vec::new(),
            interrupted: false,
            restore_failures: Vec::new(),
            container_runtime: None,
        }
    }

    fn with_snapshot(mut self, snapshot: Option<PathBuf>) -> Self {
        self.snapshot = snapshot;
        self
    }

    fn with_rollback(mut self, rollback: Vec<RollbackCommand>) -> Self {
        self.rollback = rollback;
        self
    }

    fn with_interrupted(mut self, interrupted: bool) -> Self {
        self.interrupted = interrupted;
        self
    }

    fn with_restore_failures(mut self, failures: Vec<String>) -> Self {
        self.restore_failures = failures;
        self
    }

    fn with_container_runtime(mut self, runtime: Option<ActiveContainerRuntime>) -> Self {
        self.container_runtime = runtime;
        self
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

    /// Number of steps that exceeded a timeout.
    #[must_use]
    pub fn timed_out_count(&self) -> usize {
        self.count(StepStatus::TimedOut)
    }

    /// Number of steps that were interrupted.
    #[must_use]
    pub fn interrupted_count(&self) -> usize {
        self.count(StepStatus::Interrupted)
    }

    /// Returns `true` when the run was interrupted by a signal.
    #[must_use]
    pub fn was_interrupted(&self) -> bool {
        self.interrupted
    }

    /// Restore commands that failed after an interrupt.
    #[must_use]
    pub fn restore_failures(&self) -> &[String] {
        &self.restore_failures
    }

    /// The active container runtime, when one was detected or overridden.
    #[must_use]
    pub fn container_runtime(&self) -> Option<&ActiveContainerRuntime> {
        self.container_runtime.as_ref()
    }

    /// Returns `true` when at least one step failed or timed out.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.failed_count() > 0 || self.timed_out_count() > 0
    }

    /// Results of the steps that failed or timed out.
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

    /// Path of the pre-upgrade snapshot written by this run, when one was.
    #[must_use]
    pub fn snapshot_path(&self) -> Option<&Path> {
        self.snapshot.as_deref()
    }

    /// Commands that restore the versions recorded before the run.
    #[must_use]
    pub fn rollback_commands(&self) -> &[RollbackCommand] {
        &self.rollback
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

    /// Process exit code: `0` when every step succeeded, `1` when a step
    /// failed or timed out and `130` when the run was interrupted. These
    /// values are stable; see [`crate::exit`].
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        if self.interrupted {
            crate::exit::code(crate::exit::INTERRUPTED)
        } else if self.has_failures() {
            crate::exit::code(crate::exit::STEP_FAILURE)
        } else {
            crate::exit::code(crate::exit::OK)
        }
    }
}

impl fmt::Display for RunSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(runtime) = &self.container_runtime {
            writeln!(f, "Container runtime: {runtime}")?;
        }
        writeln!(
            f,
            "Summary: {} ok, {} planned, {} skipped, {} failed, {} timed out, {} interrupted ({})",
            self.ok_count(),
            self.planned_count(),
            self.skipped_count(),
            self.failed_count(),
            self.timed_out_count(),
            self.interrupted_count(),
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

        if let Some(path) = &self.snapshot {
            writeln!(f)?;
            writeln!(f, "Snapshot: {}", path.display())?;
        }
        if !self.rollback.is_empty() {
            writeln!(f, "Rollback hints (previous versions from the snapshot):")?;
            for command in &self.rollback {
                writeln!(f, "  {}", command.command())?;
            }
        }
        if !self.restore_failures.is_empty() {
            writeln!(f, "Restore warnings:")?;
            for failure in &self.restore_failures {
                writeln!(f, "  {failure}")?;
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
    interrupt: fn() -> bool,
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
    ///
    /// A step the configuration file disables is planned as skipped, with the
    /// reason [`SkipReason::Disabled`], so the summary names it instead of
    /// leaving the user wondering where the step went.
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
            if !options.is_step_enabled(step.id()) {
                planned.push(PlannedStep::Skip {
                    step,
                    reason: SkipReason::Disabled,
                });
                continue;
            }

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
            interrupt: crate::signals::requested,
        }
    }

    #[cfg(test)]
    fn with_interrupt_check(mut self, interrupt: fn() -> bool) -> Self {
        self.interrupt = interrupt;
        self
    }

    /// Options that control this run.
    #[must_use]
    pub fn options(&self) -> &RunOptions {
        &self.options
    }

    fn runs_container_step(&self) -> bool {
        self.steps.iter().copied().any(|planned| match planned {
            PlannedStep::Run(step) => matches!(step.id(), COLIMA_STOP | DOCKER),
            PlannedStep::Skip { .. } => false,
        })
    }

    /// Runs every step and returns the collected results.
    #[must_use]
    pub fn run(&self) -> RunSummary {
        if self.runs_container_step() {
            crate::warnings::warn_about_running_docker_builds();
        }

        let mut snapshots =
            SnapshotRecorder::new().with_retention(self.options.snapshot_retention());
        let mut results = Vec::with_capacity(self.steps.len());
        let mut interrupted = false;
        let mut timed_out = false;
        let run_started = Instant::now();
        let global_deadline = self
            .options
            .global_timeout()
            .map(|limit| Deadline::after(run_started, limit));

        for planned in self.steps.iter().copied() {
            if interrupted || (self.interrupt)() {
                interrupted = true;
                let step = match planned {
                    PlannedStep::Run(step) | PlannedStep::Skip { step, .. } => step,
                };
                results.push(StepResult::skipped_because(
                    step,
                    SkipReason::Interrupted,
                    Duration::ZERO,
                ));
                continue;
            }

            let now = Instant::now();
            if global_deadline.is_some_and(|deadline| now >= deadline.at()) {
                timed_out = true;
                match planned {
                    PlannedStep::Run(step) => {
                        results.push(StepResult::timed_out(step, Duration::ZERO));
                    }
                    PlannedStep::Skip { step, reason } => {
                        results.push(StepResult::skipped_because(step, reason, Duration::ZERO));
                    }
                }
                continue;
            }

            let result = match planned {
                PlannedStep::Run(step) => {
                    let deadline =
                        effective_deadline(now, self.options.step_timeout(), global_deadline);
                    let guard = deadline::enter(deadline);
                    let mut result = run_step(step, &self.options, &mut snapshots, self.interrupt);
                    drop(guard);

                    if deadline.is_some_and(|deadline| deadline.expired())
                        && result.status() != StepStatus::Interrupted
                    {
                        timed_out = true;
                        result = result.into_timed_out();
                    }
                    result
                }
                PlannedStep::Skip { step, reason } => {
                    StepResult::skipped_because(step, reason, Duration::ZERO)
                }
            };
            interrupted |= result.status() == StepStatus::Interrupted;
            results.push(result);
        }

        let mut restore_failures = Vec::new();
        if interrupted || timed_out {
            if interrupted {
                eprintln!("freshen: interrupted; restoring everything that was stopped");
                crate::signals::reset();
            } else {
                eprintln!("freshen: timed out; restoring everything that was stopped");
            }

            for planned in self.steps.iter().rev().copied() {
                let Some(step) = planned.running() else {
                    continue;
                };
                if let Err(error) = step.restore() {
                    eprintln!(
                        "freshen: could not restore {}: {error}",
                        step.display_name()
                    );
                    restore_failures.push(format!("{}: {error}", step.display_name()));
                }
            }
        }

        RunSummary::from_results(results)
            .with_snapshot(snapshots.path().map(Path::to_path_buf))
            .with_rollback(snapshots.rollback_commands())
            .with_interrupted(interrupted)
            .with_restore_failures(restore_failures)
            .with_container_runtime(self.options.container_runtime().cloned())
    }

    /// Detects every step and asks each installed one what is outdated,
    /// without running or cleaning up anything. The configured step and global
    /// timeouts bound detection and the read-only query too, so a hung query
    /// cannot hang the report.
    ///
    /// Up to [`RunOptions::jobs`] queries run at the same time. Results keep
    /// the run order regardless of which query finishes first, so the printed
    /// report and its summary stay deterministic.
    #[must_use]
    pub fn check(&self) -> CheckSummary {
        if self.steps.is_empty() {
            return CheckSummary::from_results(Vec::new());
        }

        let run_started = Instant::now();
        let global_deadline = self
            .options
            .global_timeout()
            .map(|limit| Deadline::after(run_started, limit));
        let step_timeout = self.options.step_timeout();
        let jobs = self.options.jobs().min(self.steps.len());
        let next = AtomicUsize::new(0);
        let results = Mutex::new(vec![None; self.steps.len()]);

        std::thread::scope(|scope| {
            for _ in 0..jobs {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(planned) = self.steps.get(index).copied() else {
                        break;
                    };
                    let result =
                        check_planned(planned, &self.options, step_timeout, global_deadline);
                    let mut slots = match results.lock() {
                        Ok(slots) => slots,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    if let Some(slot) = slots.get_mut(index) {
                        *slot = Some(result);
                    }
                });
            }
        });

        let results = match results.into_inner() {
            Ok(slots) => slots,
            Err(poisoned) => poisoned.into_inner(),
        };
        CheckSummary::from_results(results.into_iter().flatten().collect())
    }
}

fn check_planned(
    planned: PlannedStep<'_>,
    options: &RunOptions,
    step_timeout: Option<Duration>,
    global_deadline: Option<Deadline>,
) -> CheckResult {
    let now = Instant::now();
    if global_deadline.is_some_and(|deadline| now >= deadline.at()) {
        return match planned {
            PlannedStep::Run(step) => CheckResult::timed_out(step, Duration::ZERO),
            PlannedStep::Skip { step, reason } => {
                CheckResult::skipped(step, reason, Duration::ZERO)
            }
        };
    }

    match planned {
        PlannedStep::Run(step) => {
            let deadline = effective_deadline(now, step_timeout, global_deadline);
            let guard = deadline::enter(deadline);
            let result = check_step(step, options);
            drop(guard);

            if deadline.is_some_and(|deadline| deadline.expired()) {
                result.into_timed_out()
            } else {
                result
            }
        }
        PlannedStep::Skip { step, reason } => CheckResult::skipped(step, reason, Duration::ZERO),
    }
}

fn effective_deadline(
    now: Instant,
    step_timeout: Option<Duration>,
    global: Option<Deadline>,
) -> Option<Deadline> {
    let step = step_timeout.map(|limit| Deadline::after(now, limit));
    match (step, global) {
        (Some(step), Some(global)) if step.at() <= global.at() => Some(step),
        (Some(_), Some(global)) | (None, Some(global)) => Some(Deadline::new(
            global.at(),
            global.at().saturating_duration_since(now),
        )),
        (Some(step), None) => Some(step),
        (None, None) => None,
    }
}

fn check_step(step: &dyn Step, options: &RunOptions) -> CheckResult {
    let started = Instant::now();
    match step.detect() {
        Ok(Detection::NotInstalled) => {
            CheckResult::skipped(step, SkipReason::NotInstalled, started.elapsed())
        }
        Ok(Detection::Unavailable(reason)) => CheckResult::skipped(step, reason, started.elapsed()),
        Ok(Detection::Installed) => match step.check_with_options(options) {
            Ok(report) if report.is_up_to_date() => {
                CheckResult::up_to_date(step, started.elapsed())
            }
            Ok(report) => CheckResult::outdated(step, report, started.elapsed()),
            Err(error) if error.is_registry_unreachable() => {
                CheckResult::skipped(step, SkipReason::RegistryUnreachable, started.elapsed())
            }
            Err(error) => CheckResult::failed(step, error, started.elapsed()),
        },
        Err(error) if error.is_registry_unreachable() => {
            CheckResult::skipped(step, SkipReason::RegistryUnreachable, started.elapsed())
        }
        Err(error) => CheckResult::failed(step, error, started.elapsed()),
    }
}

fn run_step(
    step: &dyn Step,
    options: &RunOptions,
    snapshots: &mut SnapshotRecorder,
    interrupt: fn() -> bool,
) -> StepResult {
    let started = Instant::now();
    if interrupt() {
        return StepResult::interrupted(step, started.elapsed());
    }

    match step.detect() {
        Ok(Detection::NotInstalled) => StepResult::skipped(step, started.elapsed()),
        Ok(Detection::Unavailable(reason)) => {
            StepResult::skipped_because(step, reason, started.elapsed())
        }
        Ok(Detection::Installed) if options.is_dry_run() => {
            StepResult::planned(step, started.elapsed())
        }
        Ok(Detection::Installed) => {
            capture_snapshot(step, snapshots);
            if interrupt() {
                return StepResult::interrupted(step, started.elapsed());
            }

            let before = step.check_with_options(options).ok();
            if interrupt() {
                return StepResult::interrupted(step, started.elapsed());
            }

            let recorder = cleanup::Recorder::start();
            let result = match step.run_with_options(options) {
                Ok(()) if interrupt() => StepResult::interrupted(step, started.elapsed()),
                Ok(()) => match step.cleanup() {
                    Ok(()) if interrupt() => StepResult::interrupted(step, started.elapsed()),
                    Ok(()) => StepResult::ok(step, started.elapsed()),
                    Err(_error) if interrupt() => StepResult::interrupted(step, started.elapsed()),
                    Err(error) => StepResult::failed(step, started.elapsed(), error),
                },
                Err(_error) if interrupt() => StepResult::interrupted(step, started.elapsed()),
                Err(error) if error.is_registry_unreachable() => StepResult::skipped_because(
                    step,
                    SkipReason::RegistryUnreachable,
                    started.elapsed(),
                ),
                Err(error) => StepResult::failed(step, started.elapsed(), error),
            };
            let cleanups = recorder.finish();

            let changes =
                if result.status() == StepStatus::Interrupted || result.skip_reason().is_some() {
                    Vec::new()
                } else {
                    match before {
                        Some(before) if !before.is_up_to_date() => {
                            match step.check_with_options(options) {
                                Ok(after) => diff_reports(&before, &after),
                                Err(_) => Vec::new(),
                            }
                        }
                        _ => Vec::new(),
                    }
                };

            result.with_changes(changes).with_cleanups(cleanups)
        }
        Err(_error) if interrupt() => StepResult::interrupted(step, started.elapsed()),
        Err(error) if error.is_registry_unreachable() => {
            StepResult::skipped_because(step, SkipReason::RegistryUnreachable, started.elapsed())
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
    use std::collections::BTreeMap;
    use std::process::ExitCode;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    static INTERRUPT: AtomicBool = AtomicBool::new(false);

    fn interrupt_flag() -> bool {
        INTERRUPT.load(Ordering::SeqCst)
    }

    #[derive(Default)]
    struct Calls {
        detect: AtomicUsize,
        check: AtomicUsize,
        run: AtomicUsize,
        cleanup: AtomicUsize,
        restore: AtomicUsize,
        check_active: AtomicUsize,
        check_max_active: AtomicUsize,
    }

    struct FakeStep {
        id: StepId,
        name: &'static str,
        detection: Detection,
        report: CheckReport,
        fail_detect: bool,
        fail_check: bool,
        check_error: &'static str,
        fail_run: bool,
        run_error: &'static str,
        fail_cleanup: bool,
        conflicts: &'static [StepId],
        sleep: Duration,
        check_sleep: Duration,
        interrupt_on_run: bool,
        calls: Arc<Calls>,
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
                check_error: "cannot check",
                fail_run: false,
                run_error: "upgrade failed",
                fail_cleanup: false,
                conflicts: &[],
                sleep: Duration::ZERO,
                check_sleep: Duration::ZERO,
                interrupt_on_run: false,
                calls: Arc::new(Calls::default()),
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

        fn fail_check_with(mut self, message: &'static str) -> Self {
            self.fail_check = true;
            self.check_error = message;
            self
        }

        fn fail_run(mut self) -> Self {
            self.fail_run = true;
            self
        }

        fn fail_run_with(mut self, message: &'static str) -> Self {
            self.fail_run = true;
            self.run_error = message;
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

        fn check_sleep(mut self, duration: Duration) -> Self {
            self.check_sleep = duration;
            self
        }

        fn sharing_calls(mut self, calls: Arc<Calls>) -> Self {
            self.calls = calls;
            self
        }

        fn interrupt_on_run(mut self) -> Self {
            self.interrupt_on_run = true;
            self
        }

        fn calls(&self) -> Arc<Calls> {
            Arc::clone(&self.calls)
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
            self.calls.detect.fetch_add(1, Ordering::SeqCst);
            if self.fail_detect {
                return Err(StepError::detect(self.id, "cannot detect"));
            }
            Ok(self.detection)
        }

        fn check(&self) -> Result<CheckReport, StepError> {
            self.calls.check.fetch_add(1, Ordering::SeqCst);
            let active = self.calls.check_active.fetch_add(1, Ordering::SeqCst) + 1;
            self.calls
                .check_max_active
                .fetch_max(active, Ordering::SeqCst);
            if !self.check_sleep.is_zero() {
                std::thread::sleep(self.check_sleep);
            }
            self.calls.check_active.fetch_sub(1, Ordering::SeqCst);
            if self.fail_check {
                return Err(StepError::check(self.id, self.check_error));
            }
            Ok(self.report.clone())
        }

        fn run(&self) -> Result<(), StepError> {
            self.calls.run.fetch_add(1, Ordering::SeqCst);
            if !self.sleep.is_zero() {
                std::thread::sleep(self.sleep);
            }
            if self.interrupt_on_run {
                INTERRUPT.store(true, Ordering::SeqCst);
            }
            if self.fail_run {
                return Err(StepError::run(self.id, self.run_error));
            }
            Ok(())
        }

        fn cleanup(&self) -> Result<(), StepError> {
            self.calls.cleanup.fetch_add(1, Ordering::SeqCst);
            if self.fail_cleanup {
                return Err(StepError::cleanup(self.id, "cleanup failed"));
            }
            Ok(())
        }

        fn restore(&self) -> Result<(), StepError> {
            self.calls.restore.fetch_add(1, Ordering::SeqCst);
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

        assert_eq!(first_calls.detect.load(Ordering::SeqCst), 1);
        assert_eq!(first_calls.run.load(Ordering::SeqCst), 1);
        assert_eq!(first_calls.cleanup.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.run.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.cleanup.load(Ordering::SeqCst), 1);
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

        assert_eq!(calls.detect.load(Ordering::SeqCst), 1);
        assert_eq!(calls.run.load(Ordering::SeqCst), 0);
        assert_eq!(calls.cleanup.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_step_that_is_installed_but_blocked_is_skipped_with_its_reason() {
        let step = FakeStep::new("mas", "Mac App Store")
            .detection(Detection::Unavailable(SkipReason::NotSignedIn));
        let calls = step.calls();

        let summary = run(&[&step]);

        let result = &summary.steps()[0];
        assert_eq!(result.status(), StepStatus::Skipped);
        assert_eq!(result.skip_reason(), Some(SkipReason::NotSignedIn));
        assert_eq!(result.error(), None);
        assert_eq!(summary.skipped_count(), 1);
        assert_eq!(summary.failed_count(), 0);
        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);

        assert_eq!(calls.detect.load(Ordering::SeqCst), 1);
        assert_eq!(calls.run.load(Ordering::SeqCst), 0);
        assert_eq!(calls.check.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_blocked_step_is_skipped_in_check_mode_too() {
        let step = FakeStep::new("mas", "Mac App Store")
            .detection(Detection::Unavailable(SkipReason::NotSignedIn));
        let calls = step.calls();
        let steps: [&dyn Step; 1] = [&step];

        let summary = Runner::new(steps).check();

        assert_eq!(summary.steps()[0].status(), CheckStatus::Skipped);
        assert_eq!(
            summary.steps()[0].skip_reason(),
            Some(SkipReason::NotSignedIn)
        );
        assert_eq!(summary.skipped_count(), 1);
        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);

        assert_eq!(calls.check.load(Ordering::SeqCst), 0);
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

        assert_eq!(calls.detect.load(Ordering::SeqCst), 1);
        assert_eq!(calls.run.load(Ordering::SeqCst), 0);
        assert_eq!(calls.cleanup.load(Ordering::SeqCst), 0);
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

        assert_eq!(outdated_calls.detect.load(Ordering::SeqCst), 1);
        assert_eq!(outdated_calls.check.load(Ordering::SeqCst), 1);
        assert_eq!(outdated_calls.run.load(Ordering::SeqCst), 0);
        assert_eq!(outdated_calls.cleanup.load(Ordering::SeqCst), 0);
        assert_eq!(current_calls.check.load(Ordering::SeqCst), 1);
        assert_eq!(current_calls.run.load(Ordering::SeqCst), 0);
        assert_eq!(missing_calls.check.load(Ordering::SeqCst), 0);
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
        assert_eq!(calls.check.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_check_that_exceeds_the_step_timeout_is_timed_out_not_failed() {
        let step = FakeStep::new("brew", "Homebrew").check_sleep(Duration::from_millis(200));
        let options = RunOptions::new().with_step_timeout(Some(Duration::from_millis(20)));

        let summary = Runner::with_options([&step as &dyn Step], options).check();

        let result = &summary.steps()[0];
        assert_eq!(result.status(), CheckStatus::TimedOut);
        assert_eq!(result.error(), None);
        assert_eq!(summary.timed_out_count(), 1);
        assert_eq!(summary.failed_count(), 0);
        assert!(summary.has_failures());
        assert_exit_code(summary.exit_code(), ExitCode::FAILURE);
        assert!(summary.to_string().contains("Homebrew: timed out"));
    }

    #[test]
    fn a_global_timeout_marks_remaining_check_steps_as_timed_out() {
        let slow = FakeStep::new("brew", "Homebrew").check_sleep(Duration::from_millis(200));
        let remaining = FakeStep::new("uv", "uv");
        let remaining_calls = remaining.calls();
        let options = RunOptions::new()
            .with_jobs(1)
            .with_global_timeout(Some(Duration::from_millis(20)));
        let steps: [&dyn Step; 2] = [&slow, &remaining];

        let summary = Runner::with_options(steps, options).check();

        assert_eq!(summary.steps()[0].status(), CheckStatus::TimedOut);
        assert_eq!(summary.steps()[1].status(), CheckStatus::TimedOut);
        assert_eq!(summary.timed_out_count(), 2);
        assert_eq!(summary.failed_count(), 0);
        assert_eq!(remaining_calls.check.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn check_runs_independent_queries_concurrently_within_the_job_limit() {
        let slow = Duration::from_millis(100);
        let calls = Arc::new(Calls::default());
        let first = FakeStep::new("brew", "Homebrew")
            .check_sleep(slow)
            .sharing_calls(Arc::clone(&calls));
        let second = FakeStep::new("uv", "uv")
            .check_sleep(slow)
            .sharing_calls(Arc::clone(&calls));
        let third = FakeStep::new("mise", "mise")
            .check_sleep(slow)
            .sharing_calls(Arc::clone(&calls));
        let fourth = FakeStep::new("docker", "Docker")
            .check_sleep(slow)
            .sharing_calls(Arc::clone(&calls));
        let steps: [&dyn Step; 4] = [&first, &second, &third, &fourth];
        let options = RunOptions::new().with_jobs(2);

        let summary = Runner::with_options(steps, options).check();

        assert_eq!(summary.up_to_date_count(), 4);
        assert_eq!(calls.check.load(Ordering::SeqCst), 4);
        assert_eq!(calls.check_max_active.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn check_keeps_the_run_order_when_a_later_query_finishes_first() {
        let slow = FakeStep::new("brew", "Homebrew")
            .check_sleep(Duration::from_millis(100))
            .report(CheckReport::from_items([OutdatedItem::new("jq")]));
        let fast =
            FakeStep::new("uv", "uv").report(CheckReport::from_items([OutdatedItem::new("ruff")]));
        let medium = FakeStep::new("mise", "mise")
            .check_sleep(Duration::from_millis(50))
            .report(CheckReport::from_items([OutdatedItem::new("node")]));
        let steps: [&dyn Step; 3] = [&slow, &fast, &medium];
        let options = RunOptions::new().with_jobs(3);

        let summary = Runner::with_options(steps, options).check();

        let ids: Vec<StepId> = summary.steps().iter().map(|step| step.id()).collect();
        assert_eq!(ids, ["brew", "uv", "mise"]);
        assert_eq!(summary.outdated_steps(), 3);
        assert_eq!(summary.pending(), 3);
        assert_eq!(
            summary.to_string(),
            "Homebrew:\n  jq\nuv:\n  ruff\nmise:\n  node\n3 update(s) pending across 3 step(s).\n"
        );
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

        assert_eq!(earlier_calls.run.load(Ordering::SeqCst), 1);
        assert_eq!(later_calls.detect.load(Ordering::SeqCst), 0);
        assert_eq!(later_calls.run.load(Ordering::SeqCst), 0);
        assert_eq!(later_calls.cleanup.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_step_disabled_in_the_configuration_file_is_skipped_with_the_reason() {
        let brew = FakeStep::new("brew", "Homebrew");
        let uv = FakeStep::new("uv", "uv");
        let steps: [&dyn Step; 2] = [&brew, &uv];
        let mut overrides = BTreeMap::new();
        overrides.insert("uv".to_owned(), false);
        let options = RunOptions::new().with_step_overrides(overrides);

        let summary = Runner::with_options(steps, options).run();

        assert_eq!(summary.steps().len(), 2);
        assert_eq!(summary.steps()[0].status(), StepStatus::Ok);
        assert_eq!(summary.steps()[1].status(), StepStatus::Skipped);
        assert_eq!(summary.steps()[1].skip_reason(), Some(SkipReason::Disabled));
        assert!(summary
            .to_string()
            .contains("disabled in the configuration file"));
    }

    #[test]
    fn a_disabled_step_is_never_detected_or_run() {
        let brew = FakeStep::new("brew", "Homebrew");
        let calls = brew.calls();
        let mut overrides = BTreeMap::new();
        overrides.insert("brew".to_owned(), false);
        let options = RunOptions::new().with_step_overrides(overrides);

        let _ = Runner::with_options([&brew as &dyn Step], options).run();

        assert_eq!(calls.detect.load(Ordering::SeqCst), 0);
        assert_eq!(calls.run.load(Ordering::SeqCst), 0);
        assert_eq!(calls.cleanup.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_disabled_step_does_not_conflict_with_the_rest_of_the_run() {
        let brew = FakeStep::new("brew", "Homebrew").conflicting(&["mas"]);
        let mas = FakeStep::new("mas", "Mac App Store");
        let mut overrides = BTreeMap::new();
        overrides.insert("brew".to_owned(), false);
        let options = RunOptions::new().with_step_overrides(overrides);

        let summary = Runner::with_options([&brew as &dyn Step, &mas], options).run();

        assert_eq!(summary.steps()[0].status(), StepStatus::Skipped);
        assert_eq!(summary.steps()[1].status(), StepStatus::Ok);
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
        assert_eq!(failing_calls.cleanup.load(Ordering::SeqCst), 0);

        assert_eq!(healthy_calls.run.load(Ordering::SeqCst), 1);
        assert_eq!(healthy_calls.cleanup.load(Ordering::SeqCst), 1);
        assert_eq!(summary.steps()[1].status(), StepStatus::Ok);
    }

    #[test]
    fn an_unreachable_registry_skips_the_step_instead_of_failing_it() {
        let step = FakeStep::new("brew", "Homebrew")
            .fail_run_with("curl: (6) Could not resolve host: formulae.brew.sh");

        let summary = run(&[&step]);

        let result = &summary.steps()[0];
        assert_eq!(result.status(), StepStatus::Skipped);
        assert_eq!(result.skip_reason(), Some(SkipReason::RegistryUnreachable));
        assert_eq!(result.error(), None);
        assert!(!summary.has_failures());
        assert_exit_code(summary.exit_code(), ExitCode::SUCCESS);
    }

    #[test]
    fn an_unreachable_registry_skips_the_step_during_check() {
        let step = FakeStep::new("brew", "Homebrew")
            .fail_check_with("curl: (6) Could not resolve host: formulae.brew.sh");

        let summary = Runner::new([&step as &dyn Step]).check();

        let result = &summary.steps()[0];
        assert_eq!(result.status(), CheckStatus::Skipped);
        assert_eq!(result.skip_reason(), Some(SkipReason::RegistryUnreachable));
        assert!(!summary.has_failures());
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
        assert_eq!(calls.detect.load(Ordering::SeqCst), 1);
        assert_eq!(calls.run.load(Ordering::SeqCst), 0);
        assert_eq!(calls.cleanup.load(Ordering::SeqCst), 0);
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
        assert_eq!(calls.run.load(Ordering::SeqCst), 1);
        assert_eq!(calls.cleanup.load(Ordering::SeqCst), 1);
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
    fn a_step_that_exceeds_its_timeout_is_timed_out_not_failed() {
        let slow = FakeStep::new("brew", "Homebrew").sleep(Duration::from_millis(30));
        let calls = slow.calls();
        let options = RunOptions::new().with_step_timeout(Some(Duration::from_millis(5)));

        let summary = Runner::with_options([&slow as &dyn Step], options).run();
        let result = &summary.steps()[0];

        assert_eq!(result.status(), StepStatus::TimedOut);
        assert_eq!(result.error(), None);
        assert_eq!(summary.failed_count(), 0);
        assert_eq!(summary.timed_out_count(), 1);
        assert!(summary.has_failures());
        assert_eq!(summary.failures().count(), 1);
        assert_exit_code(summary.exit_code(), ExitCode::FAILURE);
        let text = summary.to_string();
        assert!(text.contains("timed out"), "summary: {text}");
        assert!(text.contains("0 failed, 1 timed out"), "summary: {text}");
        assert_eq!(calls.restore.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_step_timeout_does_not_stop_the_next_step() {
        let slow = FakeStep::new("brew", "Homebrew").sleep(Duration::from_millis(30));
        let healthy = FakeStep::new("uv", "uv");
        let healthy_calls = healthy.calls();
        let options = RunOptions::new().with_step_timeout(Some(Duration::from_millis(5)));
        let steps: [&dyn Step; 2] = [&slow, &healthy];

        let summary = Runner::with_options(steps, options).run();

        assert_eq!(summary.steps()[0].status(), StepStatus::TimedOut);
        assert_eq!(summary.steps()[1].status(), StepStatus::Ok);
        assert_eq!(summary.timed_out_count(), 1);
        assert_eq!(summary.ok_count(), 1);
        assert_eq!(healthy_calls.run.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_global_timeout_marks_unstarted_steps_as_timed_out() {
        let slow = FakeStep::new("brew", "Homebrew").sleep(Duration::from_millis(30));
        let later = FakeStep::new("uv", "uv");
        let slow_calls = slow.calls();
        let later_calls = later.calls();
        let options = RunOptions::new().with_global_timeout(Some(Duration::from_millis(5)));
        let steps: [&dyn Step; 2] = [&slow, &later];

        let summary = Runner::with_options(steps, options).run();

        assert_eq!(summary.steps()[0].status(), StepStatus::TimedOut);
        assert_eq!(summary.steps()[1].status(), StepStatus::TimedOut);
        assert_eq!(summary.timed_out_count(), 2);
        assert_eq!(summary.failed_count(), 0);
        assert_exit_code(summary.exit_code(), ExitCode::FAILURE);
        assert_eq!(slow_calls.restore.load(Ordering::SeqCst), 1);
        assert_eq!(later_calls.restore.load(Ordering::SeqCst), 1);
        assert_eq!(later_calls.detect.load(Ordering::SeqCst), 0);
        assert_eq!(later_calls.run.load(Ordering::SeqCst), 0);
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

    #[test]
    fn an_interrupt_stops_the_run_skips_cleanup_and_restores_in_reverse_order() {
        INTERRUPT.store(false, Ordering::SeqCst);
        let first = FakeStep::new("brew", "Homebrew");
        let second = FakeStep::new("uv", "uv").interrupt_on_run();
        let third = FakeStep::new("mise", "mise");
        let first_calls = first.calls();
        let second_calls = second.calls();
        let third_calls = third.calls();
        let steps: [&dyn Step; 3] = [&first, &second, &third];

        let summary = Runner::new(steps)
            .with_interrupt_check(interrupt_flag)
            .run();

        assert!(summary.was_interrupted());
        assert_eq!(summary.interrupted_count(), 1);
        assert_eq!(summary.steps()[0].status(), StepStatus::Ok);
        assert_eq!(summary.steps()[1].status(), StepStatus::Interrupted);
        assert_eq!(summary.steps()[2].status(), StepStatus::Skipped);
        assert_eq!(
            summary.steps()[2].skip_reason(),
            Some(SkipReason::Interrupted)
        );
        assert_exit_code(summary.exit_code(), ExitCode::from(130));
        assert!(summary.to_string().contains("interrupted"));

        assert_eq!(first_calls.cleanup.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.cleanup.load(Ordering::SeqCst), 0);
        assert_eq!(third_calls.detect.load(Ordering::SeqCst), 0);
        assert_eq!(third_calls.run.load(Ordering::SeqCst), 0);

        assert_eq!(first_calls.restore.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.restore.load(Ordering::SeqCst), 1);
        assert_eq!(third_calls.restore.load(Ordering::SeqCst), 1);

        INTERRUPT.store(false, Ordering::SeqCst);
    }
}
