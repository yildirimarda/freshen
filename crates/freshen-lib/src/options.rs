//! Run options parsed from the command line and applied to every platform.
//!
//! The flags themselves are platform-independent: `--dry-run`, `--only`,
//! `--skip`, `--yes` and `--force-kill` mean the same thing on macOS, Linux
//! and Windows. Step availability is the only platform-specific input, and it
//! comes from the step registry rather than from the host operating system.

use std::collections::HashSet;
use std::fmt;

use crate::package_manager::PackageManagerConfig;
use crate::platform::Platform;
use crate::preflight::PreflightConfig;
use crate::registry::{Registry, RegistryError};
use crate::step::{Step, StepId};

/// The flag that named a step id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionFlag {
    /// `--only`
    Only,
    /// `--skip`
    Skip,
}

impl SelectionFlag {
    /// Lowercase name used in error messages and help text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Only => "only",
            Self::Skip => "skip",
        }
    }
}

impl fmt::Display for SelectionFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A problem that prevented a run plan from being built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// The step registry could not produce an order.
    #[error(transparent)]
    Registry(#[from] RegistryError),

    /// A flag named a step that is not registered at all.
    #[error("unknown step `{id}` in --{flag} (known steps: {known})")]
    UnknownStep {
        /// Flag that named the step.
        flag: SelectionFlag,
        /// Id that was not found.
        id: String,
        /// Comma-separated list of known step ids.
        known: String,
    },

    /// `--only` named a registered step that cannot run on this platform.
    #[error("step `{id}` in --only is not available on {platform}")]
    UnavailableStep {
        /// Id that is not available.
        id: String,
        /// Platform of the run.
        platform: Platform,
    },
}

impl PlanError {
    fn unknown(flag: SelectionFlag, id: &str, known: &[StepId]) -> Self {
        let mut names = known.to_vec();
        names.sort_unstable();
        names.dedup();
        let known = if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(", ")
        };
        Self::UnknownStep {
            flag,
            id: id.to_owned(),
            known,
        }
    }
}

/// The subset of steps selected by `--only` and `--skip`.
///
/// `--only` keeps the named steps (all steps when it is absent) and `--skip`
/// removes steps afterwards, so a step named by both is skipped. Ids are
/// matched exactly; unknown ids are rejected when the plan is built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepSelection {
    only: Vec<String>,
    skip: Vec<String>,
}

impl StepSelection {
    /// Creates a selection that keeps every step.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the ids selected by `--only`.
    #[must_use]
    pub fn with_only(mut self, ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.only = ids.into_iter().map(Into::into).collect();
        self
    }

    /// Replaces the ids excluded by `--skip`.
    #[must_use]
    pub fn with_skip(mut self, ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.skip = ids.into_iter().map(Into::into).collect();
        self
    }

    /// Ids named by `--only`, in the order they were given.
    #[must_use]
    pub fn only(&self) -> &[String] {
        &self.only
    }

    /// Ids named by `--skip`, in the order they were given.
    #[must_use]
    pub fn skip(&self) -> &[String] {
        &self.skip
    }

    /// Returns `true` when neither flag restricts the run.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.only.is_empty() && self.skip.is_empty()
    }

    /// Returns `true` when `id` should be part of the run.
    #[must_use]
    pub fn includes(&self, id: &str) -> bool {
        (self.only.is_empty() || self.only.iter().any(|only| only == id))
            && !self.skip.iter().any(|skip| skip == id)
    }

    fn validate(
        &self,
        known: &[StepId],
        available: &[StepId],
        platform: Platform,
    ) -> Result<(), PlanError> {
        let known_ids: HashSet<&str> = known.iter().copied().collect();
        let available_ids: HashSet<&str> = available.iter().copied().collect();

        for id in &self.only {
            if !known_ids.contains(id.as_str()) {
                return Err(PlanError::unknown(SelectionFlag::Only, id, known));
            }
            if !available_ids.contains(id.as_str()) {
                return Err(PlanError::UnavailableStep {
                    id: id.clone(),
                    platform,
                });
            }
        }

        for id in &self.skip {
            if !known_ids.contains(id.as_str()) {
                return Err(PlanError::unknown(SelectionFlag::Skip, id, known));
            }
        }

        Ok(())
    }
}

/// Everything the command line controls about a run.
///
/// The same options are used on every platform; the runner carries them and
/// exposes them through [`crate::Runner::options`] so steps that prompt or quit
/// applications can share one source of truth.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOptions {
    dry_run: bool,
    assume_yes: bool,
    force_kill: bool,
    selection: StepSelection,
    preflight: PreflightConfig,
    package_manager: PackageManagerConfig,
}

impl RunOptions {
    /// Creates the default options: run every step, ask before quitting apps
    /// and never force-kill.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `--dry-run`: detect each step but never run or clean it up.
    #[must_use]
    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Sets `--yes`: skip interactive confirmation prompts.
    #[must_use]
    pub fn with_assume_yes(mut self, assume_yes: bool) -> Self {
        self.assume_yes = assume_yes;
        self
    }

    /// Sets `--force-kill`: kill apps that do not quit gracefully.
    #[must_use]
    pub fn with_force_kill(mut self, force_kill: bool) -> Self {
        self.force_kill = force_kill;
        self
    }

    /// Replaces the `--only`/`--skip` selection.
    #[must_use]
    pub fn with_selection(mut self, selection: StepSelection) -> Self {
        self.selection = selection;
        self
    }

    /// Replaces the preflight check configuration.
    #[must_use]
    pub fn with_preflight(mut self, preflight: PreflightConfig) -> Self {
        self.preflight = preflight;
        self
    }

    /// Replaces the other-package-manager check configuration.
    #[must_use]
    pub fn with_package_manager(mut self, package_manager: PackageManagerConfig) -> Self {
        self.package_manager = package_manager;
        self
    }

    /// Returns `true` when `--dry-run` is set.
    #[must_use]
    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    /// Returns `true` when `--yes` is set.
    #[must_use]
    pub fn assume_yes(&self) -> bool {
        self.assume_yes
    }

    /// Returns `true` when `--force-kill` is set.
    #[must_use]
    pub fn force_kill(&self) -> bool {
        self.force_kill
    }

    /// The `--only`/`--skip` selection.
    #[must_use]
    pub fn selection(&self) -> &StepSelection {
        &self.selection
    }

    /// Configuration for the checks that run before an update.
    #[must_use]
    pub fn preflight(&self) -> &PreflightConfig {
        &self.preflight
    }

    /// Configuration for waiting on another package manager process.
    #[must_use]
    pub fn package_manager(&self) -> &PackageManagerConfig {
        &self.package_manager
    }

    /// Builds the ordered list of steps this run should execute on `platform`.
    ///
    /// Unknown ids in either flag are an error, and an `--only` id that is not
    /// available on `platform` is an error too. Steps that `--skip` names but
    /// that cannot run on `platform` are accepted, since they are not part of
    /// the run anyway.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] when the registry cannot be ordered or the
    /// selection names an unknown or unavailable step.
    pub fn plan<'a>(
        &self,
        registry: &'a Registry,
        platform: Platform,
    ) -> Result<Vec<&'a dyn Step>, PlanError> {
        let ordered = registry.ordered_for(platform)?;
        let known: Vec<StepId> = registry.steps().map(Step::id).collect();
        let available: Vec<StepId> = ordered.iter().map(|step| step.id()).collect();

        self.selection.validate(&known, &available, platform)?;

        Ok(ordered
            .into_iter()
            .filter(|step| self.selection.includes(step.id()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{PlanError, RunOptions, SelectionFlag, StepSelection};
    use crate::platform::Platform;
    use crate::preflight::{PreflightCheck, PreflightConfig};
    use crate::registry::Registry;
    use crate::step::{CheckReport, Detection, Step, StepError, StepId};

    struct TestStep {
        id: StepId,
        platforms: &'static [Platform],
    }

    impl TestStep {
        fn on(id: StepId, platforms: &'static [Platform]) -> Self {
            Self { id, platforms }
        }
    }

    impl Step for TestStep {
        fn id(&self) -> StepId {
            self.id
        }

        fn display_name(&self) -> &str {
            self.id
        }

        fn detect(&self) -> Result<Detection, StepError> {
            Ok(Detection::Installed)
        }

        fn check(&self) -> Result<CheckReport, StepError> {
            Ok(CheckReport::up_to_date())
        }

        fn run(&self) -> Result<(), StepError> {
            Ok(())
        }

        fn platforms(&self) -> &'static [Platform] {
            self.platforms
        }
    }

    fn ids(steps: &[&dyn Step]) -> Vec<StepId> {
        steps.iter().map(|step| step.id()).collect()
    }

    fn registry() -> Registry {
        Registry::new()
            .with(TestStep::on("common", &Platform::ALL))
            .with(TestStep::on("mac", &[Platform::Macos]))
            .with(TestStep::on("linux", &[Platform::Linux]))
            .with(TestStep::on("windows", &[Platform::Windows]))
    }

    fn plan(
        options: &RunOptions,
        registry: &Registry,
        platform: Platform,
    ) -> Result<Vec<StepId>, PlanError> {
        options.plan(registry, platform).map(|steps| ids(&steps))
    }

    #[test]
    fn defaults_run_everything_and_do_not_skip_prompts() {
        let options = RunOptions::new();

        assert!(!options.is_dry_run());
        assert!(!options.assume_yes());
        assert!(!options.force_kill());
        assert!(options.selection().is_empty());
    }

    #[test]
    fn builder_flags_are_independent() {
        let options = RunOptions::new()
            .with_dry_run(true)
            .with_assume_yes(true)
            .with_force_kill(true);

        assert!(options.is_dry_run());
        assert!(options.assume_yes());
        assert!(options.force_kill());
    }

    #[test]
    fn preflight_defaults_are_enabled_and_can_be_replaced() {
        let defaults = RunOptions::new();
        for check in PreflightCheck::ALL {
            assert!(defaults.preflight().is_enabled(check), "{check}");
        }

        let configured = RunOptions::new().with_preflight(
            PreflightConfig::new()
                .with_check(PreflightCheck::Network, false)
                .with_min_free_disk_bytes(42),
        );

        assert!(!configured.preflight().is_enabled(PreflightCheck::Network));
        assert!(configured.preflight().is_enabled(PreflightCheck::DiskSpace));
        assert_eq!(configured.preflight().min_free_disk_bytes(), 42);
    }

    #[test]
    fn only_keeps_named_steps_and_skip_removes_them() {
        let selection = StepSelection::new()
            .with_only(["common", "mac"])
            .with_skip(["mac"]);

        assert!(selection.includes("common"));
        assert!(!selection.includes("mac"));
        assert!(!selection.includes("linux"));
    }

    #[test]
    fn skip_alone_keeps_every_other_step() {
        let selection = StepSelection::new().with_skip(["mac"]);

        assert!(selection.includes("common"));
        assert!(selection.includes("linux"));
        assert!(!selection.includes("mac"));
    }

    #[test]
    fn empty_selection_includes_every_step() {
        let selection = StepSelection::new();

        assert!(selection.is_empty());
        assert!(selection.includes("anything"));
    }

    #[test]
    fn plan_keeps_registry_order_after_filtering() {
        let options =
            RunOptions::new().with_selection(StepSelection::new().with_only(["mac", "common"]));

        let planned = plan(&options, &registry(), Platform::Macos);

        assert_eq!(planned, Ok(vec!["common", "mac"]));
    }

    #[test]
    fn skip_of_an_unavailable_step_is_allowed() {
        let options = RunOptions::new().with_selection(StepSelection::new().with_skip(["mac"]));

        let planned = plan(&options, &registry(), Platform::Linux);

        assert_eq!(planned, Ok(vec!["common", "linux"]));
    }

    #[test]
    fn unknown_only_step_is_rejected_with_the_known_ids() {
        let options = RunOptions::new().with_selection(StepSelection::new().with_only(["nope"]));

        let error = plan(&options, &registry(), Platform::Linux).err();

        let Some(PlanError::UnknownStep { flag, id, known }) = error else {
            panic!("expected an unknown step error");
        };
        assert_eq!(flag, SelectionFlag::Only);
        assert_eq!(id, "nope");
        assert_eq!(known, "common, linux, mac, windows");
    }

    #[test]
    fn unknown_skip_step_is_rejected() {
        let options = RunOptions::new().with_selection(StepSelection::new().with_skip(["nope"]));

        let error = plan(&options, &registry(), Platform::Linux).err();

        let Some(PlanError::UnknownStep { flag, .. }) = error else {
            panic!("expected an unknown step error");
        };
        assert_eq!(flag, SelectionFlag::Skip);
    }

    #[test]
    fn only_for_an_unavailable_platform_is_rejected() {
        let options = RunOptions::new().with_selection(StepSelection::new().with_only(["mac"]));

        let error = plan(&options, &registry(), Platform::Linux).err();

        assert_eq!(
            error,
            Some(PlanError::UnavailableStep {
                id: "mac".to_owned(),
                platform: Platform::Linux,
            })
        );
    }

    #[test]
    fn selection_has_identical_semantics_on_every_platform() {
        let registry = registry();

        for platform in Platform::ALL {
            let platform_step = match platform {
                Platform::Macos => "mac",
                Platform::Linux => "linux",
                Platform::Windows => "windows",
            };

            let all = plan(&RunOptions::new(), &registry, platform);
            let only_common = plan(
                &RunOptions::new().with_selection(StepSelection::new().with_only(["common"])),
                &registry,
                platform,
            );
            let only_platform = plan(
                &RunOptions::new().with_selection(StepSelection::new().with_only([platform_step])),
                &registry,
                platform,
            );
            let skip_common = plan(
                &RunOptions::new().with_selection(StepSelection::new().with_skip(["common"])),
                &registry,
                platform,
            );

            assert_eq!(all, Ok(vec!["common", platform_step]), "{platform}");
            assert_eq!(only_common, Ok(vec!["common"]), "{platform}");
            assert_eq!(only_platform, Ok(vec![platform_step]), "{platform}");
            assert_eq!(skip_common, Ok(vec![platform_step]), "{platform}");
        }
    }

    #[test]
    fn dry_run_is_a_run_option_not_a_platform_condition() {
        let options = RunOptions::new().with_dry_run(true);

        for platform in Platform::ALL {
            assert_eq!(
                plan(&options, &registry(), platform),
                Ok(vec![
                    "common",
                    match platform {
                        Platform::Macos => "mac",
                        Platform::Linux => "linux",
                        Platform::Windows => "windows",
                    }
                ])
            );
            assert!(options.is_dry_run());
        }
    }
}
