//! Test-only probe steps used by the stub harness.
//!
//! This module is compiled only with the `test-harness` feature. It reads
//! `FRESHEN_TEST_STEPS` (steps separated by `;`, each a program followed by
//! arguments) and registers one step per entry, in order. The steps always
//! report themselves as installed and run their command during `run()`, which
//! lets an integration test assert the exact call order against stubs on
//! `PATH`.

use freshen_lib::{
    BatteryStatus, CheckReport, CommandSpec, Detection, PackageManagerProbe, PackageManagerProcess,
    Platform, PreflightProbe, Registry, Step, StepError, StepId,
};

const ENV_VAR: &str = "FRESHEN_TEST_STEPS";
const PREFLIGHT_ENV_VAR: &str = "FRESHEN_TEST_PREFLIGHT";
const PACKAGE_MANAGER_ENV_VAR: &str = "FRESHEN_TEST_PACKAGE_MANAGERS";

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

/// A preflight probe built from `FRESHEN_TEST_PREFLIGHT`, when it is set.
///
/// The value is a semicolon-separated list of `key=value` facts:
/// `network=down`, `disk=<bytes>`, `battery=<percent>:<charging|discharging>`
/// and `metered=<yes|no>`. Unknown keys are ignored.
pub fn preflight_probe() -> Option<ScriptedPreflightProbe> {
    let value = std::env::var(PREFLIGHT_ENV_VAR).ok()?;
    let mut probe = ScriptedPreflightProbe {
        network: true,
        disk: None,
        battery: None,
        metered: None,
    };

    for entry in value.split(';') {
        let Some((key, value)) = entry.split_once('=') else {
            continue;
        };
        match key.trim() {
            "network" => probe.network = !is_false(value),
            "disk" => probe.disk = value.trim().parse().ok(),
            "battery" => {
                let (percent, state) = value.split_once(':').unwrap_or((value, "discharging"));
                if let Ok(percent) = percent.trim().parse() {
                    probe.battery = Some(BatteryStatus::new(
                        percent,
                        state.trim().eq_ignore_ascii_case("charging"),
                    ));
                }
            }
            "metered" => {
                probe.metered = if is_true(value) {
                    Some(true)
                } else if is_false(value) {
                    Some(false)
                } else {
                    None
                };
            }
            _ => {}
        }
    }

    Some(probe)
}

fn is_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "yes" | "true" | "up" | "metered"
    )
}

fn is_false(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "no" | "false" | "down" | "unreachable" | "unmetered"
    )
}

/// Preflight facts scripted by an integration test.
pub struct ScriptedPreflightProbe {
    network: bool,
    disk: Option<u64>,
    battery: Option<BatteryStatus>,
    metered: Option<bool>,
}

impl PreflightProbe for ScriptedPreflightProbe {
    fn network_reachable(&self) -> bool {
        self.network
    }

    fn free_disk_bytes(&self) -> Option<u64> {
        self.disk
    }

    fn battery(&self) -> Option<BatteryStatus> {
        self.battery
    }

    fn metered_connection(&self) -> Option<bool> {
        self.metered
    }
}

/// A package manager probe built from `FRESHEN_TEST_PACKAGE_MANAGERS`.
///
/// The value is a semicolon-separated list of `name:pid` entries, for example
/// `brew:1234;apt:5678`. An empty value means no package manager is running.
pub fn package_manager_probe() -> Option<ScriptedPackageManagerProbe> {
    let value = std::env::var(PACKAGE_MANAGER_ENV_VAR).ok()?;
    let processes = value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (name, pid) = entry.split_once(':').unwrap_or((entry, "0"));
            PackageManagerProcess::new(name.trim(), pid.trim().parse().unwrap_or_default())
        })
        .collect();

    Some(ScriptedPackageManagerProbe { processes })
}

/// Package manager processes scripted by an integration test.
pub struct ScriptedPackageManagerProbe {
    processes: Vec<PackageManagerProcess>,
}

impl PackageManagerProbe for ScriptedPackageManagerProbe {
    fn running_package_managers(&self) -> Vec<PackageManagerProcess> {
        self.processes.clone()
    }
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
            .map_err(|error| StepError::run_from_process(self.id, &error))
    }

    fn platforms(&self) -> &'static [Platform] {
        &Platform::ALL
    }
}
