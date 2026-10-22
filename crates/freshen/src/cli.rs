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

use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, Parser};
use freshen_lib::{
    Config, ContainerRuntimeKind, PackageManagerConfig, PreflightCheck, PreflightConfig,
    RunOptions, StepSelection, DEFAULT_JOBS, DEFAULT_MIN_BATTERY_PERCENT,
    DEFAULT_MIN_FREE_DISK_GIB, DEFAULT_SNAPSHOT_RETENTION, DEFAULT_WAIT,
};

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
    /// (colima, orbstack, docker-desktop, rancher-desktop, podman, docker).
    #[arg(long, value_name = "RUNTIME", value_parser = parse_container_runtime)]
    pub container_runtime: Option<ContainerRuntimeKind>,

    /// Only run the named steps (comma-separated, repeatable).
    #[arg(long, value_name = "STEP", value_delimiter = ',', action = ArgAction::Append)]
    pub only: Vec<String>,

    /// Skip the named steps (comma-separated, repeatable).
    #[arg(long, value_name = "STEP", value_delimiter = ',', action = ArgAction::Append)]
    pub skip: Vec<String>,

    /// Assume yes for every confirmation prompt.
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Force-kill applications that do not quit gracefully.
    #[arg(long)]
    pub force_kill: bool,

    /// Run destructive cleanup (cache, old-version and image pruning).
    #[arg(long, alias = "allow-cleanup", alias = "destructive-cleanup")]
    pub cleanup: bool,

    /// Install the macOS system updates instead of only reporting them
    /// (macOS only).
    #[arg(long)]
    pub install_macos_updates: bool,

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
}

impl Cli {
    /// Converts the parsed flags and the configuration file into the
    /// library's platform-independent options.
    ///
    /// A flag that was given on the command line wins over the same value in
    /// the configuration file, and the configuration file wins over the
    /// built-in default.
    #[must_use]
    pub fn run_options_with(&self, config: &Config) -> RunOptions {
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

        RunOptions::new()
            .with_dry_run(self.dry_run)
            .with_assume_yes(self.yes)
            .with_force_kill(self.force_kill)
            .with_cleanup(self.cleanup)
            .with_install_macos_updates(self.install_macos_updates)
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
            .with_step_overrides(config.step_overrides())
            .with_filters(config.item_filters())
            .with_app_lists(config.app_lists())
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
    use super::Cli;
    use clap::Parser;
    use freshen_lib::{
        AppSection, Config, FilterSection, PreflightCheck, PreflightSection, RunSection,
    };
    use std::collections::BTreeMap;

    fn options(cli: &Cli) -> freshen_lib::RunOptions {
        cli.run_options_with(&Config::default())
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
        let options = options(&cli);

        assert!(!cli.list_steps);
        assert!(!cli.check);
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
        assert!(!options.install_macos_updates());
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
    }

    #[test]
    fn parses_every_flag() {
        let cli = parse(&[
            "freshen",
            "--dry-run",
            "--check",
            "--list-steps",
            "--config",
            "/tmp/config.toml",
            "--only",
            "brew",
            "--skip",
            "casks",
            "--yes",
            "--force-kill",
            "--cleanup",
            "--install-macos-updates",
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
        ]);
        let options = options(&cli);

        assert!(cli.list_steps);
        assert!(cli.check);
        assert_eq!(
            cli.config,
            Some(std::path::PathBuf::from("/tmp/config.toml"))
        );
        assert!(options.is_dry_run());
        assert!(options.assume_yes());
        assert!(options.force_kill());
        assert!(options.cleanup_enabled());
        assert!(options.install_macos_updates());
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
        let options = options(&cli);

        assert_eq!(options.selection().only(), ["brew", "uv", "mise"]);
        assert_eq!(options.selection().skip(), ["casks", "docker", "uv"]);
    }

    #[test]
    fn yes_has_a_short_alias() {
        let cli = parse(&["freshen", "-y"]);

        assert!(options(&cli).assume_yes());
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
        assert!(options(&cli)
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
            },
            ..Config::default()
        };

        let options = parse(&["freshen"]).run_options_with(&config);

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

        let options = parse(&["freshen"]).run_options_with(&config);

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
        let options = parse(&["freshen"]).run_options_with(&Config::default());

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
            },
            ..Config::default()
        };

        let options = parse(&[
            "freshen",
            "--skip-network-check=false",
            "--battery-threshold",
            "10",
            "--jobs",
            "3",
            "--snapshot-retention",
            "7",
        ])
        .run_options_with(&config);

        assert!(options.preflight().is_enabled(PreflightCheck::Network));
        assert_eq!(options.preflight().min_battery_percent(), 10);
        assert_eq!(options.jobs(), 3);
        assert_eq!(options.snapshot_retention(), 7);
    }

    #[test]
    fn help_lists_every_platform_independent_flag() {
        let help = Cli::try_parse_from(["freshen", "--help"])
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();

        for flag in [
            "--dry-run",
            "--check",
            "--list-steps",
            "--json",
            "--markdown",
            "--config",
            "--only",
            "--skip",
            "--yes",
            "--force-kill",
            "--cleanup",
            "--install-macos-updates",
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
