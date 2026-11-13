//! Command-line interface for the `freshen` binary.
//!
//! Argument parsing lives in the binary; the meaning of every flag lives in
//! [`freshen_lib::RunOptions`], so macOS, Linux and Windows share one
//! implementation.
//!
//! A flag that was given on the command line wins over the same value in the
//! configuration file, and the configuration file wins over the built-in
//! default. Flags that are not given stay `None` here, so the configuration
//! file can fill them in.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, Parser, Subcommand};
use clap_complete::Shell;
use freshen_lib::{
    Config, ContainerRuntimeKind, OutputLevel, PackageManagerConfig, PreflightCheck,
    PreflightConfig, RunOptions, StepSelection, DEFAULT_JOBS, DEFAULT_MIN_BATTERY_PERCENT,
    DEFAULT_MIN_FREE_DISK_GIB, DEFAULT_SNAPSHOT_RETENTION, DEFAULT_WAIT,
};

/// A problem combining the command line with the configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// `--profile` named a profile that the configuration file does not define.
    UnknownProfile {
        /// Name that was given on the command line.
        name: String,
        /// Profiles that the configuration file defines.
        known: String,
    },
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProfile { name, known } => {
                write!(f, "unknown profile `{name}` (known profiles: {known})")
            }
        }
    }
}

impl std::error::Error for CliError {}

/// Keep a developer machine up to date: package managers, language toolchains
/// and containers, on macOS, Linux and Windows.
#[derive(Debug, Parser)]
#[command(
    name = "freshen",
    version,
    about,
    after_help = "Configuration file (optional):\n  ~/.config/freshen/config.toml (macOS, Linux)\n  %APPDATA%\\freshen\\config.toml (Windows)\n\nExit codes:\n  0    success\n  1    a step failed or timed out (also used for startup errors)\n  2    updates are pending (--check)\n  3    a preflight or package-manager check failed\n  130  the run was interrupted"
)]
pub struct Cli {
    /// Show what would run without changing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Verify the environment (tools found, permissions, config validity) and
    /// print actionable fixes without changing anything.
    #[arg(long)]
    pub doctor: bool,

    /// Print more detail about what freshen is doing, including diagnostic
    /// lines that are hidden by default.
    #[arg(long, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Print less: suppress informational progress lines, keeping only errors.
    #[arg(long, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Report outdated items without changing anything; exit 2 when updates are pending.
    #[arg(long)]
    pub check: bool,

    /// List every known step, whether it was detected and why it would be skipped.
    #[arg(long)]
    pub list_steps: bool,

    /// Print the report as JSON instead of the plain-text summary.
    #[arg(long, conflicts_with = "markdown")]
    pub json: bool,

    /// Print the report as Markdown instead of the plain-text summary.
    #[arg(long, conflicts_with = "json")]
    pub markdown: bool,

    /// Read the configuration from this file instead of the default location.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Use this container runtime instead of detecting the active one
    /// (colima, orbstack, docker-desktop, podman-desktop, rancher-desktop,
    /// podman, docker).
    #[arg(long, value_name = "RUNTIME", value_parser = parse_container_runtime)]
    pub container_runtime: Option<ContainerRuntimeKind>,

    /// Only run the named steps (comma-separated, repeatable).
    #[arg(long, value_name = "STEP", value_delimiter = ',', action = ArgAction::Append)]
    pub only: Vec<String>,

    /// Skip the named steps (comma-separated, repeatable).
    #[arg(long, value_name = "STEP", value_delimiter = ',', action = ArgAction::Append)]
    pub skip: Vec<String>,

    /// Run the steps enabled by the named profile, defined in the
    /// configuration file under `[profiles.<name>]`. The profile's steps
    /// override the top-level `[steps]` section.
    #[arg(long, value_name = "PROFILE")]
    pub profile: Option<String>,

    /// Exclude these Chocolatey packages from the upgrade (repeatable,
    /// comma-separated). Chocolatey only runs on Windows.
    #[arg(long, value_name = "PACKAGE", value_delimiter = ',', action = ArgAction::Append)]
    pub except: Vec<String>,

    /// Assume yes for every confirmation prompt.
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Force-kill applications that do not quit gracefully.
    #[arg(long)]
    pub force_kill: bool,

    /// Run destructive cleanup (cache, old-version and image pruning).
    #[arg(long, alias = "allow-cleanup", alias = "destructive-cleanup")]
    pub cleanup: bool,

    /// Report the reclaimable space from each cleanup step's dry-run listing
    /// without removing anything. Implies a read-only prune: no mutating
    /// cleanup command is ever run.
    #[arg(long)]
    pub prune_report: bool,

    /// Install the macOS system updates instead of only reporting them
    /// (macOS only).
    #[arg(long)]
    pub install_macos_updates: bool,

    /// Install the available firmware updates instead of only reporting them
    /// (Linux only).
    #[arg(long)]
    pub install_firmware_updates: bool,

    /// Install the WSL kernel update with `wsl --update` instead of only
    /// reporting that it is available (Windows only). `wsl --update` shuts
    /// down every running WSL distribution, so the update is opt-in.
    #[arg(long)]
    pub install_wsl_updates: bool,

    /// Sync lazy.nvim plugins with `nvim --headless "+Lazy! sync" +qa`
    /// instead of only reporting that the sync is available. Syncing plugins
    /// can modify the user's neovim config, so the step is opt-in.
    #[arg(long)]
    pub install_nvim_lazy: bool,

    /// Sync packer.nvim plugins with `nvim --headless "+PackerSync" +qa`
    /// instead of only reporting that the sync is available. Syncing plugins
    /// can modify the user's neovim config, so the step is opt-in.
    #[arg(long)]
    pub install_nvim_packer: bool,

    /// Update Mason packages with `nvim --headless "+MasonUpdate" +qa`
    /// instead of only reporting that the update is available. Updating Mason
    /// packages can modify the user's neovim config, so the step is opt-in.
    #[arg(long)]
    pub install_nvim_mason: bool,

    /// Update Treesitter parsers with `nvim --headless "+TSUpdate" +qa`
    /// instead of only reporting that the update is available. Updating
    /// Treesitter parsers can modify the user's neovim config, so the step is
    /// opt-in.
    #[arg(long)]
    pub install_nvim_treesitter: bool,

    /// Run the Linux package-manager steps inside this WSL distribution on
    /// Windows (for example `--wsl-distro Ubuntu`). Implies the steps are
    /// detected through `wsl` and executed as `wsl -d <distro> <command>`.
    #[arg(long)]
    pub wsl_distro: Option<String>,

    /// Restart the services that need a restart after a step, restoring their
    /// previous enabled/running state on error or interrupt (Linux only).
    #[arg(long)]
    pub restart_services: bool,

    /// Restart the desktop applications that were stopped before an upgrade,
    /// reusing the recorded snapshot of what was stopped (Linux only).
    #[arg(long)]
    pub restart_apps: bool,

    /// Replace `lts`/`latest` mise tool requests with a fixed version when
    /// upgrading mise-managed tools (opt-in, off by default).
    #[arg(long)]
    pub bump: bool,

    /// Pull pinned container images too (images referenced by digest or a
    /// fixed non-`latest` tag in a compose file). Without this flag freshen
    /// only pulls images that are not pinned (opt-in, off by default).
    #[arg(long)]
    pub docker_bump: bool,

    /// Prune dangling container images that have not been used for N days
    /// (only runs with `--cleanup`, opt-in, off by default).
    #[arg(long, value_name = "DAYS")]
    pub prune_unused_older_than: Option<usize>,

    /// Reinstall outdated `go install` binaries instead of only reporting them
    /// (opt-in, off by default). Does not change the pinned Go toolchain.
    #[arg(long)]
    pub reinstall_go: bool,

    /// Update AWS CLI v2 through the official installer instead of only
    /// reporting that a newer version is available (opt-in, off by default).
    #[arg(long)]
    pub install_aws_cli_updates: bool,

    /// Keep this many pre-upgrade snapshot files (0 keeps every snapshot,
    /// default 10).
    #[arg(long, value_name = "COUNT")]
    pub snapshot_retention: Option<usize>,

    /// Stop a single step after this many seconds (0 disables the timeout,
    /// default 0).
    #[arg(long, value_name = "SECONDS")]
    pub step_timeout: Option<u64>,

    /// Stop the whole run after this many seconds (0 disables the timeout,
    /// default 0).
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<u64>,

    /// How many read-only checks `--check` may query at the same time
    /// (default 4).
    #[arg(long, value_name = "N")]
    pub jobs: Option<usize>,

    /// Skip the preflight check that the network is reachable.
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        action = ArgAction::Set
    )]
    pub skip_network_check: Option<bool>,

    /// Skip the preflight check that enough disk space is free.
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        action = ArgAction::Set
    )]
    pub skip_disk_check: Option<bool>,

    /// Skip the preflight check that the battery is not too low.
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        action = ArgAction::Set
    )]
    pub skip_battery_check: Option<bool>,

    /// Skip the preflight check for a metered connection or hotspot.
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        action = ArgAction::Set
    )]
    pub skip_metered_check: Option<bool>,

    /// Skip the check for another package manager process.
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        action = ArgAction::Set
    )]
    pub skip_package_manager_check: Option<bool>,

    /// Seconds to wait for another package manager to finish before aborting
    /// (default 30).
    #[arg(long, value_name = "SECONDS")]
    pub package_manager_wait: Option<u64>,

    /// Minimum free disk space in GiB required before updating (default 1).
    #[arg(long, value_name = "GIB")]
    pub min_free_disk_gib: Option<u64>,

    /// Battery percentage below which an update on battery is refused
    /// (default 20).
    #[arg(long, value_name = "PERCENT")]
    pub battery_threshold: Option<u8>,

    /// Subcommand to run. When absent freshen runs the detected steps.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands of freshen.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Show the log of the last freshen run.
    Logs,
    /// List previous runs and what each one changed, from the snapshots.
    History,
    /// Print a shell completion script for zsh, bash, fish or PowerShell.
    Completions {
        /// The shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

impl Cli {
    /// Converts the parsed flags and the configuration file into the
    /// library's platform-independent options.
    ///
    /// A flag that was given on the command line wins over the same value in
    /// the configuration file, and the configuration file wins over the
    /// built-in default.
    ///
    /// # Errors
    ///
    /// Returns [`CliError`] when `--profile` names a profile that the
    /// configuration file does not define.
    pub fn run_options_with(&self, config: &Config) -> Result<RunOptions, CliError> {
        let preflight = PreflightConfig::new()
            .with_check(
                PreflightCheck::Network,
                enabled(self.skip_network_check, config.preflight.network),
            )
            .with_check(
                PreflightCheck::DiskSpace,
                enabled(self.skip_disk_check, config.preflight.disk_space),
            )
            .with_check(
                PreflightCheck::Battery,
                enabled(self.skip_battery_check, config.preflight.battery),
            )
            .with_check(
                PreflightCheck::MeteredConnection,
                enabled(self.skip_metered_check, config.preflight.metered_connection),
            )
            .with_min_free_disk_bytes(
                self.min_free_disk_gib
                    .or(config.preflight.min_free_disk_gib)
                    .unwrap_or(DEFAULT_MIN_FREE_DISK_GIB)
                    .saturating_mul(1024 * 1024 * 1024),
            )
            .with_min_battery_percent(
                self.battery_threshold
                    .or(config.preflight.battery_threshold)
                    .unwrap_or(DEFAULT_MIN_BATTERY_PERCENT),
            );

        let package_manager = PackageManagerConfig::new()
            .with_enabled(enabled(
                self.skip_package_manager_check,
                config.package_manager.enabled,
            ))
            .with_wait(Duration::from_secs(
                self.package_manager_wait
                    .or(config.package_manager.wait_seconds)
                    .unwrap_or(DEFAULT_WAIT.as_secs()),
            ));

        Ok(RunOptions::new()
            .with_dry_run(self.dry_run)
            .with_doctor(self.doctor)
            .with_output_level(match (self.verbose, self.quiet) {
                (true, _) => OutputLevel::Verbose,
                (_, true) => OutputLevel::Quiet,
                _ => OutputLevel::Normal,
            })
            .with_assume_yes(self.yes)
            .with_force_kill(self.force_kill)
            .with_cleanup(self.cleanup)
            .with_prune_report(self.prune_report)
            .with_install_macos_updates(self.install_macos_updates)
            .with_install_firmware_updates(self.install_firmware_updates)
            .with_install_wsl_updates(self.install_wsl_updates)
            .with_install_nvim_lazy(self.install_nvim_lazy)
            .with_install_nvim_packer(self.install_nvim_packer)
            .with_install_nvim_mason(self.install_nvim_mason)
            .with_install_nvim_treesitter(self.install_nvim_treesitter)
            .with_wsl_distro(self.wsl_distro.clone())
            .with_restart_services(self.restart_services)
            .with_restart_apps(self.restart_apps)
            .with_bump(self.bump)
            .with_docker_bump(self.docker_bump)
            .with_prune_unused_older_than(self.prune_unused_older_than)
            .with_go_update(self.reinstall_go)
            .with_install_aws_cli_updates(self.install_aws_cli_updates)
            .with_step_timeout(timeout(self.step_timeout.or(config.timeouts.step_seconds)))
            .with_global_timeout(timeout(self.timeout.or(config.timeouts.total_seconds)))
            .with_snapshot_retention(
                self.snapshot_retention
                    .or(config.run.snapshot_retention)
                    .unwrap_or(DEFAULT_SNAPSHOT_RETENTION),
            )
            .with_jobs(self.jobs.or(config.run.jobs).unwrap_or(DEFAULT_JOBS))
            .with_preflight(preflight)
            .with_package_manager(package_manager)
            .with_selection(
                StepSelection::new()
                    .with_only(self.only.clone())
                    .with_skip(self.skip.clone()),
            )
            .with_step_overrides(self.step_overrides(config)?)
            .with_filters(config.item_filters())
            .with_app_lists(config.app_lists())
            .with_pre_commit_repositories(config.pre_commit_repositories())
            .with_chocolatey_except(self.except.clone())
            .with_hooks(config.hooks.clone())
            .with_pre_run(config.run.pre_run.clone())
            .with_post_run(config.run.post_run.clone()))
    }

    /// The per-step enable and disable flags, with the selected profile's
    /// steps applied on top of the top-level `[steps]` section.
    fn step_overrides(&self, config: &Config) -> Result<BTreeMap<String, bool>, CliError> {
        let mut overrides = config.step_overrides();

        if let Some(profile) = &self.profile {
            let profile_steps = config
                .profile_steps(profile)
                .ok_or_else(|| CliError::unknown_profile(profile, &config.profiles))?;
            for (id, enabled) in profile_steps {
                overrides.insert(id.clone(), *enabled);
            }
        }

        Ok(overrides)
    }
}

impl CliError {
    fn unknown_profile(name: &str, profiles: &BTreeMap<String, BTreeMap<String, bool>>) -> Self {
        let mut known: Vec<&str> = profiles.keys().map(String::as_str).collect();
        known.sort_unstable();
        let known = if known.is_empty() {
            "none".to_owned()
        } else {
            known.join(", ")
        };
        Self::UnknownProfile {
            name: name.to_owned(),
            known,
        }
    }
}

/// Parses `--container-runtime`.
fn parse_container_runtime(value: &str) -> Result<ContainerRuntimeKind, String> {
    value.parse()
}

/// Resolves a check that the command line can skip and the configuration file
/// can disable.
fn enabled(skip: Option<bool>, configured: Option<bool>) -> bool {
    match skip {
        Some(skip) => !skip,
        None => configured.unwrap_or(true),
    }
}

fn timeout(seconds: Option<u64>) -> Option<Duration> {
    seconds
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::{Cli, CliError};
    use clap::Parser;
    use freshen_lib::{
        AppSection, Config, FilterSection, OutputLevel, PreflightCheck, PreflightSection,
        RunSection,
    };
    use std::collections::BTreeMap;

    fn options(cli: &Cli, config: &Config) -> freshen_lib::RunOptions {
        match cli.run_options_with(config) {
            Ok(options) => options,
            Err(error) => panic!("failed to build run options: {error}"),
        }
    }

    fn parse(args: &[&str]) -> Cli {
        match Cli::try_parse_from(args) {
            Ok(cli) => cli,
            Err(error) => panic!("failed to parse {args:?}: {error}"),
        }
    }

    #[test]
    fn flags_default_to_off() {
        let cli = parse(&["freshen"]);
        let options = options(&cli, &Config::default());

        assert!(!cli.list_steps);
        assert!(!cli.check);
        assert!(!cli.doctor);
        assert!(!cli.json);
        assert!(!cli.markdown);
        assert_eq!(cli.config, None);
        assert_eq!(cli.skip_network_check, None);
        assert_eq!(cli.skip_disk_check, None);
        assert_eq!(cli.skip_battery_check, None);
        assert_eq!(cli.skip_metered_check, None);
        assert_eq!(cli.skip_package_manager_check, None);
        assert!(!options.is_dry_run());
        assert!(!options.assume_yes());
        assert!(!options.force_kill());
        assert!(!options.cleanup_enabled());
        assert_eq!(options.output_level(), OutputLevel::Normal);
        assert!(!options.install_macos_updates());
        assert!(!options.install_firmware_updates());
        assert!(!options.install_wsl_updates());
        assert!(!options.restart_services());
        assert!(!options.restart_apps());
        assert!(!options.bump());
        assert!(!options.go_update());
        assert_eq!(options.prune_unused_older_than(), None);
        assert!(!options.install_aws_cli_updates());
        assert_eq!(options.wsl_distro(), None);
        assert_eq!(
            options.snapshot_retention(),
            freshen_lib::DEFAULT_SNAPSHOT_RETENTION
        );
        assert_eq!(options.step_timeout(), None);
        assert_eq!(options.global_timeout(), None);
        assert_eq!(options.jobs(), freshen_lib::DEFAULT_JOBS);
        assert!(options.selection().is_empty());
        for check in freshen_lib::PreflightCheck::ALL {
            assert!(options.preflight().is_enabled(check), "{check}");
        }
        assert_eq!(
            options.preflight().min_free_disk_bytes(),
            freshen_lib::DEFAULT_MIN_FREE_DISK_BYTES
        );
        assert_eq!(
            options.preflight().min_battery_percent(),
            freshen_lib::DEFAULT_MIN_BATTERY_PERCENT
        );
        assert!(options.package_manager().is_enabled());
        assert_eq!(
            options.package_manager().wait(),
            std::time::Duration::from_secs(30)
        );
        assert!(options.chocolatey_except().is_empty());
    }

    #[test]
    fn parses_every_flag() {
        let cli = parse(&[
            "freshen",
            "--dry-run",
            "--check",
            "--list-steps",
            "--doctor",
            "--config",
            "/tmp/config.toml",
            "--only",
            "brew",
            "--skip",
            "casks",
            "--yes",
            "--verbose",
            "--force-kill",
            "--cleanup",
            "--install-macos-updates",
            "--install-firmware-updates",
            "--install-wsl-updates",
            "--wsl-distro",
            "Ubuntu",
            "--restart-services",
            "--restart-apps",
            "--bump",
            "--docker-bump",
            "--prune-unused-older-than",
            "30",
            "--reinstall-go",
            "--install-aws-cli-updates",
            "--snapshot-retention",
            "3",
            "--step-timeout",
            "120",
            "--timeout",
            "3600",
            "--jobs",
            "6",
            "--skip-network-check",
            "--skip-disk-check",
            "--skip-battery-check",
            "--skip-metered-check",
            "--skip-package-manager-check",
            "--package-manager-wait",
            "5",
            "--min-free-disk-gib",
            "5",
            "--battery-threshold",
            "35",
            "--except",
            "git",
            "--except",
            "python",
        ]);
        let options = options(&cli, &Config::default());

        assert!(cli.list_steps);
        assert!(cli.doctor);
        assert!(cli.check);
        assert_eq!(
            cli.config,
            Some(std::path::PathBuf::from("/tmp/config.toml"))
        );
        assert!(options.is_dry_run());
        assert!(options.assume_yes());
        assert!(options.force_kill());
        assert_eq!(options.output_level(), OutputLevel::Verbose);
        assert!(options.cleanup_enabled());
        assert!(options.install_macos_updates());
        assert!(options.install_firmware_updates());
        assert!(options.install_wsl_updates());
        assert_eq!(options.wsl_distro(), Some("Ubuntu"));
        assert!(options.restart_services());
        assert!(options.restart_apps());
        assert!(options.bump());
        assert!(options.docker_bump());
        assert_eq!(options.prune_unused_older_than(), Some(30));
        assert!(options.go_update());
        assert!(options.install_aws_cli_updates());
        assert_eq!(options.snapshot_retention(), 3);
        assert_eq!(
            options.step_timeout(),
            Some(std::time::Duration::from_secs(120))
        );
        assert_eq!(
            options.global_timeout(),
            Some(std::time::Duration::from_secs(3600))
        );
        assert_eq!(options.jobs(), 6);
        assert_eq!(options.selection().only(), ["brew"]);
        assert_eq!(options.selection().skip(), ["casks"]);
        for check in freshen_lib::PreflightCheck::ALL {
            assert!(!options.preflight().is_enabled(check), "{check}");
        }
        assert_eq!(
            options.preflight().min_free_disk_bytes(),
            5 * 1024 * 1024 * 1024
        );
        assert_eq!(options.preflight().min_battery_percent(), 35);
        assert!(!options.package_manager().is_enabled());
        assert_eq!(
            options.package_manager().wait(),
            std::time::Duration::from_secs(5)
        );
        assert_eq!(options.chocolatey_except(), ["git", "python"]);
    }

    #[test]
    fn parses_report_format_flags() {
        assert!(parse(&["freshen", "--json"]).json);
        assert!(parse(&["freshen", "--markdown"]).markdown);
    }

    #[test]
    fn json_and_markdown_are_mutually_exclusive() {
        let error = Cli::try_parse_from(["freshen", "--json", "--markdown"]).err();

        assert!(error.is_some());
    }

    #[test]
    fn verbose_sets_output_level_to_verbose() {
        let cli = parse(&["freshen", "--verbose"]);

        assert!(cli.verbose);
        assert!(!cli.quiet);
        assert_eq!(
            options(&cli, &Config::default()).output_level(),
            OutputLevel::Verbose
        );
    }

    #[test]
    fn quiet_sets_output_level_to_quiet() {
        let cli = parse(&["freshen", "--quiet"]);

        assert!(!cli.verbose);
        assert!(cli.quiet);
        assert_eq!(
            options(&cli, &Config::default()).output_level(),
            OutputLevel::Quiet
        );
    }

    #[test]
    fn verbose_and_quiet_are_mutually_exclusive() {
        let error = Cli::try_parse_from(["freshen", "--verbose", "--quiet"]).err();

        assert!(error.is_some());
    }

    #[test]
    fn only_and_skip_accept_repeated_and_comma_separated_values() {
        let cli = parse(&[
            "freshen",
            "--only",
            "brew,uv",
            "--only",
            "mise",
            "--skip",
            "casks",
            "--skip",
            "docker,uv",
        ]);
        let options = options(&cli, &Config::default());

        assert_eq!(options.selection().only(), ["brew", "uv", "mise"]);
        assert_eq!(options.selection().skip(), ["casks", "docker", "uv"]);
    }

    #[test]
    fn yes_has_a_short_alias() {
        let cli = parse(&["freshen", "-y"]);

        assert!(options(&cli, &Config::default()).assume_yes());
    }

    #[test]
    fn unknown_flags_are_rejected() {
        let error = Cli::try_parse_from(["freshen", "--nope"]).err();

        assert!(error.is_some());
    }

    #[test]
    fn a_skip_flag_can_be_turned_off_again() {
        let cli = parse(&["freshen", "--skip-network-check=false"]);

        assert_eq!(cli.skip_network_check, Some(false));
        assert!(options(&cli, &Config::default())
            .preflight()
            .is_enabled(PreflightCheck::Network));
    }

    #[test]
    fn the_configuration_file_fills_in_what_the_command_line_leaves_out() {
        let config = Config {
            preflight: PreflightSection {
                network: Some(false),
                battery_threshold: Some(35),
                ..PreflightSection::default()
            },
            run: RunSection {
                jobs: Some(8),
                snapshot_retention: Some(2),
                pre_run: None,
                post_run: None,
            },
            ..Config::default()
        };

        let cli = parse(&["freshen"]);
        let options = options(&cli, &config);

        assert!(!options.preflight().is_enabled(PreflightCheck::Network));
        assert!(options.preflight().is_enabled(PreflightCheck::DiskSpace));
        assert_eq!(options.preflight().min_battery_percent(), 35);
        assert_eq!(options.jobs(), 8);
        assert_eq!(options.snapshot_retention(), 2);
    }

    #[test]
    fn the_lists_of_the_configuration_file_reach_the_run_options() {
        let config = Config {
            steps: BTreeMap::from([("brew-casks".to_owned(), false)]),
            include: FilterSection {
                packages: Some(vec!["node".to_owned()]),
                ..FilterSection::default()
            },
            exclude: FilterSection {
                casks: Some(vec!["dropbox".to_owned()]),
                ..FilterSection::default()
            },
            apps: AppSection {
                never_quit: Some(vec!["com.docker".to_owned()]),
                always_quit: Some(vec!["com.spotify".to_owned()]),
            },
            ..Config::default()
        };

        let cli = parse(&["freshen"]);
        let options = options(&cli, &config);

        assert!(!options.is_step_enabled("brew-casks"));
        assert!(options.is_step_enabled("brew"));
        assert!(options.filters().packages().includes("node"));
        assert!(!options.filters().packages().includes("python"));
        assert!(!options.filters().casks().includes("dropbox"));
        assert!(options.filters().casks().includes("firefox"));
        assert!(options
            .app_lists()
            .is_never_quit("com.docker", "Docker.app"));
        assert!(options
            .app_lists()
            .is_always_quit("com.spotify", "Spotify.app"));
    }

    #[test]
    fn without_a_configuration_file_nothing_is_filtered() {
        let cli = parse(&["freshen"]);
        let options = options(&cli, &Config::default());

        assert!(options.step_overrides().is_empty());
        assert!(options.filters().is_empty());
        assert!(options.app_lists().is_empty());
    }

    #[test]
    fn the_command_line_wins_over_the_configuration_file() {
        let config = Config {
            preflight: PreflightSection {
                network: Some(false),
                battery_threshold: Some(35),
                ..PreflightSection::default()
            },
            run: RunSection {
                jobs: Some(8),
                snapshot_retention: Some(2),
                pre_run: None,
                post_run: None,
            },
            ..Config::default()
        };

        let cli = parse(&[
            "freshen",
            "--skip-network-check=false",
            "--battery-threshold",
            "10",
            "--jobs",
            "3",
            "--snapshot-retention",
            "7",
        ]);
        let options = options(&cli, &config);

        assert!(options.preflight().is_enabled(PreflightCheck::Network));
        assert_eq!(options.preflight().min_battery_percent(), 10);
        assert_eq!(options.jobs(), 3);
        assert_eq!(options.snapshot_retention(), 7);
    }

    #[test]
    fn a_profile_selects_its_steps_over_the_top_level_steps() {
        let config = Config {
            steps: BTreeMap::from([("brew-casks".to_owned(), false)]),
            profiles: BTreeMap::from([
                (
                    "work".to_owned(),
                    BTreeMap::from([
                        ("brew-casks".to_owned(), true),
                        ("docker".to_owned(), false),
                    ]),
                ),
                (
                    "minimal".to_owned(),
                    BTreeMap::from([("brew".to_owned(), false)]),
                ),
            ]),
            ..Config::default()
        };

        let cli = parse(&["freshen", "--profile", "work"]);
        let options = options(&cli, &config);

        assert!(options.is_step_enabled("brew-casks"));
        assert!(!options.is_step_enabled("docker"));
        assert!(options.is_step_enabled("brew"));
    }

    #[test]
    fn a_profile_can_disable_a_step_that_the_top_level_section_enabled() {
        let config = Config {
            steps: BTreeMap::from([("docker".to_owned(), true)]),
            profiles: BTreeMap::from([(
                "minimal".to_owned(),
                BTreeMap::from([("docker".to_owned(), false)]),
            )]),
            ..Config::default()
        };

        let cli = parse(&["freshen", "--profile", "minimal"]);
        let options = options(&cli, &config);

        assert!(!options.is_step_enabled("docker"));
    }

    #[test]
    fn an_unknown_profile_is_rejected_with_the_known_profiles() {
        let config = Config {
            profiles: BTreeMap::from([(
                "work".to_owned(),
                BTreeMap::from([("brew".to_owned(), false)]),
            )]),
            ..Config::default()
        };

        let cli = parse(&["freshen", "--profile", "nope"]);
        let error = cli.run_options_with(&config).err();

        let Some(CliError::UnknownProfile { name, known }) = error else {
            panic!("expected an unknown profile error, got {error:?}");
        };
        assert_eq!(name, "nope");
        assert_eq!(known, "work");
    }

    #[test]
    fn an_unknown_profile_is_rejected_even_when_no_profiles_are_defined() {
        let cli = parse(&["freshen", "--profile", "work"]);
        let error = cli.run_options_with(&Config::default()).err();

        let Some(CliError::UnknownProfile { name, known }) = error else {
            panic!("expected an unknown profile error, got {error:?}");
        };
        assert_eq!(name, "work");
        assert_eq!(known, "none");
    }

    #[test]
    fn help_lists_every_platform_independent_flag() {
        let help = Cli::try_parse_from(["freshen", "--help"])
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();

        for flag in [
            "--dry-run",
            "--doctor",
            "--check",
            "--list-steps",
            "--json",
            "--markdown",
            "--config",
            "--verbose",
            "--quiet",
            "--only",
            "--skip",
            "--profile",
            "--yes",
            "--force-kill",
            "--cleanup",
            "--install-macos-updates",
            "--install-firmware-updates",
            "--install-wsl-updates",
            "--wsl-distro",
            "--restart-services",
            "--restart-apps",
            "--bump",
            "--reinstall-go",
            "--install-aws-cli-updates",
            "--prune-unused-older-than",
            "--snapshot-retention",
            "--step-timeout",
            "--timeout",
            "--skip-network-check",
            "--skip-disk-check",
            "--skip-battery-check",
            "--skip-metered-check",
            "--skip-package-manager-check",
            "--package-manager-wait",
            "--min-free-disk-gib",
            "--battery-threshold",
        ] {
            assert!(help.contains(flag), "help does not mention {flag}: {help}");
        }
    }
}
