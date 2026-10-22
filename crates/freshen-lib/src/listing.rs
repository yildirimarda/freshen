//! Inspecting every known step before a run.
//!
//! `--list-steps` answers three questions for each registered step: does it
//! support the current platform, was its tool detected, and would the current
//! `--only`/`--skip` selection skip it? The listing never runs a step, so it
//! is safe to use while deciding what a run should include.

use std::collections::HashMap;
use std::fmt;

use crate::containers::ActiveContainerRuntime;
use crate::options::{RunOptions, SelectionFlag, StepSelection};
use crate::platform::Platform;
use crate::registry::Registry;
use crate::runner::SkipReason;
use crate::step::{Detection, Step, StepError, StepId};

/// Detection outcome shown by [`StepList`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionState {
    /// The tool managed by the step is installed.
    Installed,
    /// The tool managed by the step is missing.
    NotInstalled,
    /// The tool is installed, but the step cannot run in the current state.
    Unavailable,
    /// The step does not support the platform, so detection was not attempted.
    Unsupported,
    /// Detection itself failed.
    Failed(StepError),
}

impl DetectionState {
    /// Short human-readable name used in the listing.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::NotInstalled => "not installed",
            Self::Unavailable => "unavailable",
            Self::Unsupported => "not checked",
            Self::Failed(_) => "detection failed",
        }
    }
}

impl fmt::Display for DetectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One known step as it would take part in a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepListing {
    id: StepId,
    name: String,
    detection: DetectionState,
    skip_reason: Option<SkipReason>,
}

impl StepListing {
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

    /// Whether the tool was detected on this machine.
    #[must_use]
    pub fn detection(&self) -> &DetectionState {
        &self.detection
    }

    /// Why a run would skip this step, when it would.
    #[must_use]
    pub fn skip_reason(&self) -> Option<SkipReason> {
        self.skip_reason
    }
}

/// Every known step, in registration order, with its detection and skip state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepList {
    steps: Vec<StepListing>,
    container_runtime: Option<ActiveContainerRuntime>,
}

impl StepList {
    /// The listed steps.
    #[must_use]
    pub fn steps(&self) -> &[StepListing] {
        &self.steps
    }

    /// The active container runtime, when one was detected or overridden.
    #[must_use]
    pub fn container_runtime(&self) -> Option<&ActiveContainerRuntime> {
        self.container_runtime.as_ref()
    }

    /// Number of known steps.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Returns `true` when the registry has no steps.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

impl fmt::Display for StepList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(runtime) = &self.container_runtime {
            writeln!(f, "Container runtime: {runtime}")?;
        }
        if self.steps.is_empty() {
            return writeln!(f, "No known steps.");
        }

        let id_width = self
            .steps
            .iter()
            .map(|step| step.id.len())
            .max()
            .unwrap_or(0);
        let name_width = self
            .steps
            .iter()
            .map(|step| step.name.len())
            .max()
            .unwrap_or(0);

        writeln!(f, "Steps ({}):", self.steps.len())?;
        for step in &self.steps {
            write!(
                f,
                "  {:<id_width$}  {:<name_width$}  {}",
                step.id, step.name, step.detection
            )?;
            if let Some(reason) = step.skip_reason {
                write!(f, " - skipped: {reason}")?;
            }
            if let DetectionState::Failed(error) = &step.detection {
                write!(f, " - {error}")?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

/// Lists every step in `registry` for `platform`, detecting each available
/// step and applying the selection and the configuration file to decide why a
/// run would skip it.
///
/// Steps that do not support `platform` are never detected. Detection errors
/// are reported in the listing instead of aborting it, so one broken tool
/// does not hide the rest of the steps.
#[must_use]
pub fn list_steps(registry: &Registry, options: &RunOptions, platform: Platform) -> StepList {
    let conflicts = conflict_skips(registry, options, platform);
    let steps = registry
        .steps()
        .map(|step| list_step(step, options, platform, &conflicts))
        .collect();

    StepList {
        steps,
        container_runtime: options.container_runtime().cloned(),
    }
}

fn list_step(
    step: &dyn Step,
    options: &RunOptions,
    platform: Platform,
    conflicts: &HashMap<StepId, StepId>,
) -> StepListing {
    let id = step.id();
    let name = step.display_name().to_owned();

    if !step.platforms().contains(&platform) {
        return StepListing {
            id,
            name,
            detection: DetectionState::Unsupported,
            skip_reason: Some(SkipReason::UnsupportedPlatform),
        };
    }

    let (detection, blocked) = match step.detect() {
        Ok(Detection::Installed) => (DetectionState::Installed, None),
        Ok(Detection::NotInstalled) => (DetectionState::NotInstalled, None),
        Ok(Detection::Unavailable(reason)) => (DetectionState::Unavailable, Some(reason)),
        Err(error) => (DetectionState::Failed(error), None),
    };

    let skip_reason = selection_reason(options.selection(), id)
        .or_else(|| (!options.is_step_enabled(id)).then_some(SkipReason::Disabled))
        .or_else(|| {
            conflicts
                .get(&id)
                .map(|&earlier| SkipReason::Conflict(earlier))
        })
        .or(blocked)
        .or((detection == DetectionState::NotInstalled).then_some(SkipReason::NotInstalled));

    StepListing {
        id,
        name,
        detection,
        skip_reason,
    }
}

fn conflict_skips(
    registry: &Registry,
    options: &RunOptions,
    platform: Platform,
) -> HashMap<StepId, StepId> {
    let Ok(ordered) = registry.ordered_for(platform) else {
        return HashMap::new();
    };

    let selection = options.selection();
    let mut running: Vec<&dyn Step> = Vec::new();
    let mut skipped = HashMap::new();

    for step in ordered
        .into_iter()
        .filter(|step| selection.includes(step.id()))
        .filter(|step| options.is_step_enabled(step.id()))
    {
        let conflict = running.iter().find(|earlier| {
            earlier.conflicts().contains(&step.id()) || step.conflicts().contains(&earlier.id())
        });

        match conflict {
            Some(earlier) => {
                skipped.insert(step.id(), earlier.id());
            }
            None => running.push(step),
        }
    }

    skipped
}

fn selection_reason(selection: &StepSelection, id: &str) -> Option<SkipReason> {
    if selection.skip().iter().any(|skip| skip == id) {
        Some(SkipReason::NotSelected(SelectionFlag::Skip))
    } else if !selection.only().is_empty() && !selection.only().iter().any(|only| only == id) {
        Some(SkipReason::NotSelected(SelectionFlag::Only))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{list_steps, DetectionState, StepList};
    use crate::options::{RunOptions, SelectionFlag, StepSelection};
    use crate::platform::Platform;
    use crate::registry::Registry;
    use crate::runner::{Runner, SkipReason};
    use crate::step::{CheckReport, Detection, Step, StepError, StepId};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct TestStep {
        id: StepId,
        name: &'static str,
        platforms: &'static [Platform],
        detection: Detection,
        fail_detect: bool,
        prerequisites: &'static [StepId],
        conflicts: &'static [StepId],
        detect_calls: Arc<AtomicUsize>,
    }

    impl TestStep {
        fn new(id: StepId, platforms: &'static [Platform]) -> Self {
            Self {
                id,
                name: id,
                platforms,
                detection: Detection::Installed,
                fail_detect: false,
                prerequisites: &[],
                conflicts: &[],
                detect_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn detection(mut self, detection: Detection) -> Self {
            self.detection = detection;
            self
        }

        fn after(mut self, prerequisites: &'static [StepId]) -> Self {
            self.prerequisites = prerequisites;
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

        fn detect_calls(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.detect_calls)
        }
    }

    impl Step for TestStep {
        fn id(&self) -> StepId {
            self.id
        }

        fn display_name(&self) -> &str {
            self.name
        }

        fn detect(&self) -> Result<Detection, StepError> {
            self.detect_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_detect {
                return Err(StepError::detect(self.id, "cannot detect"));
            }
            Ok(self.detection)
        }

        fn check(&self) -> Result<CheckReport, StepError> {
            Ok(CheckReport::up_to_date())
        }

        fn run(&self) -> Result<(), StepError> {
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

    fn registry(steps: impl IntoIterator<Item = TestStep>) -> Registry {
        steps
            .into_iter()
            .fold(Registry::new(), |registry, step| registry.with(step))
    }

    fn list(registry: &Registry, selection: &StepSelection) -> StepList {
        list_steps(
            registry,
            &RunOptions::new().with_selection(selection.clone()),
            Platform::Linux,
        )
    }

    #[test]
    fn lists_every_step_in_registration_order() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("uv", &Platform::ALL),
            TestStep::new("mise", &Platform::ALL),
        ]);

        let listed = list(&registry, &StepSelection::new());

        assert_eq!(listed.len(), 3);
        assert_eq!(
            listed
                .steps()
                .iter()
                .map(|step| step.id())
                .collect::<Vec<_>>(),
            ["brew", "uv", "mise"]
        );
        assert!(listed
            .steps()
            .iter()
            .all(|step| step.detection() == &DetectionState::Installed));
    }

    #[test]
    fn reports_missing_tools_as_skipped() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("uv", &Platform::ALL).detection(Detection::NotInstalled),
        ]);

        let listed = list(&registry, &StepSelection::new());

        assert_eq!(listed.steps()[0].skip_reason(), None);
        assert_eq!(listed.steps()[1].detection(), &DetectionState::NotInstalled);
        assert_eq!(
            listed.steps()[1].skip_reason(),
            Some(SkipReason::NotInstalled)
        );
    }

    #[test]
    fn unsupported_steps_are_not_detected_and_say_why() {
        let mac_only = TestStep::new("mas", &[Platform::Macos]);
        let calls = mac_only.detect_calls();
        let registry = registry([mac_only]);

        let listed = list(&registry, &StepSelection::new());

        assert_eq!(listed.steps()[0].detection(), &DetectionState::Unsupported);
        assert_eq!(
            listed.steps()[0].skip_reason(),
            Some(SkipReason::UnsupportedPlatform)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_step_that_is_installed_but_blocked_says_why_it_is_skipped() {
        let registry = registry([TestStep::new("mas", &Platform::ALL)
            .detection(Detection::Unavailable(SkipReason::NotSignedIn))]);

        let listed = list(&registry, &StepSelection::new());

        assert_eq!(listed.steps()[0].detection(), &DetectionState::Unavailable);
        assert_eq!(
            listed.steps()[0].skip_reason(),
            Some(SkipReason::NotSignedIn)
        );
        assert!(
            listed.to_string().contains("not signed in"),
            "listing: {listed}"
        );
    }

    #[test]
    fn only_and_skip_explain_the_exclusion() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("uv", &Platform::ALL),
            TestStep::new("mise", &Platform::ALL),
        ]);

        let only_brew = list(&registry, &StepSelection::new().with_only(["brew"]));
        assert_eq!(
            only_brew.steps()[1].skip_reason(),
            Some(SkipReason::NotSelected(SelectionFlag::Only))
        );

        let skip_uv = list(&registry, &StepSelection::new().with_skip(["uv"]));
        assert_eq!(
            skip_uv.steps()[1].skip_reason(),
            Some(SkipReason::NotSelected(SelectionFlag::Skip))
        );
    }

    #[test]
    fn selection_takes_precedence_over_a_missing_tool() {
        let registry =
            registry([TestStep::new("uv", &Platform::ALL).detection(Detection::NotInstalled)]);

        let listed = list(&registry, &StepSelection::new().with_skip(["uv"]));

        assert_eq!(
            listed.steps()[0].skip_reason(),
            Some(SkipReason::NotSelected(SelectionFlag::Skip))
        );
    }

    #[test]
    fn a_later_conflicting_step_names_the_earlier_one() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("mas", &Platform::ALL).conflicting(&["brew"]),
        ]);

        let listed = list(&registry, &StepSelection::new());

        assert_eq!(listed.steps()[0].skip_reason(), None);
        assert_eq!(
            listed.steps()[1].skip_reason(),
            Some(SkipReason::Conflict("brew"))
        );
        assert!(
            listed.to_string().contains("conflicts with `brew`"),
            "listing: {listed}"
        );
    }

    #[test]
    fn the_earlier_step_wins_even_when_it_declares_the_conflict() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL).conflicting(&["mas"]),
            TestStep::new("mas", &Platform::ALL),
        ]);

        let listed = list(&registry, &StepSelection::new());

        assert_eq!(listed.steps()[0].skip_reason(), None);
        assert_eq!(
            listed.steps()[1].skip_reason(),
            Some(SkipReason::Conflict("brew"))
        );
    }

    #[test]
    fn conflicts_ignore_steps_excluded_by_selection() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("mas", &Platform::ALL).conflicting(&["brew"]),
        ]);

        let listed = list(&registry, &StepSelection::new().with_skip(["brew"]));

        assert_eq!(listed.steps()[1].skip_reason(), None);
    }

    #[test]
    fn a_step_disabled_in_the_configuration_file_says_why() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("uv", &Platform::ALL),
        ]);
        let mut overrides = BTreeMap::new();
        overrides.insert("uv".to_owned(), false);
        let options = RunOptions::new().with_step_overrides(overrides);

        let listed = list_steps(&registry, &options, Platform::Linux);

        assert_eq!(listed.steps()[0].skip_reason(), None);
        assert_eq!(listed.steps()[1].skip_reason(), Some(SkipReason::Disabled));
        assert!(
            listed
                .to_string()
                .contains("disabled in the configuration file"),
            "listing: {listed}"
        );
    }

    #[test]
    fn a_disabled_step_does_not_conflict_with_the_rest_of_the_run() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("mas", &Platform::ALL).conflicting(&["brew"]),
        ]);
        let mut overrides = BTreeMap::new();
        overrides.insert("brew".to_owned(), false);
        let options = RunOptions::new().with_step_overrides(overrides);

        let listed = list_steps(&registry, &options, Platform::Linux);

        assert_eq!(listed.steps()[0].skip_reason(), Some(SkipReason::Disabled));
        assert_eq!(listed.steps()[1].skip_reason(), None);
    }

    #[test]
    fn a_conflict_skipped_step_does_not_hide_the_steps_that_follow() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("mas", &Platform::ALL).conflicting(&["brew"]),
            TestStep::new("uv", &Platform::ALL),
        ]);

        let listed = list(&registry, &StepSelection::new());

        assert_eq!(listed.steps()[2].skip_reason(), None);
    }

    #[test]
    fn conflicts_follow_run_order_not_registration_order() {
        let registry = registry([
            TestStep::new("mas", &Platform::ALL)
                .after(&["brew"])
                .conflicting(&["brew"]),
            TestStep::new("brew", &Platform::ALL),
        ]);

        let listed = list(&registry, &StepSelection::new());

        assert_eq!(
            listed.steps()[0].skip_reason(),
            Some(SkipReason::Conflict("brew"))
        );
        assert_eq!(listed.steps()[1].skip_reason(), None);
    }

    #[test]
    fn a_conflict_takes_precedence_over_a_missing_tool() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("mas", &Platform::ALL)
                .conflicting(&["brew"])
                .detection(Detection::NotInstalled),
        ]);

        let listed = list(&registry, &StepSelection::new());

        assert_eq!(
            listed.steps()[1].skip_reason(),
            Some(SkipReason::Conflict("brew"))
        );
    }

    #[test]
    fn the_listing_reports_the_same_conflict_reason_as_the_run_summary() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("mas", &Platform::ALL).conflicting(&["brew"]),
        ]);
        let selection = StepSelection::new();

        let listed = list(&registry, &selection);
        let Ok(ordered) = registry.ordered_for(Platform::Linux) else {
            panic!("registry should produce an order");
        };
        let summary =
            Runner::with_options(ordered, RunOptions::new().with_selection(selection)).run();

        assert_eq!(
            listed.steps()[1].skip_reason(),
            summary.steps()[1].skip_reason()
        );
        assert_eq!(
            summary.steps()[1].skip_reason(),
            Some(SkipReason::Conflict("brew"))
        );
    }

    #[test]
    fn a_detection_error_is_reported_without_hiding_other_steps() {
        let broken = TestStep::new("broken", &Platform::ALL).fail_detect();
        let registry = registry([broken, TestStep::new("uv", &Platform::ALL)]);

        let listed = list(&registry, &StepSelection::new());

        let DetectionState::Failed(error) = listed.steps()[0].detection() else {
            panic!("expected a detection failure");
        };
        assert_eq!(error.step(), "broken");
        assert_eq!(listed.steps()[0].skip_reason(), None);
        assert_eq!(listed.steps()[1].detection(), &DetectionState::Installed);
    }

    #[test]
    fn the_listing_names_steps_detection_and_skip_reasons() {
        let registry = registry([
            TestStep::new("brew", &Platform::ALL),
            TestStep::new("uv", &Platform::ALL).detection(Detection::NotInstalled),
            TestStep::new("mas", &[Platform::Macos]),
        ]);

        let text = list(&registry, &StepSelection::new()).to_string();

        assert!(text.contains("Steps (3):"), "{text}");
        assert!(text.contains("brew"), "{text}");
        assert!(text.contains("installed"), "{text}");
        assert!(text.contains("not installed"), "{text}");
        assert!(text.contains("skipped: not installed"), "{text}");
        assert!(text.contains("not supported on this platform"), "{text}");
    }

    #[test]
    fn an_empty_registry_says_there_are_no_steps() {
        let text = list(&Registry::new(), &StepSelection::new()).to_string();

        assert_eq!(text, "No known steps.\n");
    }
}
