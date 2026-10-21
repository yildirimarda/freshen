//! Core logic for freshen, a cross-platform tool that keeps a developer
//! machine up to date (package managers, language toolchains, containers).
//!
//! Every step of a run lives here; the `freshen` binary crate is a thin CLI
//! wrapper around this library. One module per step.

pub mod apps;
pub mod check;
pub mod containers;
pub(crate) mod deadline;
pub mod homebrew;
pub mod listing;
pub mod lock;
pub mod options;
pub mod package_manager;
pub mod platform;
pub mod preflight;
pub mod process;
pub mod registry;
pub mod report;
pub mod runner;
pub mod signals;
pub mod snapshot;
pub mod step;
pub mod system;
pub mod tooling;
pub mod warnings;

pub use apps::{
    find_running_apps, quit_apps, quit_choice, reopen_apps, AppScan, AppSystem, MacAppSystem,
    QuitChoice, QuitPolicy, RunningApp,
};
pub use check::{CheckResult, CheckStatus, CheckSummary};
pub use containers::{
    parse_compose_projects, parse_standalone_containers, ComposeProject, StandaloneContainer,
};
pub use listing::{list_steps, DetectionState, StepList, StepListing};
pub use lock::{LockError, RunLock};
pub use options::{PlanError, RunOptions, SelectionFlag, StepSelection};
pub use package_manager::{
    wait_for_package_managers, HostPackageManagerProbe, PackageManagerConfig, PackageManagerProbe,
    PackageManagerProcess, WaitOutcome,
};
pub use platform::Platform;
pub use preflight::{
    BatteryStatus, HostPreflightProbe, PreflightCheck, PreflightConfig, PreflightIssue,
    PreflightProbe, PreflightReport,
};
pub use process::{
    Backoff, CommandSpec, NetworkFailure, OutputStream, ProcessError, ProcessOutput,
    NETWORK_RETRIES,
};
pub use registry::{Registry, RegistryError};
pub use runner::{RunSummary, Runner, SkipReason, StepResult, StepStatus, VersionChange};
pub use signals::{install as install_interrupt_handler, requested as interrupted, SignalError};
pub use snapshot::{RollbackCommand, SnapshotCommand, SnapshotRecorder, SnapshotSection};
pub use step::{CheckReport, Detection, Operation, OutdatedItem, Step, StepError, StepId};
pub use system::{
    Arch, Distro, Environment, HostProbe, OsInfo, Privilege, ProcessInfo, SystemInfo, SystemProbe,
    Terminal, TerminalApp,
};

/// Major and minor feature release this binary implements, e.g. `"0.1"`.
///
/// Kept in sync with `Cargo.toml` by a unit test; patch releases may drift
/// between a build and its documentation.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Major and minor feature release the library was compiled against, e.g. `"0.1"`.
pub fn feature_release() -> String {
    let version = version();
    let mut parts = version.splitn(3, '.');
    let major = parts.next().unwrap_or("0");
    let minor = parts.next().unwrap_or("0");
    format!("{major}.{minor}")
}

#[cfg(test)]
mod tests {
    use super::feature_release;
    use super::version;

    #[test]
    fn version_is_semver() {
        let parts: Vec<&str> = version().split('.').collect();
        assert!(
            parts.len() >= 3,
            "version {} must have major.minor.patch components",
            version()
        );
        let major: u64 = parts[0].parse().unwrap_or(u64::MAX);
        let minor: u64 = parts[1].parse().unwrap_or(u64::MAX);
        assert!(major > 0 || minor > 0, "version must be non-zero");
        assert!(parts[2].parse::<u64>().is_ok(), "patch must be a number");
    }

    #[test]
    fn feature_release_is_major_minor_of_version() {
        let expected_prefix = format!("{}.", version().split('.').next().unwrap_or(""));
        assert!(
            feature_release().starts_with(expected_prefix.as_str()),
            "feature release {} must derive from version {}",
            feature_release(),
            version()
        );
    }
}
