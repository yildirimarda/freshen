//! Core logic for freshen, a cross-platform tool that keeps a developer
//! machine up to date (package managers, language toolchains, containers).
//!
//! Every step of a run lives here; the `freshen` binary crate is a thin CLI
//! wrapper around this library. One module per step.

pub mod antidote;
pub mod apk;
pub mod apps;
pub mod appstore;
pub mod apt;
pub mod asdf;
pub mod aur;
pub mod aws;
pub mod az;
pub mod bootc;
pub mod bun;
pub mod bundle_cleanup_packages;
pub mod cargo_update;
pub mod check;
pub mod chocolatey;
pub mod cleanup;
pub mod cmd;
pub mod composer;
pub mod conda;
pub mod config;
pub mod containers;
pub mod cursor;
pub mod custom_step;
pub(crate) mod deadline;
#[cfg(target_os = "linux")]
pub mod desktop_apps;
pub mod dnf;
pub mod docker_df;
pub mod doctor;
pub mod doctor_warnings;
pub mod dotnet_tools;
pub mod elevation;
pub mod elixir;
pub mod emerge;
mod executable;
pub mod exit;
pub mod firmware;
pub mod fisher;
pub mod flatpak;
pub mod fnm;
pub mod gcloud;
pub mod gem;
pub mod gh;
pub mod git_lfs;
pub mod go;
pub mod helm;
pub mod homebrew;
pub mod jenv;
pub mod kubectl;
pub mod left_running_apps;
pub mod listing;
pub mod locale;
pub mod lock;
pub mod logging;
pub mod macos_updates;
pub mod mamba;
pub mod mason;
pub mod microsoft_store;
mod network;
pub mod nix;
pub mod notifications;
pub mod nvim;
pub mod nvm;
pub mod omz;
pub mod options;
pub mod output;
pub mod package_manager;
pub mod package_processes;
pub mod packer;
pub mod pacman;
pub mod pacman_findings;
pub mod pip;
pub mod pipx;
pub mod platform;
pub mod pnpm;
pub mod policy;
pub mod pre_commit;
pub mod preflight;
pub mod process;
pub mod progress;
pub mod pyenv;
pub mod rbenv;
pub mod registry;
pub mod reopened_hidden;
pub mod report;
pub mod restart_needed;
pub mod restarted_services;
pub mod restored_windows;
pub mod rpm_ostree;
pub mod runner;
pub mod rustup;
pub mod scoop;
pub mod sdkman;
pub mod self_update;
pub mod service_restart;
pub mod services_needing_restart;
pub mod shell;
pub mod signals;
pub mod skipped_items;
pub mod snap;
pub mod snapshot;
pub mod step;
pub mod sudo;
pub mod system;
pub mod termux;
pub mod tfenv;
pub mod tooling;
pub mod tpm;
pub mod treesitter;
pub mod trust;
pub mod volta;
pub mod vscode;
pub mod vscodium;
pub mod warnings;
pub mod windows_duplicates;
pub mod windows_update;
pub mod winget;
pub mod wsl;
pub mod xcode;
pub mod yarn;
pub mod zinit;
pub mod zypper;

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
    AppSection, Config, ConfigEnvironment, ConfigError, FilterSection, HookSection, Hooks,
    PackageManagerSection, PreflightSection, RunSection, TimeoutSection, CONFIG_DIR_NAME,
    CONFIG_FILE_NAME,
};
pub use containers::{
    detect_active_runtime, detect_compose_command, parse_compose_projects,
    parse_standalone_containers, ActiveContainerRuntime, ColimaRuntime, ComposeCommand,
    ComposeCommandProbe, ComposeProject, ContainerRuntime, ContainerRuntimeKind,
    ContainerRuntimeProbe, ContainerRuntimeSource, DockerDesktopRuntime, DockerEngineProbe,
    HostComposeCommandProbe, HostContainerRuntimeProbe, HostDockerEngineProbe, K3dRuntime,
    KindRuntime, MinikubeRuntime, NativeDockerRuntime, RancherDesktopRuntime, StandaloneContainer,
};
#[cfg(windows)]
pub use containers::{DockerDesktopWindowsRuntime, PodmanDesktopWindowsRuntime};
pub use custom_step::{CustomStepConfig, CustomStepError};
#[cfg(windows)]
pub use elevation::windows::HostElevationProbe;
pub use elevation::{
    decide, ElevationDecision, ElevationOutcome, ElevationProbe, ElevationState, NoElevationProbe,
};
pub use listing::{list_steps, DetectionState, StepList, StepListing};
pub use lock::{LockError, RunLock};
pub use macos_updates::{
    parse_software_update_list, pending_reboot, SoftwareUpdateClient, SoftwareUpdateOutput,
    MACOS_UPDATES,
};
pub use options::{PlanError, RunOptions, SelectionFlag, StepSelection, DEFAULT_JOBS};
pub use output::{route_to_stderr, route_to_stdout, set_output_level, OutputLevel};
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
    list_snapshots, snapshot_directory, RollbackCommand, SnapshotCommand, SnapshotEntryInfo,
    SnapshotRecord, SnapshotRecorder, SnapshotSection, SnapshotSectionInfo,
    DEFAULT_SNAPSHOT_RETENTION,
};
pub use step::{
    CheckReport, Detection, Operation, OutdatedItem, SkipReason, Step, StepError, StepId,
};
pub use system::{
    supported_terminals, terminal_is_interactive, Arch, Distro, Environment, HostProbe, OsInfo,
    Privilege, ProcessInfo, SystemInfo, SystemProbe, Terminal, TerminalApp,
};
pub use trust::{TrustStore, TRUSTED_FILE_NAME};
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
