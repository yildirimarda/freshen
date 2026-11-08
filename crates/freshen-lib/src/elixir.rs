//! The Elixir step: `mix local.hex --force` and `mix local.rebar --force`,
//! skipped when `mix` is absent.
//!
//! The step runs both `mix local.hex --force` and `mix local.rebar --force`
//! whenever `mix` (the Elixir build tool that ships with Elixir) is available.
//! It is skipped entirely when `mix` is absent, so it is safe to register on
//! every machine. mix manages user-level Hex and rebar installations, so it
//! must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The Elixir Hex and rebar step.
pub const ELIXIR: StepId = "elixir";

/// `mix` runs on every platform freshen supports.
const ELIXIR_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the Elixir step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(ElixirStep)
}

struct ElixirStep;

impl ElixirStep {
    /// Runs `mix local.hex --force` then `mix local.rebar --force`.
    fn update_tooling(&self, failures: &mut Vec<StepError>) {
        human!("elixir: local hex");
        if let Err(error) = CommandSpec::new("mix")
            .args(["local", "hex", "--force"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ELIXIR, &error));
        }
        human!("elixir: local rebar");
        if let Err(error) = CommandSpec::new("mix")
            .args(["local", "rebar", "--force"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ELIXIR, &error));
        }
    }
}

impl Step for ElixirStep {
    fn id(&self) -> StepId {
        ELIXIR
    }

    fn display_name(&self) -> &str {
        "Elixir"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_mix())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        // mix has no read-only "what is outdated" query, so report up to date.
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.update_tooling(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &ELIXIR_PLATFORMS
    }

    fn is_user_level(&self) -> bool {
        true
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether `mix` is discoverable on `PATH`.
fn detect_mix() -> Detection {
    if crate::executable::command_exists("mix") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    use crate::step::Detection;

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }
}
