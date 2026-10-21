//! Command-line interface for the `freshen` binary.
//!
//! Argument parsing lives in the binary; the meaning of every flag lives in
//! [`freshen_lib::RunOptions`], so macOS, Linux and Windows share one
//! implementation.

use std::time::Duration;

use clap::{ArgAction, Parser};
use freshen_lib::{
    PackageManagerConfig, PreflightCheck, PreflightConfig, RunOptions, StepSelection,
};

/// Keep a developer machine up to date: package managers, language toolchains
/// and containers, on macOS, Linux and Windows.
#[derive(Debug, Parser)]
#[command(
    name = "freshen",
    version,
    about,
    after_help = "Exit codes:\n  0    success\n  1    a step failed or timed out (also used for startup errors)\n  2    updates are pending (--check)\n  3    a preflight or package-manager check failed\n  130  the run was interrupted"
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

    /// Keep this many pre-upgrade snapshot files (0 keeps every snapshot).
    #[arg(
        long,
        value_name = "COUNT",
        default_value_t = freshen_lib::DEFAULT_SNAPSHOT_RETENTION
    )]
    pub snapshot_retention: usize,

    /// Stop a single step after this many seconds (0 disables the timeout).
    #[arg(long, value_name = "SECONDS", default_value_t = 0)]
    pub step_timeout: u64,

    /// Stop the whole run after this many seconds (0 disables the timeout).
    #[arg(long, value_name = "SECONDS", default_value_t = 0)]
    pub timeout: u64,

    /// Skip the preflight check that the network is reachable.
    #[arg(long)]
    pub skip_network_check: bool,

    /// Skip the preflight check that enough disk space is free.
    #[arg(long)]
    pub skip_disk_check: bool,

    /// Skip the preflight check that the battery is not too low.
    #[arg(long)]
    pub skip_battery_check: bool,

    /// Skip the preflight check for a metered connection or hotspot.
    #[arg(long)]
    pub skip_metered_check: bool,

    /// Skip the check for another package manager process.
    #[arg(long)]
    pub skip_package_manager_check: bool,

    /// Seconds to wait for another package manager to finish before aborting.
    #[arg(long, value_name = "SECONDS", default_value_t = 30)]
    pub package_manager_wait: u64,

    /// Minimum free disk space in GiB required before updating.
    #[arg(long, value_name = "GIB", default_value_t = 1)]
    pub min_free_disk_gib: u64,

    /// Battery percentage below which an update on battery is refused.
    #[arg(long, value_name = "PERCENT", default_value_t = 20)]
    pub battery_threshold: u8,
}

impl Cli {
    /// Converts the parsed flags into the library's platform-independent
    /// options.
    #[must_use]
    pub fn run_options(&self) -> RunOptions {
        let preflight = PreflightConfig::new()
            .with_check(PreflightCheck::Network, !self.skip_network_check)
            .with_check(PreflightCheck::DiskSpace, !self.skip_disk_check)
            .with_check(PreflightCheck::Battery, !self.skip_battery_check)
            .with_check(PreflightCheck::MeteredConnection, !self.skip_metered_check)
            .with_min_free_disk_bytes(self.min_free_disk_gib.saturating_mul(1024 * 1024 * 1024))
            .with_min_battery_percent(self.battery_threshold);

        let package_manager = PackageManagerConfig::new()
            .with_enabled(!self.skip_package_manager_check)
            .with_wait(Duration::from_secs(self.package_manager_wait));

        RunOptions::new()
            .with_dry_run(self.dry_run)
            .with_assume_yes(self.yes)
            .with_force_kill(self.force_kill)
            .with_cleanup(self.cleanup)
            .with_step_timeout(timeout(self.step_timeout))
            .with_global_timeout(timeout(self.timeout))
            .with_snapshot_retention(self.snapshot_retention)
            .with_preflight(preflight)
            .with_package_manager(package_manager)
            .with_selection(
                StepSelection::new()
                    .with_only(self.only.clone())
                    .with_skip(self.skip.clone()),
            )
    }
}

fn timeout(seconds: u64) -> Option<Duration> {
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        match Cli::try_parse_from(args) {
            Ok(cli) => cli,
            Err(error) => panic!("failed to parse {args:?}: {error}"),
        }
    }

    #[test]
    fn flags_default_to_off() {
        let cli = parse(&["freshen"]);
        let options = cli.run_options();

        assert!(!cli.list_steps);
        assert!(!cli.check);
        assert!(!cli.json);
        assert!(!cli.markdown);
        assert!(!cli.skip_network_check);
        assert!(!cli.skip_disk_check);
        assert!(!cli.skip_battery_check);
        assert!(!cli.skip_metered_check);
        assert!(!cli.skip_package_manager_check);
        assert!(!options.is_dry_run());
        assert!(!options.assume_yes());
        assert!(!options.force_kill());
        assert!(!options.cleanup_enabled());
        assert_eq!(
            options.snapshot_retention(),
            freshen_lib::DEFAULT_SNAPSHOT_RETENTION
        );
        assert_eq!(options.step_timeout(), None);
        assert_eq!(options.global_timeout(), None);
        assert!(options.selection().is_empty());
        for check in freshen_lib::PreflightCheck::ALL {
            assert!(options.preflight().is_enabled(check), "{check}");
        }
        assert_eq!(
            options.preflight().min_free_disk_bytes(),
            1024 * 1024 * 1024
        );
        assert_eq!(options.preflight().min_battery_percent(), 20);
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
            "--only",
            "brew",
            "--skip",
            "casks",
            "--yes",
            "--force-kill",
            "--cleanup",
            "--snapshot-retention",
            "3",
            "--step-timeout",
            "120",
            "--timeout",
            "3600",
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
        let options = cli.run_options();

        assert!(cli.list_steps);
        assert!(cli.check);
        assert!(options.is_dry_run());
        assert!(options.assume_yes());
        assert!(options.force_kill());
        assert!(options.cleanup_enabled());
        assert_eq!(options.snapshot_retention(), 3);
        assert_eq!(
            options.step_timeout(),
            Some(std::time::Duration::from_secs(120))
        );
        assert_eq!(
            options.global_timeout(),
            Some(std::time::Duration::from_secs(3600))
        );
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
        let options = cli.run_options();

        assert_eq!(options.selection().only(), ["brew", "uv", "mise"]);
        assert_eq!(options.selection().skip(), ["casks", "docker", "uv"]);
    }

    #[test]
    fn yes_has_a_short_alias() {
        let cli = parse(&["freshen", "-y"]);

        assert!(cli.run_options().assume_yes());
    }

    #[test]
    fn unknown_flags_are_rejected() {
        let error = Cli::try_parse_from(["freshen", "--nope"]).err();

        assert!(error.is_some());
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
            "--only",
            "--skip",
            "--yes",
            "--force-kill",
            "--cleanup",
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
