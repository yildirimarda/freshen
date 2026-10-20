//! Test-only probe steps used by the stub harness.
//!
//! This module is compiled only with the `test-harness` feature. It reads
//! `FRESHEN_TEST_STEPS` (steps separated by `;`, each a program followed by
//! arguments) and registers one step per entry, in order. The steps always
//! report themselves as installed and run their command during `run()`, which
//! lets an integration test assert the exact call order against stubs on
//! `PATH`.

use freshen_lib::{
    CheckReport, CommandSpec, Detection, Platform, Registry, Step, StepError, StepId,
};

const ENV_VAR: &str = "FRESHEN_TEST_STEPS";

const IDS: [StepId; 16] = [
    "probe-0", "probe-1", "probe-2", "probe-3", "probe-4", "probe-5", "probe-6", "probe-7",
    "probe-8", "probe-9", "probe-10", "probe-11", "probe-12", "probe-13", "probe-14", "probe-15",
];

/// Returns `true` when the test harness asked for probe steps.
///
/// The probe steps replace the built-in registry, so a test that describes its
/// own steps never also runs the real Homebrew steps.
pub fn requested() -> bool {
    std::env::var_os(ENV_VAR).is_some()
}

/// Registers one probe step per `FRESHEN_TEST_STEPS` entry.
pub fn with_probe_steps(mut registry: Registry) -> Registry {
    let Some(value) = std::env::var_os(ENV_VAR) else {
        return registry;
    };
    let value = value.to_string_lossy();

    for (index, entry) in value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .enumerate()
    {
        if let Some(step) = ProbeStep::parse(index, entry) {
            registry.register(step);
        }
    }

    registry
}

struct ProbeStep {
    id: StepId,
    name: String,
    command: CommandSpec,
}

impl ProbeStep {
    fn parse(index: usize, entry: &str) -> Option<Self> {
        let id = *IDS.get(index)?;
        let mut words = entry.split_whitespace();
        let program = words.next()?;
        let command = CommandSpec::new(program).args(words);

        Some(Self {
            id,
            name: program.to_owned(),
            command,
        })
    }
}

impl Step for ProbeStep {
    fn id(&self) -> StepId {
        self.id
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(Detection::Installed)
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.command
            .run()
            .map(|_| ())
            .map_err(|error| StepError::run(self.id, error.to_string()))
    }

    fn platforms(&self) -> &'static [Platform] {
        &Platform::ALL
    }
}
