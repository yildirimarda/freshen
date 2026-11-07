//! The Go step: binaries installed with `go install`, checked with
//! `go list -m -u`.
//!
//! The step reports a binary as outdated when `go list -m -u` lists a newer
//! module version than the one in the build list. Running `go list -m -u`
//! requires a module context (a `go.mod` in the current directory), which is
//! where the binaries installed with `go install` are tracked.
//!
//! Reinstalling is opt-in: `--reinstall-go` runs `go install <module>@<latest>`
//! for each outdated binary. Without the flag the step only reports outdated
//! binaries and never touches the pinned Go toolchain. The step is skipped
//! entirely when `go` is absent, so it is safe to register on every machine.
//! Go installs user-level binaries, so it must never run as root.

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The Go binaries step.
pub const GO: StepId = "go";

/// Go runs on every platform freshen supports.
const GO_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the Go step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(GoStep)
}

struct GoStep;

impl GoStep {
    /// Reports the modules `go list -m -u` lists as outdated.
    fn check_modules(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("go")
            .args(["list", "-m", "-u"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(GO, &error))?;
        Ok(parse_go_list(output.stdout()))
    }

    /// Upgrades every outdated module to its latest version.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        let report = match self.check_modules() {
            Ok(report) => report,
            Err(error) => {
                failures.push(error);
                return;
            }
        };

        for item in report.items() {
            let Some(latest) = item.latest() else {
                continue;
            };
            human!("go: install {}@{}", item.name(), latest);
            if let Err(error) = CommandSpec::new("go")
                .args(["install"])
                .arg(format!("{}@{}", item.name(), latest))
                .retry_transient_network(NETWORK_RETRIES)
                .run()
            {
                failures.push(StepError::run_from_process(GO, &error));
            }
        }
    }

    /// Whether the opt-in flag that permits reinstalling outdated binaries is
    /// set.
    fn update_enabled(options: &RunOptions) -> bool {
        options.go_update()
    }
}

impl Step for GoStep {
    fn id(&self) -> StepId {
        GO
    }

    fn display_name(&self) -> &str {
        "Go"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_go())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_modules()
    }

    /// Reinstalls outdated binaries only when `--reinstall-go` is given.
    /// Without the flag the step reports what is outdated and changes nothing,
    /// so the pinned Go toolchain is never touched.
    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if !Self::update_enabled(options) {
            human!("go: outdated binaries available; nothing reinstalled (use --reinstall-go)");
            return Ok(());
        }
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &GO_PLATFORMS
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

/// Whether the `go` binary is discoverable on `PATH`.
fn detect_go() -> Detection {
    if crate::executable::command_exists("go") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses the modules `go list -m -u` reports as outdated.
///
/// Each line is `module version` or `module version [latest]`. Only modules
/// with a bracketed latest version have an update available.
fn parse_go_list(stdout: &str) -> CheckReport {
    let mut items = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        let Some(current) = fields.next() else {
            continue;
        };
        let latest = fields
            .next()
            .and_then(|candidate| candidate.strip_prefix('['))
            .and_then(|candidate| candidate.strip_suffix(']'))
            .filter(|version| !version.is_empty());
        if let Some(latest) = latest {
            items.push(
                OutdatedItem::new(name)
                    .with_current(current)
                    .with_latest(latest),
            );
        }
    }
    CheckReport::from_items(items)
}

#[cfg(test)]
mod tests {
    use crate::step::Detection;

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }

    #[test]
    fn parse_go_list_reports_only_modules_with_an_update() {
        let stdout = "example.com/old v1.0.0 [v1.1.0]\nexample.com/current v2.0.0\n";
        let report = super::parse_go_list(stdout);

        assert!(!report.is_up_to_date());
        assert_eq!(report.pending(), 1);
        let item = &report.items()[0];
        assert_eq!(item.name(), "example.com/old");
        assert_eq!(item.current(), Some("v1.0.0"));
        assert_eq!(item.latest(), Some("v1.1.0"));
    }

    #[test]
    fn parse_go_list_ignores_up_to_date_modules() {
        let stdout = "example.com/current v2.0.0\n";
        let report = super::parse_go_list(stdout);

        assert!(report.is_up_to_date());
        assert_eq!(report.pending(), 0);
    }

    #[test]
    fn parse_go_list_skips_blank_and_malformed_lines() {
        let stdout = "\nexample.com/old v1.0.0 [v1.1.0]\n  \n";
        let report = super::parse_go_list(stdout);

        assert_eq!(report.pending(), 1);
    }
}
