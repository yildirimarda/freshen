//! Rich terminal output: step progress and spinners when interactive, plain
//! output when piped or in CI.
//!
//! [`Progress`] announces every step as it starts and finishes. When the run
//! is interactive it renders an [`indicatif`] spinner per step that updates in
//! place; when stdout is piped or the run is in CI it falls back to plain
//! one-line messages through [`crate::output::human`].
//!
//! [`indicatif`]: https://crates.io/crates/indicatif

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::output::human;
use crate::step::{SkipReason, Step, StepId};

/// The spinner frames used while a step is running.
const SPINNER_TICKS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠶", "⠦", "⠧", "⠇", "⠏"];

/// Rich progress output for a run of steps.
///
/// The interactive flag decides the rendering mode. It is read from the
/// attached terminal by default and can be overridden in tests so both the
/// spinner and the plain path can be exercised without a real terminal.
pub struct Progress {
    multi: MultiProgress,
    bars: Mutex<HashMap<StepId, ProgressBar>>,
    interactive: bool,
}

impl Progress {
    /// Creates progress output that follows the real terminal: spinners when
    /// interactive, plain lines when piped or in CI.
    pub fn new() -> Self {
        let interactive = crate::system::terminal_is_interactive();
        Self {
            multi: MultiProgress::new(),
            bars: Mutex::new(HashMap::new()),
            interactive,
        }
    }

    /// Overrides the interactive flag. Only available in tests so the spinner
    /// and plain paths can both be exercised without a real terminal.
    #[cfg(test)]
    pub fn with_interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    /// Announces that `step` is starting.
    pub fn step_started(&self, step: &dyn Step) {
        if self.interactive {
            let bar = ProgressBar::new_spinner();
            let style = ProgressStyle::default_spinner().tick_strings(SPINNER_TICKS);
            let style = match style.template("{spinner:.green} {msg}") {
                Ok(style) => style,
                Err(_) => ProgressStyle::default_spinner().tick_strings(SPINNER_TICKS),
            };
            bar.set_style(style);
            bar.set_message(format!("Running: {}", step.display_name()));
            bar.enable_steady_tick(Duration::from_millis(100));
            let bar = self.multi.add(bar);
            self.bars
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(step.id(), bar);
        } else {
            human!("Running: {}", step.display_name());
        }
    }

    /// Announces that `step` finished successfully.
    pub fn step_finished_ok(&self, step: &dyn Step) {
        self.step_finished(step, true);
    }

    /// Announces that `step` failed.
    pub fn step_finished_failed(&self, step: &dyn Step) {
        self.step_finished(step, false);
    }

    /// Announces that `step` was skipped.
    pub fn step_skipped(&self, step: &dyn Step, reason: SkipReason) {
        if self.interactive {
            if let Some(bar) = self
                .bars
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&step.id())
            {
                bar.finish_with_message(format!("Skipped: {} ({})", step.display_name(), reason));
            }
        } else {
            human!("Skipped: {} - {}", step.display_name(), reason);
        }
    }

    fn step_finished(&self, step: &dyn Step, ok: bool) {
        if self.interactive {
            if let Some(bar) = self
                .bars
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&step.id())
            {
                if ok {
                    bar.finish_with_message(format!("Done: {}", step.display_name()));
                } else {
                    bar.finish_with_message(format!("Failed: {}", step.display_name()));
                }
            }
        } else {
            if ok {
                human!("Done: {}", step.display_name());
            } else {
                human!("Failed: {}", step.display_name());
            }
        }
    }
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::platform::Platform;
    use crate::step::{CheckReport, Detection, SkipReason, Step, StepError, StepId};

    use super::Progress;

    struct FakeStep {
        id: StepId,
        name: &'static str,
    }

    impl FakeStep {
        fn new(id: StepId, name: &'static str) -> Self {
            Self { id, name }
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
            Ok(Detection::Installed)
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

    fn bars(
        progress: &Progress,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<StepId, indicatif::ProgressBar>> {
        progress
            .bars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn step_started_records_a_spinner_bar_when_interactive() {
        let progress = Progress::new().with_interactive(true);
        let step = FakeStep::new("fake", "Fake");
        progress.step_started(&step);
        assert!(bars(&progress).contains_key(&step.id()));
    }

    #[test]
    fn step_finished_ok_removes_the_spinner_bar_when_interactive() {
        let progress = Progress::new().with_interactive(true);
        let step = FakeStep::new("fake", "Fake");
        progress.step_started(&step);
        progress.step_finished_ok(&step);
        assert!(bars(&progress).is_empty());
    }

    #[test]
    fn step_finished_failed_removes_the_spinner_bar_when_interactive() {
        let progress = Progress::new().with_interactive(true);
        let step = FakeStep::new("fake", "Fake");
        progress.step_started(&step);
        progress.step_finished_failed(&step);
        assert!(bars(&progress).is_empty());
    }

    #[test]
    fn step_skipped_removes_the_spinner_bar_when_interactive() {
        let progress = Progress::new().with_interactive(true);
        let step = FakeStep::new("fake", "Fake");
        progress.step_started(&step);
        progress.step_skipped(&step, SkipReason::NotInstalled);
        assert!(bars(&progress).is_empty());
    }

    #[test]
    fn plain_mode_keeps_no_spinner_bars() {
        let progress = Progress::new().with_interactive(false);
        let step = FakeStep::new("fake", "Fake");
        progress.step_started(&step);
        progress.step_finished_ok(&step);
        assert!(bars(&progress).is_empty());
    }

    #[test]
    fn finishing_a_step_that_was_not_started_is_a_no_op() {
        let progress = Progress::new().with_interactive(true);
        let step = FakeStep::new("fake", "Fake");
        progress.step_finished_ok(&step);
        assert!(bars(&progress).is_empty());
    }
}
