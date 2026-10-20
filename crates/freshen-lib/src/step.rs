//! The step abstraction: one independently runnable unit of work.
//!
//! A step answers four questions and can declare ordering constraints:
//!
//! * [`Step::detect`] — is the tool this step manages installed?
//! * [`Step::check`] — what is outdated (read-only, safe for `--check` mode)?
//! * [`Step::run`] — perform the upgrade.
//! * [`Step::cleanup`] — remove leftovers after a successful run.
//!
//! [`Step::prerequisites`] and [`Step::conflicts`] name other steps by id.
//! The registry uses prerequisites to order a run; the runner uses conflicts
//! to skip the later of two conflicting steps before anything is executed.
//!
//! Steps are object-safe: the registry stores `Box<dyn Step>` and dispatches
//! through the trait without knowing the concrete type.

use std::fmt;

use crate::options::RunOptions;
use crate::platform::Platform;

/// Stable identifier of a step, e.g. `"brew"` or `"uv"`.
pub type StepId = &'static str;

/// Result of [`Step::detect`]: whether the tool managed by the step exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    /// The tool is installed and the step can run.
    Installed,
    /// The tool is missing; the step is skipped.
    NotInstalled,
}

impl Detection {
    /// Returns `true` when the tool is installed.
    #[must_use]
    pub fn is_installed(self) -> bool {
        matches!(self, Self::Installed)
    }
}

/// One outdated package, tool or artifact reported by [`Step::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedItem {
    name: String,
    current: Option<String>,
    latest: Option<String>,
}

impl OutdatedItem {
    /// Creates an item with no known current or latest version.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            current: None,
            latest: None,
        }
    }

    /// Sets the currently installed version.
    #[must_use]
    pub fn with_current(mut self, version: impl Into<String>) -> Self {
        self.current = Some(version.into());
        self
    }

    /// Sets the version the upgrade would install.
    #[must_use]
    pub fn with_latest(mut self, version: impl Into<String>) -> Self {
        self.latest = Some(version.into());
        self
    }

    /// Name of the outdated item.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Currently installed version, when known.
    #[must_use]
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Version available after an upgrade, when known.
    #[must_use]
    pub fn latest(&self) -> Option<&str> {
        self.latest.as_deref()
    }
}

/// Read-only result of [`Step::check`].
///
/// An empty report means the step is up to date. The check must not change
/// the system, so it is safe to run in `--check` mode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    items: Vec<OutdatedItem>,
}

impl CheckReport {
    /// Creates a report saying the step is up to date.
    #[must_use]
    pub fn up_to_date() -> Self {
        Self::default()
    }

    /// Creates a report from the given outdated items.
    #[must_use]
    pub fn from_items(items: impl IntoIterator<Item = OutdatedItem>) -> Self {
        Self {
            items: items.into_iter().collect(),
        }
    }

    /// The outdated items, in the order the step reported them.
    #[must_use]
    pub fn items(&self) -> &[OutdatedItem] {
        &self.items
    }

    /// Returns `true` when nothing needs upgrading.
    #[must_use]
    pub fn is_up_to_date(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of outdated items.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.items.len()
    }
}

impl FromIterator<OutdatedItem> for CheckReport {
    fn from_iter<T: IntoIterator<Item = OutdatedItem>>(iter: T) -> Self {
        Self::from_items(iter)
    }
}

/// The phase in which a step failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// [`Step::detect`]
    Detect,
    /// [`Step::check`]
    Check,
    /// [`Step::run`]
    Run,
    /// [`Step::cleanup`]
    Cleanup,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Detect => "detect",
            Self::Check => "check",
            Self::Run => "run",
            Self::Cleanup => "cleanup",
        };
        f.write_str(name)
    }
}

/// A typed failure from one step, tagged with the step id and phase.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("step `{step}` failed during {operation}: {message}")]
pub struct StepError {
    step: StepId,
    operation: Operation,
    message: String,
}

impl StepError {
    /// Creates an error for `step` that failed during `operation`.
    #[must_use]
    pub fn new(step: StepId, operation: Operation, message: impl Into<String>) -> Self {
        Self {
            step,
            operation,
            message: message.into(),
        }
    }

    /// Creates an error from [`Step::detect`].
    #[must_use]
    pub fn detect(step: StepId, message: impl Into<String>) -> Self {
        Self::new(step, Operation::Detect, message)
    }

    /// Creates an error from [`Step::check`].
    #[must_use]
    pub fn check(step: StepId, message: impl Into<String>) -> Self {
        Self::new(step, Operation::Check, message)
    }

    /// Creates an error from [`Step::run`].
    #[must_use]
    pub fn run(step: StepId, message: impl Into<String>) -> Self {
        Self::new(step, Operation::Run, message)
    }

    /// Creates an error from [`Step::cleanup`].
    #[must_use]
    pub fn cleanup(step: StepId, message: impl Into<String>) -> Self {
        Self::new(step, Operation::Cleanup, message)
    }

    /// Id of the step that failed.
    #[must_use]
    pub fn step(&self) -> StepId {
        self.step
    }

    /// Phase in which the step failed.
    #[must_use]
    pub fn operation(&self) -> Operation {
        self.operation
    }

    /// Human-readable failure detail.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// One unit of work in a freshen run.
///
/// Implementors are stateless where possible; `&self` lets the registry keep
/// steps behind `Box<dyn Step>` and call them in any order.
pub trait Step {
    /// Stable identifier used by prerequisites, conflicts and `--only`/`--skip`.
    fn id(&self) -> StepId;

    /// Name shown to the user, e.g. `"Homebrew"`.
    fn display_name(&self) -> &str;

    /// Checks whether the tool this step manages is installed.
    fn detect(&self) -> Result<Detection, StepError>;

    /// Reports what is outdated without changing anything.
    fn check(&self) -> Result<CheckReport, StepError>;

    /// Performs the upgrade.
    fn run(&self) -> Result<(), StepError>;

    /// Performs the upgrade with the options of the current run.
    ///
    /// Steps that prompt the user or kill processes use this to honour
    /// `--yes` and `--force-kill`. The default ignores the options and calls
    /// [`Step::run`], so a step only overrides it when it needs them.
    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        self.run()
    }

    /// Removes leftovers after the upgrade. Defaults to doing nothing.
    fn cleanup(&self) -> Result<(), StepError> {
        Ok(())
    }

    /// Steps that must run before this one, by id.
    fn prerequisites(&self) -> &'static [StepId] {
        &[]
    }

    /// Steps that must not run in the same run as this one, by id.
    fn conflicts(&self) -> &'static [StepId] {
        &[]
    }

    /// Operating systems on which this step can run.
    ///
    /// The registry filters out steps that do not list the platform of the
    /// current run, so they are never detected, checked or run.
    fn platforms(&self) -> &'static [Platform];
}

#[cfg(test)]
mod tests {
    use super::{CheckReport, Detection, Operation, OutdatedItem, Step, StepError, StepId};
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use std::cell::Cell;

    struct FakeStep {
        id: StepId,
        name: &'static str,
        detection: Detection,
        report: CheckReport,
        prerequisites: &'static [StepId],
        conflicts: &'static [StepId],
        platforms: &'static [Platform],
        run_calls: Cell<usize>,
        cleanup_calls: Cell<usize>,
        fail_run: bool,
    }

    impl FakeStep {
        fn new(id: StepId, name: &'static str) -> Self {
            Self {
                id,
                name,
                detection: Detection::Installed,
                report: CheckReport::up_to_date(),
                prerequisites: &[],
                conflicts: &[],
                platforms: &Platform::ALL,
                run_calls: Cell::new(0),
                cleanup_calls: Cell::new(0),
                fail_run: false,
            }
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
            Ok(self.detection)
        }

        fn check(&self) -> Result<CheckReport, StepError> {
            Ok(self.report.clone())
        }

        fn run(&self) -> Result<(), StepError> {
            self.run_calls.set(self.run_calls.get() + 1);
            if self.fail_run {
                return Err(StepError::run(self.id, "upgrade failed"));
            }
            Ok(())
        }

        fn cleanup(&self) -> Result<(), StepError> {
            self.cleanup_calls.set(self.cleanup_calls.get() + 1);
            Ok(())
        }

        fn prerequisites(&self) -> &'static [StepId] {
            self.prerequisites
        }

        fn conflicts(&self) -> &'static [StepId] {
            self.conflicts
        }

        fn platforms(&self) -> &'static [Platform] {
            self.platforms
        }
    }

    struct MinimalStep;

    impl Step for MinimalStep {
        fn id(&self) -> StepId {
            "minimal"
        }

        fn display_name(&self) -> &str {
            "Minimal"
        }

        fn detect(&self) -> Result<Detection, StepError> {
            Ok(Detection::NotInstalled)
        }

        fn check(&self) -> Result<CheckReport, StepError> {
            Ok(CheckReport::up_to_date())
        }

        fn run(&self) -> Result<(), StepError> {
            Ok(())
        }

        fn platforms(&self) -> &'static [Platform] {
            &Platform::ALL
        }
    }

    fn report(step: &dyn Step) -> CheckReport {
        match step.check() {
            Ok(report) => report,
            Err(error) => panic!("check failed: {error}"),
        }
    }

    #[test]
    fn exposes_identity_and_detection() {
        let step = FakeStep::new("brew", "Homebrew");

        assert_eq!(step.id(), "brew");
        assert_eq!(step.display_name(), "Homebrew");
        assert!(step.detect().is_ok_and(Detection::is_installed));
    }

    #[test]
    fn reports_a_missing_tool() {
        let mut step = FakeStep::new("uv", "uv");
        step.detection = Detection::NotInstalled;

        assert!(step
            .detect()
            .is_ok_and(|detection| !detection.is_installed()));
    }

    #[test]
    fn check_is_read_only_and_reports_outdated_items() {
        let mut step = FakeStep::new("brew", "Homebrew");
        step.report = CheckReport::from_items([
            OutdatedItem::new("jq")
                .with_current("1.7.1")
                .with_latest("1.8.0"),
            OutdatedItem::new("ripgrep"),
        ]);

        let checked = report(&step);

        assert_eq!(step.run_calls.get(), 0);
        assert!(!checked.is_up_to_date());
        assert_eq!(checked.pending(), 2);
        assert_eq!(checked.items()[0].name(), "jq");
        assert_eq!(checked.items()[0].current(), Some("1.7.1"));
        assert_eq!(checked.items()[0].latest(), Some("1.8.0"));
        assert_eq!(checked.items()[1].current(), None);
        assert_eq!(checked.items()[1].latest(), None);
    }

    #[test]
    fn an_empty_report_means_up_to_date() {
        let step = FakeStep::new("uv", "uv");

        let checked = report(&step);

        assert!(checked.is_up_to_date());
        assert_eq!(checked.pending(), 0);
        assert!(checked.items().is_empty());
    }

    #[test]
    fn run_and_cleanup_are_invoked() {
        let step = FakeStep::new("brew", "Homebrew");

        assert!(step.run().is_ok());
        assert!(step.cleanup().is_ok());

        assert_eq!(step.run_calls.get(), 1);
        assert_eq!(step.cleanup_calls.get(), 1);
    }

    #[test]
    fn run_with_options_defaults_to_run() {
        let step = FakeStep::new("brew", "Homebrew");

        assert!(step.run_with_options(&RunOptions::new()).is_ok());

        assert_eq!(step.run_calls.get(), 1);
    }

    #[test]
    fn cleanup_is_optional_and_defaults_to_a_no_op() {
        let step = MinimalStep;

        assert!(step.cleanup().is_ok());
    }

    #[test]
    fn prerequisites_and_conflicts_default_to_empty() {
        let step = MinimalStep;

        assert!(step.prerequisites().is_empty());
        assert!(step.conflicts().is_empty());
    }

    #[test]
    fn prerequisites_and_conflicts_are_exposed_by_id() {
        let mut step = FakeStep::new("casks", "Homebrew casks");
        step.prerequisites = &["brew"];
        step.conflicts = &["mas"];

        assert_eq!(step.prerequisites(), ["brew"]);
        assert_eq!(step.conflicts(), ["mas"]);
    }

    #[test]
    fn platforms_are_declared_by_the_step() {
        static MACOS_AND_LINUX: [Platform; 2] = [Platform::Macos, Platform::Linux];
        let mut step = FakeStep::new("brew", "Homebrew");
        step.platforms = &MACOS_AND_LINUX;

        assert_eq!(step.platforms(), [Platform::Macos, Platform::Linux]);
    }

    #[test]
    fn steps_are_object_safe_and_dispatch_through_the_trait() {
        let step: Box<dyn Step> = Box::new(FakeStep::new("mise", "mise"));

        assert_eq!(step.id(), "mise");
        assert_eq!(step.display_name(), "mise");
        assert!(step.detect().is_ok_and(Detection::is_installed));
        assert!(step.run().is_ok());
        assert!(step.cleanup().is_ok());
    }

    #[test]
    fn errors_carry_step_and_operation() {
        let error = StepError::run("brew", "upgrade failed");

        assert_eq!(error.step(), "brew");
        assert_eq!(error.operation(), Operation::Run);
        assert_eq!(error.message(), "upgrade failed");
        assert_eq!(
            error.to_string(),
            "step `brew` failed during run: upgrade failed"
        );
    }

    #[test]
    fn failing_run_returns_the_typed_error() {
        let mut step = FakeStep::new("brew", "Homebrew");
        step.fail_run = true;

        match step.run() {
            Ok(()) => panic!("run should fail"),
            Err(error) => {
                assert_eq!(error.step(), "brew");
                assert_eq!(error.operation(), Operation::Run);
            }
        }
    }

    #[test]
    fn check_report_collects_from_an_iterator() {
        let report: CheckReport = [
            OutdatedItem::new("jq"),
            OutdatedItem::new("ripgrep").with_latest("14.1.0"),
        ]
        .into_iter()
        .collect();

        assert_eq!(report.pending(), 2);
    }
}
