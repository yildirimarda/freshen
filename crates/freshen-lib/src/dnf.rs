//! The dnf step: `dnf upgrade -y`, skipped when dnf is absent, and
//! `dnf autoremove -y` when `--cleanup` is set.
//!
//! The step runs `dnf upgrade -y` whenever `dnf` is available. When dnf is
//! absent but `yum` is available, it runs `yum update -y` instead, so the
//! step is skipped only when neither is installed. It is safe to register
//! on every machine; on macOS and Windows it is never detected.

#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::cleanup;
#[cfg(target_os = "linux")]
use crate::containers::{ContainerRuntime, NativeDockerRuntime};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::executable::command_exists;
use crate::options::RunOptions;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::output::human;
use crate::platform::Platform;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
#[cfg(target_os = "linux")]
use crate::restart_needed;
#[cfg(target_os = "linux")]
use crate::services_needing_restart;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::wsl::inside_command;

/// The dnf package manager step (falls back to `yum` when dnf is absent).
pub const DNF: StepId = "dnf";

/// dnf runs on Linux natively and, on Windows, inside a selected WSL
/// distribution via `wsl -d <distro> dnf ...`.
const DNF_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// Docker packages whose upgrade requires the Docker Engine to be restarted.
#[cfg(target_os = "linux")]
const DOCKER_PACKAGES: [&str; 2] = ["docker.io", "docker-ce"];

/// Registers the dnf step, after the apt step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(DnfStep)
}

struct DnfStep;

/// Which package manager binary the step uses.
#[cfg(target_os = "linux")]
enum DnfBinary {
    Dnf,
    Yum,
}

#[cfg(target_os = "linux")]
impl DnfBinary {
    fn name(&self) -> &'static str {
        match self {
            Self::Dnf => "dnf",
            Self::Yum => "yum",
        }
    }

    /// Arguments for the upgrade command (`dnf upgrade -y` / `yum update -y`).
    fn upgrade_args(&self) -> &'static [&'static str] {
        match self {
            Self::Dnf => &["upgrade", "-y"],
            Self::Yum => &["update", "-y"],
        }
    }
}

/// The binary to run: `dnf` when present, otherwise `yum`.
#[cfg(target_os = "linux")]
fn dnf_binary() -> Option<DnfBinary> {
    if command_exists("dnf") {
        Some(DnfBinary::Dnf)
    } else if command_exists("yum") {
        Some(DnfBinary::Yum)
    } else {
        None
    }
}

impl DnfStep {
    /// Runs `dnf upgrade -y`, or `yum update -y` when dnf is absent.
    /// Runs `dnf upgrade -y` (or `yum update -y` when dnf is absent), or
    /// `dnf upgrade -y` inside the selected WSL distribution on Windows.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn upgrade(&self, options: &RunOptions, failures: &mut Vec<StepError>) {
        #[cfg(target_os = "windows")]
        {
            human!("dnf upgrade -y");
            if let Err(error) = inside_command(
                options,
                CommandSpec::new("dnf")
                    .args(["upgrade", "-y"])
                    .retry_transient_network(NETWORK_RETRIES),
            )
            .run()
            {
                failures.push(StepError::run_from_process(DNF, &error));
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            let Some(binary) = dnf_binary() else {
                return;
            };
            human!("{} {}", binary.name(), binary.upgrade_args().join(" "));
            if let Err(error) = inside_command(
                options,
                CommandSpec::new(binary.name())
                    .args(binary.upgrade_args())
                    .retry_transient_network(NETWORK_RETRIES),
            )
            .run()
            {
                failures.push(StepError::run_from_process(DNF, &error));
            }
        }
    }

    /// After a successful upgrade, records the services that need a restart
    /// (reported by `needs-restarting`) and whether a restart is required.
    #[cfg(target_os = "linux")]
    fn services_needing_restart(&self) {
        let mut needed = false;
        human!("needs-restarting");
        if let Ok(output) = CommandSpec::new(NEEDS_RESTARTING).run() {
            let services = parse_needs_restarting_services(output.stdout());
            needed = !services.is_empty();
            for service in services {
                services_needing_restart::record(service);
            }
        }
        if needed {
            restart_needed::record();
        }
    }
}

impl Step for DnfStep {
    fn id(&self) -> StepId {
        DNF
    }

    fn display_name(&self) -> &str {
        "dnf"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        #[cfg(target_os = "linux")]
        return Ok(detect_dnf());
        #[cfg(target_os = "windows")]
        return Ok(detect_wsl());
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        return Ok(Detection::NotInstalled);
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }
    #[allow(unused_variables)]
    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        #[cfg(target_os = "windows")]
        {
            let Some(_distro) = crate::wsl::selected_distro(options) else {
                human!("dnf: no WSL distribution selected; pass --wsl-distro <distro>");
                return Ok(());
            };
            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);
            if options.cleanup_enabled() {
                if let Err(error) = cleanup::run(
                    DNF,
                    "dnf: autoremove",
                    inside_command(
                        options,
                        CommandSpec::new("dnf")
                            .args(["autoremove", "-y"])
                            .retry_transient_network(NETWORK_RETRIES),
                    ),
                    options,
                ) {
                    failures.push(error);
                }
            }
            finish(failures)
        }

        #[cfg(target_os = "linux")]
        {
            let docker_upgrading = {
                let engine = NativeDockerRuntime::new();
                engine.detect().is_installed()
                    && docker_packages_are_pending()
                    && engine.is_running()
            };

            if docker_upgrading {
                if let Err(error) = NativeDockerRuntime::new().stop() {
                    return Err(StepError::run_from_process(DNF, &error));
                }
            }

            let mut failures = Vec::new();
            self.upgrade(options, &mut failures);

            if docker_upgrading {
                if let Err(error) = NativeDockerRuntime::new().start() {
                    failures.push(StepError::run_from_process(DNF, &error));
                }
            }

            if let Some(binary) = dnf_binary() {
                if let Err(error) = cleanup::run(
                    DNF,
                    "dnf: autoremove",
                    CommandSpec::new(binary.name())
                        .args(["autoremove", "-y"])
                        .retry_transient_network(NETWORK_RETRIES),
                    options,
                ) {
                    failures.push(error);
                }
            }

            if failures.is_empty() {
                self.services_needing_restart();
            }
            finish(failures)
        }

        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            // dnf is not available on macOS; nothing to run.
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &DNF_PLATFORMS
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether `dnf` or `yum` is discoverable on `PATH`.
#[cfg(target_os = "linux")]
fn detect_dnf() -> Detection {
    match dnf_binary() {
        Some(_) => Detection::Installed,
        None => Detection::NotInstalled,
    }
}

/// Whether the `wsl` binary is discoverable on `PATH` (Windows only).
#[cfg(target_os = "windows")]
fn detect_wsl() -> Detection {
    if command_exists("wsl") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(target_os = "linux")]
/// `needs-restarting` (from `dnf-utils`) reports which running services need a
/// restart after a dnf/yum upgrade. It lists one service per line when run
/// without `-r`. When the tool is not installed, or cannot be run, no services
/// are reported.
const NEEDS_RESTARTING: &str = "needs-restarting";

/// Parses the output of `needs-restarting` into the names of services that need
/// a restart. Each non-empty, non-header line is treated as a service name.
#[cfg(target_os = "linux")]
fn parse_needs_restarting_services(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.contains("PID") && !line.contains("Service"))
        .filter(|line| {
            !line
                .chars()
                .all(|c| c == '-' || c == '+' || c == '|' || c.is_whitespace())
        })
        .map(ToOwned::to_owned)
        .collect()
}

/// Whether `docker.io` or `docker-ce` has a pending upgrade, as reported by
/// `dnf list --upgrades`. The package name may carry an architecture suffix
/// (e.g. `docker-ce.x86_64`), so each line is matched by its prefix.
#[cfg(target_os = "linux")]
fn docker_packages_are_pending() -> bool {
    let Ok(output) = CommandSpec::new("dnf")
        .args(["list", "--upgrades"])
        .neutral_locale()
        .run()
    else {
        return false;
    };
    output.stdout().lines().any(|line| {
        let trimmed = line.trim();
        DOCKER_PACKAGES.iter().any(|pkg| trimmed.starts_with(*pkg))
    })
}
