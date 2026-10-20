//! The step registry: every known step, filtered and ordered for a run.
//!
//! Steps are registered in the order they should run when nothing else
//! constrains them. [`Step::prerequisites`] then pulls a step after the steps
//! it depends on, so registration order never has to encode dependencies such
//! as "Homebrew formulae before casks" or "mise after brew".
//!
//! [`Registry::ordered_for`] also drops steps that do not support the platform
//! of the run. A prerequisite that is filtered out this way does not constrain
//! its dependants: a step can declare a macOS-only prerequisite and still run
//! on Linux.

use std::collections::{HashMap, HashSet};

use crate::platform::Platform;
use crate::step::{Step, StepId};

/// A problem that prevents the registry from producing a run order.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// The prerequisites of the listed steps form a cycle.
    #[error("step dependency cycle: {}", .steps.join(" -> "))]
    DependencyCycle {
        /// Ids of the steps that could not be ordered.
        steps: Vec<StepId>,
    },
}

/// The set of known steps.
///
/// A registry owns its steps; [`Registry::ordered_for`] borrows them so the
/// caller can detect, check and run each one in turn.
#[derive(Default)]
pub struct Registry {
    steps: Vec<Box<dyn Step>>,
}

impl Registry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a step and returns the registry for chaining.
    pub fn register(&mut self, step: impl Step + 'static) -> &mut Self {
        self.steps.push(Box::new(step));
        self
    }

    /// Adds a step and returns the registry, for use in a builder chain.
    #[must_use]
    pub fn with(mut self, step: impl Step + 'static) -> Self {
        self.register(step);
        self
    }

    /// Number of registered steps, before platform filtering.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Returns `true` when no steps are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Every registered step, in registration order.
    pub fn steps(&self) -> impl Iterator<Item = &dyn Step> {
        self.steps.iter().map(|step| step.as_ref())
    }

    /// Steps that support `platform`, ordered so prerequisites run first.
    ///
    /// Independent steps keep their registration order. Prerequisites that are
    /// not registered or were filtered out for this platform are ignored.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DependencyCycle`] when prerequisites form a
    /// cycle and no complete order exists.
    pub fn ordered_for(&self, platform: Platform) -> Result<Vec<&dyn Step>, RegistryError> {
        let included: Vec<&dyn Step> = self
            .steps
            .iter()
            .map(|step| step.as_ref())
            .filter(|step| step.platforms().contains(&platform))
            .collect();

        let mut index_by_id: HashMap<StepId, usize> = HashMap::new();
        for (index, step) in included.iter().enumerate() {
            index_by_id.entry(step.id()).or_insert(index);
        }

        let mut indegree = vec![0_usize; included.len()];
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); included.len()];
        for (index, step) in included.iter().enumerate() {
            let mut seen = HashSet::new();
            for prerequisite in step.prerequisites() {
                let Some(&prerequisite_index) = index_by_id.get(prerequisite) else {
                    continue;
                };
                if seen.insert(prerequisite_index) {
                    indegree[index] += 1;
                    dependents[prerequisite_index].push(index);
                }
            }
        }

        let mut ordered = Vec::with_capacity(included.len());
        let mut emitted = vec![false; included.len()];
        loop {
            let next = (0..included.len()).find(|&index| !emitted[index] && indegree[index] == 0);
            let Some(index) = next else {
                break;
            };
            emitted[index] = true;
            ordered.push(included[index]);
            for &dependent in &dependents[index] {
                indegree[dependent] -= 1;
            }
        }

        if ordered.len() != included.len() {
            let steps = (0..included.len())
                .filter(|&index| !emitted[index])
                .map(|index| included[index].id())
                .collect();
            return Err(RegistryError::DependencyCycle { steps });
        }

        Ok(ordered)
    }

    /// Steps for the platform this binary was compiled for.
    ///
    /// Returns an empty list on an operating system freshen does not support.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DependencyCycle`] when prerequisites form a
    /// cycle and no complete order exists.
    pub fn ordered_for_current(&self) -> Result<Vec<&dyn Step>, RegistryError> {
        match Platform::current() {
            Some(platform) => self.ordered_for(platform),
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Registry, RegistryError};
    use crate::platform::Platform;
    use crate::step::{CheckReport, Detection, Step, StepError, StepId};

    struct TestStep {
        id: StepId,
        platforms: &'static [Platform],
        prerequisites: &'static [StepId],
    }

    impl TestStep {
        fn new(id: StepId) -> Self {
            Self {
                id,
                platforms: &Platform::ALL,
                prerequisites: &[],
            }
        }

        fn on(id: StepId, platforms: &'static [Platform]) -> Self {
            Self {
                id,
                platforms,
                prerequisites: &[],
            }
        }

        fn after(id: StepId, prerequisites: &'static [StepId]) -> Self {
            Self {
                id,
                platforms: &Platform::ALL,
                prerequisites,
            }
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

        fn prerequisites(&self) -> &'static [StepId] {
            self.prerequisites
        }

        fn platforms(&self) -> &'static [Platform] {
            self.platforms
        }
    }

    fn ids(steps: &[&dyn Step]) -> Vec<StepId> {
        steps.iter().map(|step| step.id()).collect()
    }

    fn platforms_for(platform: Platform) -> &'static [Platform] {
        match platform {
            Platform::Macos => &[Platform::Macos],
            Platform::Linux => &[Platform::Linux],
            Platform::Windows => &[Platform::Windows],
        }
    }

    #[test]
    fn an_empty_registry_has_no_steps() {
        let registry = Registry::new();

        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.steps().next().is_none());
        assert_eq!(
            ids(&registry.ordered_for(Platform::Linux).unwrap_or_default()),
            Vec::<StepId>::new()
        );
    }

    #[test]
    fn keeps_registration_order_for_independent_steps() {
        let registry = Registry::new()
            .with(TestStep::new("uv"))
            .with(TestStep::new("mise"))
            .with(TestStep::new("brew"));

        let ordered = registry.ordered_for(Platform::Linux);

        assert_eq!(
            ordered.as_deref().map(ids).unwrap_or_default(),
            ["uv", "mise", "brew"]
        );
    }

    #[test]
    fn runs_prerequisites_before_dependants() {
        let registry = Registry::new()
            .with(TestStep::after("casks", &["brew"]))
            .with(TestStep::new("brew"));

        let ordered = registry.ordered_for(Platform::Macos);

        assert_eq!(
            ordered.as_deref().map(ids).unwrap_or_default(),
            ["brew", "casks"]
        );
    }

    #[test]
    fn orders_brew_formulae_casks_and_mise() {
        let registry = Registry::new()
            .with(TestStep::new("brew"))
            .with(TestStep::new("formulae"))
            .with(TestStep::after("casks", &["formulae"]))
            .with(TestStep::after("mise", &["brew"]));

        let ordered = registry.ordered_for(Platform::Macos);

        assert_eq!(
            ordered.as_deref().map(ids).unwrap_or_default(),
            ["brew", "formulae", "casks", "mise"]
        );
    }

    #[test]
    fn ignores_duplicate_prerequisites() {
        let registry = Registry::new()
            .with(TestStep::new("brew"))
            .with(TestStep::after("casks", &["brew", "brew"]));

        let ordered = registry.ordered_for(Platform::Macos);

        assert_eq!(
            ordered.as_deref().map(ids).unwrap_or_default(),
            ["brew", "casks"]
        );
    }

    #[test]
    fn filters_steps_by_platform() {
        let registry = Registry::new()
            .with(TestStep::on("brew", &[Platform::Macos]))
            .with(TestStep::on("apt", &[Platform::Linux]))
            .with(TestStep::on("winget", &[Platform::Windows]))
            .with(TestStep::new("uv"));

        let ordered = registry.ordered_for(Platform::Linux);

        assert_eq!(
            ordered.as_deref().map(ids).unwrap_or_default(),
            ["apt", "uv"]
        );
    }

    #[test]
    fn a_filtered_out_prerequisite_does_not_block_its_dependant() {
        let registry = Registry::new()
            .with(TestStep::on("brew", &[Platform::Macos]))
            .with(TestStep::after("mise", &["brew"]));

        let on_linux = registry.ordered_for(Platform::Linux);
        let on_macos = registry.ordered_for(Platform::Macos);

        assert_eq!(on_linux.as_deref().map(ids).unwrap_or_default(), ["mise"]);
        assert_eq!(
            on_macos.as_deref().map(ids).unwrap_or_default(),
            ["brew", "mise"]
        );
    }

    #[test]
    fn a_step_with_no_platforms_never_runs() {
        let registry = Registry::new().with(TestStep::on("never", &[]));

        for platform in Platform::ALL {
            let ordered = registry.ordered_for(platform);
            assert!(ordered.as_deref().map(ids).unwrap_or_default().is_empty());
        }
    }

    #[test]
    fn orders_for_the_current_platform() {
        let Some(current) = Platform::current() else {
            return;
        };
        let other = Platform::ALL
            .into_iter()
            .find(|platform| *platform != current);

        let mut registry = Registry::new().with(TestStep::on("current", platforms_for(current)));
        if let Some(other) = other {
            registry = registry.with(TestStep::on("other", platforms_for(other)));
        }

        let ordered = registry.ordered_for_current();

        assert_eq!(ordered.as_deref().map(ids).unwrap_or_default(), ["current"]);
    }

    #[test]
    fn rejects_dependency_cycles() {
        let registry = Registry::new()
            .with(TestStep::after("first", &["second"]))
            .with(TestStep::after("second", &["first"]));

        let error = registry.ordered_for(Platform::Linux).err();

        assert_eq!(
            error,
            Some(RegistryError::DependencyCycle {
                steps: vec!["first", "second"],
            })
        );
    }
}
