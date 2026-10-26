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

use crate::options::{RunOptions, SelectionFlag};
use crate::platform::Platform;
use crate::process::{network_failure, ProcessError};
use crate::snapshot::SnapshotSection;

/// Stable identifier of a step, e.g. `"brew"` or `"uv"`.
pub type StepId = &'static str;

/// Result of [`Step::detect`]: whether the tool managed by the step exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    /// The tool is installed and the step can run.
    Installed,
    /// The tool is missing; the step is skipped.
    NotInstalled,
    /// The tool is installed, but the step cannot run in the current state.
    ///
    /// The step is skipped with the given reason instead of running, for
    /// example because the account of the service it manages is not signed in.
    Unavailable(SkipReason),
}

impl Detection {
    /// Returns `true` when the tool is installed.
    #[must_use]
    pub fn is_installed(self) -> bool {
        matches!(self, Self::Installed)
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
    /// The configuration file disabled the step.
    Disabled,
    /// The step conflicts with an earlier step that is part of the run.
    Conflict(StepId),
    /// A prerequisite step is disabled in the configuration file.
    PrerequisiteDisabled(StepId),
    /// The package registry the step needs could not be reached.
    RegistryUnreachable,
    /// The account the step manages is not signed in to its service.
    NotSignedIn,
    /// The configuration file the step works with does not exist.
    ///
    /// The step is skipped with this reason instead of failing, for example
    /// because there is no Brewfile for the Homebrew bundle step.
    MissingConfiguration,
    /// The run was interrupted before the step started.
    Interrupted,
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
            Self::Disabled => "disabled in the configuration file",
            Self::Conflict(_) => "conflicts with another step",
            Self::PrerequisiteDisabled(_) => "prerequisite disabled",
            Self::RegistryUnreachable => "registry unreachable",
            Self::NotSignedIn => "not signed in",
            Self::MissingConfiguration => "no configuration file found",
            Self::Interrupted => "interrupted before it started",
        }
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(conflict) => write!(f, "conflicts with `{conflict}`"),
            Self::PrerequisiteDisabled(prerequisite) => {
                write!(f, "prerequisite `{prerequisite}` disabled")
            }
            other => f.write_str(other.as_str()),
        }
    }
}

/// One outdated package, tool or artifact reported by [`Step::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedItem {
    name: String,
    /// The identifier a step uses to target the item, for example a Homebrew
    /// formula name, a uv tool name or a Mac App Store id. `None` when the
    /// step cannot identify the item precisely.
    id: Option<String>,
    current: Option<String>,
    latest: Option<String>,
    reason: Option<String>,
}

impl OutdatedItem {
    /// Creates an item with no known current or latest version.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            id: None,
            current: None,
            latest: None,
            reason: None,
        }
    }

    /// Sets the identifier used to target the item, for example a Mac App
    /// Store id.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// The identifier used to target the item, when known.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
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

    /// Explains why the item is outdated or needs special handling.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
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

    /// Why the item needs special handling, when known.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

impl fmt::Display for OutdatedItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if let Some(current) = &self.current {
            write!(f, " {current}")?;
        }
        if let Some(latest) = &self.latest {
            write!(f, " -> {latest}")?;
        }
        if let Some(reason) = &self.reason {
            write!(f, " ({reason})")?;
        }
        Ok(())
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

    /// Keeps only the items whose name passes `predicate`.
    ///
    /// Steps that apply an include or exclude list use this to report only
    /// the items they would really upgrade.
    #[must_use]
    pub fn filtered<F>(self, predicate: F) -> Self
    where
        F: Fn(&str) -> bool,
    {
        Self::from_items(
            self.items
                .into_iter()
                .filter(|item| predicate(item.name()))
                .collect::<Vec<_>>(),
        )
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
    registry_unreachable: bool,
}

impl StepError {
    /// Creates an error for `step` that failed during `operation`.
    #[must_use]
    pub fn new(step: StepId, operation: Operation, message: impl Into<String>) -> Self {
        let message = message.into();
        let registry_unreachable = network_failure(&message).is_some();
        Self {
            step,
            operation,
            message,
            registry_unreachable,
        }
    }

    /// Creates an error from a failed process, preserving a network failure
    /// found in the command's captured output.
    #[must_use]
    pub fn from_process(step: StepId, operation: Operation, error: &ProcessError) -> Self {
        Self {
            step,
            operation,
            message: error.to_string(),
            registry_unreachable: error.is_network_failure(),
        }
    }

    /// Creates a run error from a failed process.
    #[must_use]
    pub fn run_from_process(step: StepId, error: &ProcessError) -> Self {
        Self::from_process(step, Operation::Run, error)
    }

    /// Creates a check error from a failed process.
    #[must_use]
    pub fn check_from_process(step: StepId, error: &ProcessError) -> Self {
        Self::from_process(step, Operation::Check, error)
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

    /// Returns `true` when the failure means the package registry could not
    /// be reached, so the runner should skip the step instead of failing it.
    #[must_use]
    pub fn is_registry_unreachable(&self) -> bool {
        self.registry_unreachable
    }
}

/// One unit of work in a freshen run.
///
/// Implementors are stateless where possible; `&self` lets the registry keep
/// steps behind `Box<dyn Step>` and call them in any order. `--check` queries
/// independent steps on several threads, so implementations must be `Sync`:
/// shared mutable state has to use `Arc<Mutex<_>>` or atomics rather than
/// `Rc`/`Cell`.
pub trait Step: Sync {
    /// Stable identifier used by prerequisites, conflicts and `--only`/`--skip`.
    fn id(&self) -> StepId;

    /// Name shown to the user, e.g. `"Homebrew"`.
    fn display_name(&self) -> &str;

    /// Checks whether the tool this step manages is installed.
    fn detect(&self) -> Result<Detection, StepError>;

    /// Reports what is outdated without changing anything.
    fn check(&self) -> Result<CheckReport, StepError>;

    /// Reports what is outdated without changing anything, honouring the
    /// options of the current run.
    ///
    /// Steps that apply an include or exclude list use this to report only
    /// the items they would really upgrade. The default ignores the options
    /// and calls [`Step::check`], so a step only overrides it when it needs
    /// them.
    fn check_with_options(&self, _options: &RunOptions) -> Result<CheckReport, StepError> {
        self.check()
    }

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

    /// Restores anything this step stopped when the run is interrupted.
    ///
    /// The runner calls this for every planned step, in reverse execution
    /// order, after an interrupt. Steps that stop apps, containers or VMs
    /// must implement it and must make it safe to call when they stopped
    /// nothing. The default does nothing.
    fn restore(&self) -> Result<(), StepError> {
        Ok(())
    }

    /// Read-only commands that record the installed versions this step can
    /// change, captured before the step runs.
    ///
    /// Steps that manage the same package manager return the same
    /// [`SnapshotSection::group`], so the runner records those versions only
    /// once per run. The default is no snapshot.
    fn snapshot(&self) -> Option<SnapshotSection> {
        None
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
    use super::{
        CheckReport, Detection, Operation, OutdatedItem, SkipReason, Step, StepError, StepId,
    };
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::snapshot::{SnapshotCommand, SnapshotSection};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeStep {
        id: StepId,
        name: &'static str,
        detection: Detection,
        report: CheckReport,
        prerequisites: &'static [StepId],
        conflicts: &'static [StepId],
        platforms: &'static [Platform],
        snapshot: Option<SnapshotSection>,
        run_calls: AtomicUsize,
        cleanup_calls: AtomicUsize,
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
                snapshot: None,
                run_calls: AtomicUsize::new(0),
                cleanup_calls: AtomicUsize::new(0),
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
            self.run_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_run {
                return Err(StepError::run(self.id, "upgrade failed"));
            }
            Ok(())
        }

        fn cleanup(&self) -> Result<(), StepError> {
            self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn snapshot(&self) -> Option<SnapshotSection> {
            self.snapshot.clone()
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
    fn a_step_that_is_installed_but_blocked_is_not_runnable() {
        let detection = Detection::Unavailable(SkipReason::NotSignedIn);

        assert!(!detection.is_installed());
    }

    #[test]
    fn a_skip_reason_names_the_reason_in_one_line() {
        assert_eq!(SkipReason::NotSignedIn.to_string(), "not signed in");
        assert_eq!(
            SkipReason::Conflict("brew").to_string(),
            "conflicts with `brew`"
        );
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

        assert_eq!(step.run_calls.load(Ordering::SeqCst), 0);
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

        assert_eq!(step.run_calls.load(Ordering::SeqCst), 1);
        assert_eq!(step.cleanup_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn run_with_options_defaults_to_run() {
        let step = FakeStep::new("brew", "Homebrew");

        assert!(step.run_with_options(&RunOptions::new()).is_ok());

        assert_eq!(step.run_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cleanup_is_optional_and_defaults_to_a_no_op() {
        let step = MinimalStep;

        assert!(step.cleanup().is_ok());
    }

    #[test]
    fn snapshot_is_optional_and_defaults_to_none() {
        let step = MinimalStep;

        assert!(step.snapshot().is_none());
    }

    #[test]
    fn snapshot_is_exposed_through_the_trait() {
        let mut step = FakeStep::new("brew", "Homebrew");
        step.snapshot = Some(SnapshotSection::new(
            "brew",
            vec![SnapshotCommand::new("brew", ["list", "--versions"])],
        ));

        let section = step
            .snapshot()
            .unwrap_or_else(|| panic!("snapshot should be present"));
        assert_eq!(section.group(), "brew");
        assert_eq!(section.commands()[0].arguments(), ["list", "--versions"]);
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
        assert!(!error.is_registry_unreachable());
        assert_eq!(
            error.to_string(),
            "step `brew` failed during run: upgrade failed"
        );
    }

    #[test]
    fn network_errors_are_marked_as_an_unreachable_registry() {
        let error = StepError::check("brew", "curl: (6) Could not resolve host: formulae.brew.sh");

        assert!(error.is_registry_unreachable());
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

    #[test]
    fn an_outdated_item_shows_its_reason() {
        let item = OutdatedItem::new("nightly")
            .with_current("1.0")
            .with_latest("latest")
            .with_reason("version :latest; needs --greedy");

        assert_eq!(item.reason(), Some("version :latest; needs --greedy"));
        assert_eq!(
            item.to_string(),
            "nightly 1.0 -> latest (version :latest; needs --greedy)"
        );
    }
}
