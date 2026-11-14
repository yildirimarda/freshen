//! Generating one documentation page per step from the step registry.
//!
//! Every registered step gets a Markdown page whose title, id, supported
//! platforms, prerequisites and conflicts are read straight from the step
//! registry. Steps can be documented further by hand; the generated part is
//! always kept in sync with the registry.

use std::fmt::Write;

use crate::platform::Platform;
use crate::registry::Registry;
use crate::step::StepId;

/// One generated documentation page for a single registered step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepPage {
    /// Stable identifier of the step, e.g. `"brew"`.
    pub id: StepId,
    /// Name shown to the user, e.g. `"Homebrew"`.
    pub name: String,
    /// Operating systems the step runs on.
    pub platforms: Vec<Platform>,
    /// Steps that must run before this one, by id.
    pub prerequisites: Vec<StepId>,
    /// Steps that must not run in the same run as this one, by id.
    pub conflicts: Vec<StepId>,
}

/// One Markdown page per registered step, generated from the registry.
///
/// Pages are produced in registration order, so the docs navigation follows
/// the same sequence as a run.
pub fn pages(registry: &Registry) -> Vec<StepPage> {
    registry
        .steps()
        .map(|step| StepPage {
            id: step.id(),
            name: step.display_name().to_owned(),
            platforms: step.platforms().to_vec(),
            prerequisites: step.prerequisites().to_vec(),
            conflicts: step.conflicts().to_vec(),
        })
        .collect()
}

/// Renders a [`StepPage`] as Markdown.
pub fn render(page: &StepPage) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "# {}", page.name);
    let _ = writeln!(out);
    let _ = writeln!(out, "Step id: `{}`", page.id);
    let _ = writeln!(out);
    let _ = writeln!(out, "## Supported platforms");
    let _ = writeln!(out);
    if page.platforms.is_empty() {
        let _ = writeln!(out, "This step does not run on any platform.");
    } else {
        for platform in &page.platforms {
            let _ = writeln!(out, "- {}", platform);
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Prerequisites");
    let _ = writeln!(out);
    if page.prerequisites.is_empty() {
        let _ = writeln!(out, "None.");
    } else {
        for prerequisite in &page.prerequisites {
            let _ = writeln!(out, "- [{prerequisite}]({prerequisite}.md)");
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Conflicts");
    let _ = writeln!(out);
    if page.conflicts.is_empty() {
        let _ = writeln!(out, "None.");
    } else {
        for conflict in &page.conflicts {
            let _ = writeln!(out, "- `{conflict}`");
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "---");
    let _ = writeln!(out);
    let _ = writeln!(out, "This page is generated from the step registry. The step's own behaviour—what it detects, checks and upgrades—is described in its module documentation.");

    out
}

#[cfg(test)]
mod tests {
    use super::{pages, render, StepPage};
    use crate::builtin_steps;
    use crate::platform::Platform;
    use crate::registry::Registry;
    use crate::step::{CheckReport, Detection, Step, StepError, StepId};
    use std::path::Path;

    struct TestStep {
        id: StepId,
        name: &'static str,
        platforms: &'static [Platform],
        prerequisites: &'static [StepId],
        conflicts: &'static [StepId],
    }

    impl TestStep {
        fn new(id: StepId) -> Self {
            Self {
                id,
                name: id,
                platforms: &Platform::ALL,
                prerequisites: &[],
                conflicts: &[],
            }
        }

        fn named(mut self, name: &'static str) -> Self {
            self.name = name;
            self
        }

        fn on(mut self, platforms: &'static [Platform]) -> Self {
            self.platforms = platforms;
            self
        }

        fn after(mut self, prerequisites: &'static [StepId]) -> Self {
            self.prerequisites = prerequisites;
            self
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

        fn conflicts(&self) -> &'static [StepId] {
            self.conflicts
        }

        fn platforms(&self) -> &'static [Platform] {
            self.platforms
        }
    }

    #[test]
    fn generates_one_page_per_step_in_registration_order() {
        let registry = Registry::new()
            .with(TestStep::new("brew").named("Homebrew"))
            .with(TestStep::new("uv"));

        let page_ids: Vec<StepId> = pages(&registry).iter().map(|page| page.id).collect();
        assert_eq!(page_ids, ["brew", "uv"]);
        assert_eq!(pages(&registry).len(), registry.len());
    }

    #[test]
    fn page_contents_come_from_the_registry() {
        let registry = Registry::new().with(
            TestStep::new("brew-formulae")
                .named("Homebrew formulae")
                .on(&[Platform::Macos])
                .after(&["brew"]),
        );

        let page = &pages(&registry)[0];
        assert_eq!(page.id, "brew-formulae");
        assert_eq!(page.name, "Homebrew formulae");
        assert_eq!(page.platforms, &[Platform::Macos]);
        assert_eq!(page.prerequisites, &["brew"]);
        assert!(page.conflicts.is_empty());
    }

    #[test]
    fn rendered_page_names_the_step_and_its_id() {
        let page = StepPage {
            id: "uv",
            name: "uv".to_owned(),
            platforms: Platform::ALL.to_vec(),
            prerequisites: vec![],
            conflicts: vec![],
        };
        let rendered = render(&page);
        assert!(rendered.contains("# uv"));
        assert!(rendered.contains("Step id: `uv`"));
        assert!(rendered.contains("macos"));
        assert!(rendered.contains("linux"));
        assert!(rendered.contains("windows"));
    }

    #[test]
    fn rendered_page_links_prerequisites() {
        let page = StepPage {
            id: "brew-casks",
            name: "Homebrew casks".to_owned(),
            platforms: vec![Platform::Macos],
            prerequisites: vec!["brew-formulae"],
            conflicts: vec![],
        };
        let rendered = render(&page);
        assert!(rendered.contains("[brew-formulae](brew-formulae.md)"));
    }

    #[test]
    fn empty_registry_produces_no_pages() {
        let registry = Registry::new();
        assert!(pages(&registry).is_empty());
    }

    #[test]
    fn committed_pages_match_the_real_registry() {
        let registry = builtin_steps::register_all(Registry::new(), None);
        let pages = pages(&registry);
        let steps_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/steps");
        assert!(steps_dir.is_dir(), "docs/steps must exist");
        let mut missing = Vec::new();
        for page in &pages {
            let path = steps_dir.join(format!("{}.md", page.id));
            if !path.is_file() {
                missing.push(page.id);
            }
        }
        assert!(
            missing.is_empty(),
            "docs pages missing for steps: {missing:?}"
        );
        assert_eq!(pages.len(), registry.len());
    }
}
