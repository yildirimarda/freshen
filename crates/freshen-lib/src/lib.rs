//! Core logic for freshen, a cross-platform tool that keeps a developer
//! machine up to date (package managers, language toolchains, containers).
//!
//! Every step of a run lives here; the `freshen` binary crate is a thin CLI
//! wrapper around this library. One module per step.

pub mod apps;
pub mod appstore;
pub mod bundle_cleanup_packages;
pub mod check;
pub mod cleanup;
pub mod config;
pub mod containers;
pub(crate) mod deadline;
pub mod doctor_warnings;
mod executable;
pub mod exit;
pub mod homebrew;
pub mod left_running_apps;
pub mod listing;
pub mod locale;
pub mod lock;
pub mod macos_updates;
mod network;
pub mod options;
pub mod output;
pub mod package_manager;
pub mod platform;
pub mod policy;
pub mod preflight;
pub mod process;
pub mod registry;
pub mod reopened_hidden;
pub mod report;
pub mod restarted_services;
pub mod restored_windows;
pub mod runner;
pub mod signals;
pub mod skipped_items;
pub mod snapshot;
pub mod step;
pub mod system;
pub mod tooling;
pub mod warnings;
pub mod xcode;

pub use apps::{
    automation_denied_message, find_running_apps, is_automation_denied, is_menu_bar_app_from_info,
    login_item_helpers_in, quit_apps, quit_choice, reopen_apps, running_updater,
    self_updater_from_bundle, wait_for_self_update, AppKind, AppScan, AppSystem, MacAppSystem,
    QuitChoice, QuitOutcome, QuitPolicy, Relaunch, Reopen, RunningApp, SelfUpdater, SessionState,
    UpdateState, WindowState,
};
pub use appstore::{MasClient, MasOutput, MAS};
pub use check::{CheckResult, CheckStatus, CheckSummary};
pub use cleanup::{CleanupReport, ENABLE_FLAG as CLEANUP_FLAG};
pub use config::{
    AppSection, Config, ConfigEnvironment, ConfigError, FilterSection, PackageManagerSection,
    PreflightSection, RunSection, TimeoutSection, CONFIG_DIR_NAME, CONFIG_FILE_NAME,
};
pub use containers::{
    detect_active_runtime, detect_compose_command, parse_compose_projects,
    parse_standalone_containers, ActiveContainerRuntime, ColimaRuntime, ComposeCommand,
    ComposeCommandProbe, ComposeProject, ContainerRuntime, ContainerRuntimeKind,
    ContainerRuntimeProbe, ContainerRuntimeSource, DockerDesktopRuntime, DockerEngineProbe,
    HostComposeCommandProbe, HostContainerRuntimeProbe, HostDockerEngineProbe, NativeDockerRuntime,
    RancherDesktopRuntime, StandaloneContainer,
};
pub use listing::{list_steps, DetectionState, StepList, StepListing};
pub use lock::{LockError, RunLock};
pub use macos_updates::{
    parse_software_update_list, pending_reboot, SoftwareUpdateClient, SoftwareUpdateOutput,
    MACOS_UPDATES,
};
pub use options::{PlanError, RunOptions, SelectionFlag, StepSelection, DEFAULT_JOBS};
pub use output::{route_to_stderr, route_to_stdout};
pub use package_manager::{
    wait_for_package_managers, HostPackageManagerProbe, PackageManagerConfig, PackageManagerProbe,
    PackageManagerProcess, WaitOutcome, DEFAULT_WAIT,
};
pub use platform::Platform;
pub use policy::{AppLists, ItemFilters, NameFilter, ALWAYS_QUIT_ENV, NEVER_QUIT_ENV};
pub use preflight::{
    BatteryStatus, HostPreflightProbe, PreflightCheck, PreflightConfig, PreflightIssue,
    PreflightProbe, PreflightReport, DEFAULT_MIN_BATTERY_PERCENT, DEFAULT_MIN_FREE_DISK_BYTES,
    DEFAULT_MIN_FREE_DISK_GIB,
};
pub use process::{
    Backoff, CommandSpec, NetworkFailure, OutputStream, ProcessError, ProcessOutput,
    NETWORK_RETRIES,
};
pub use registry::{Registry, RegistryError};
pub use runner::{RunSummary, Runner, StepResult, StepStatus, VersionChange};
pub use signals::{install as install_interrupt_handler, requested as interrupted, SignalError};
pub use snapshot::{
    RollbackCommand, SnapshotCommand, SnapshotRecorder, SnapshotSection, DEFAULT_SNAPSHOT_RETENTION,
};
pub use step::{
    CheckReport, Detection, Operation, OutdatedItem, SkipReason, Step, StepError, StepId,
};
pub use system::{
    supported_terminals, terminal_is_interactive, Arch, Distro, Environment, HostProbe, OsInfo,
    Privilege, ProcessInfo, SystemInfo, SystemProbe, Terminal, TerminalApp,
};
pub use warnings::supported_session_managers;

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
