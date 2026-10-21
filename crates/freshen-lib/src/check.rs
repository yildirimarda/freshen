//! Read-only `--check` mode: report what an upgrade would change.
//!
//! Every step is detected first. Missing tools are skipped, installed tools
//! are asked for their outdated items through [`Step::check`], and the result
//! is collected without running or cleaning up anything. The exit code is `0`
//! when everything is current, `2` when at least one update is pending and `1`
//! when a step failed to report.

use std::fmt;
use std::process::ExitCode;
use std::time::Duration;

use crate::runner::SkipReason;
use crate::step::{CheckReport, Step, StepError, StepId};

/// Outcome of checking one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckStatus {
    /// The step reported no outdated items.
    UpToDate,
    /// The step reported at least one outdated item.
    Outdated,
    /// The tool managed by the step is not part of this check.
    Skipped,
    /// Detection or the read-only check failed.
    Failed,
    /// The step was stopped because its own timeout or the run's global
    /// timeout elapsed before it finished.
    TimedOut,
}

impl CheckStatus {
    /// Lowercase name used in output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UpToDate => "up to date",
            Self::Outdated => "outdated",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::TimedOut => "timed out",
        }
    }
}

impl fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The collected result of checking one step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    id: StepId,
    name: String,
    status: CheckStatus,
    report: CheckReport,
    skip_reason: Option<SkipReason>,
    error: Option<StepError>,
    duration: Duration,
}

impl CheckResult {
    pub(crate) fn up_to_date(step: &dyn Step, duration: Duration) -> Self {
        Self::new(
            step,
            CheckStatus::UpToDate,
            CheckReport::up_to_date(),
            duration,
        )
    }

    pub(crate) fn outdated(step: &dyn Step, report: CheckReport, duration: Duration) -> Self {
        Self::new(step, CheckStatus::Outdated, report, duration)
    }

    pub(crate) fn skipped(step: &dyn Step, reason: SkipReason, duration: Duration) -> Self {
        Self {
            skip_reason: Some(reason),
            ..Self::new(
                step,
                CheckStatus::Skipped,
                CheckReport::up_to_date(),
                duration,
            )
        }
    }

    pub(crate) fn failed(step: &dyn Step, error: StepError, duration: Duration) -> Self {
        Self {
            error: Some(error),
            ..Self::new(
                step,
                CheckStatus::Failed,
                CheckReport::up_to_date(),
                duration,
            )
        }
    }

    pub(crate) fn timed_out(step: &dyn Step, duration: Duration) -> Self {
        Self::new(
            step,
            CheckStatus::TimedOut,
            CheckReport::up_to_date(),
            duration,
        )
    }

    pub(crate) fn into_timed_out(mut self) -> Self {
        self.status = CheckStatus::TimedOut;
        self.report = CheckReport::up_to_date();
        self.skip_reason = None;
        self.error = None;
        self
    }

    fn new(step: &dyn Step, status: CheckStatus, report: CheckReport, duration: Duration) -> Self {
        Self {
            id: step.id(),
            name: step.display_name().to_owned(),
            status,
            report,
            skip_reason: None,
            error: None,
            duration,
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

    /// Outcome of the check.
    #[must_use]
    pub fn status(&self) -> CheckStatus {
        self.status
    }

    /// What the step reported as outdated.
    #[must_use]
    pub fn report(&self) -> &CheckReport {
        &self.report
    }

    /// Why the step was skipped, when it was.
    #[must_use]
    pub fn skip_reason(&self) -> Option<SkipReason> {
        self.skip_reason
    }

    /// The failure that stopped the check, when it failed.
    #[must_use]
    pub fn error(&self) -> Option<&StepError> {
        self.error.as_ref()
    }

    /// Wall-clock time spent detecting and checking the step.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.duration
    }
}

/// The final summary of a `--check` run: one [`CheckResult`] per step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckSummary {
    steps: Vec<CheckResult>,
}

impl CheckSummary {
    pub(crate) fn from_results(steps: Vec<CheckResult>) -> Self {
        Self { steps }
    }

    /// Results of every step that was part of the check, in run order.
    #[must_use]
    pub fn steps(&self) -> &[CheckResult] {
        &self.steps
    }

    /// Number of steps with the given status.
    #[must_use]
    pub fn count(&self, status: CheckStatus) -> usize {
        self.steps
            .iter()
            .filter(|result| result.status() == status)
            .count()
    }

    /// Number of steps that are up to date.
    #[must_use]
    pub fn up_to_date_count(&self) -> usize {
        self.count(CheckStatus::UpToDate)
    }

    /// Number of steps that reported pending updates.
    #[must_use]
    pub fn outdated_steps(&self) -> usize {
        self.count(CheckStatus::Outdated)
    }

    /// Number of steps whose tool was not installed or that were excluded.
    #[must_use]
    pub fn skipped_count(&self) -> usize {
        self.count(CheckStatus::Skipped)
    }

    /// Number of steps that failed to report.
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.count(CheckStatus::Failed)
    }

    /// Number of steps that ran out of time before reporting.
    #[must_use]
    pub fn timed_out_count(&self) -> usize {
        self.count(CheckStatus::TimedOut)
    }

    /// Total number of outdated items across every step.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.steps
            .iter()
            .map(|result| result.report().pending())
            .sum()
    }

    /// Returns `true` when at least one step reported an update.
    #[must_use]
    pub fn has_updates(&self) -> bool {
        self.pending() > 0
    }

    /// Returns `true` when at least one step failed or timed out.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.failed_count() > 0 || self.timed_out_count() > 0
    }

    /// Renders the check summary as a JSON report.
    #[must_use]
    pub fn to_json(&self) -> String {
        crate::report::check_json(self)
    }

    /// Renders the check summary as a Markdown report.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        crate::report::check_markdown(self)
    }

    /// Exit code for check mode: `0` up to date, `1` failure, `2` updates
    /// pending. These values are stable; see [`crate::exit`].
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        if self.has_failures() {
            crate::exit::code(crate::exit::STEP_FAILURE)
        } else if self.has_updates() {
            crate::exit::code(crate::exit::UPDATES_PENDING)
        } else {
            crate::exit::code(crate::exit::OK)
        }
    }
}

impl fmt::Display for CheckSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut reported = false;

        for result in &self.steps {
            match result.status() {
                CheckStatus::Outdated => {
                    reported = true;
                    writeln!(f, "{}:", result.name())?;
                    for item in result.report().items() {
                        writeln!(f, "  {item}")?;
                    }
                }
                CheckStatus::Failed => {
                    reported = true;
                    if let Some(error) = result.error() {
                        writeln!(f, "{}: {error}", result.name())?;
                    } else {
                        writeln!(f, "{}: check failed", result.name())?;
                    }
                }
                CheckStatus::TimedOut => {
                    reported = true;
                    writeln!(f, "{}: timed out", result.name())?;
                }
                CheckStatus::UpToDate | CheckStatus::Skipped => {}
            }
        }

        if !reported {
            return writeln!(f, "Everything is up to date.");
        }

        if self.has_updates() {
            writeln!(
                f,
                "{} update(s) pending across {} step(s).",
                self.pending(),
                self.outdated_steps()
            )?;
        }

        Ok(())
    }
}
