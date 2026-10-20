//! Command-line interface for the `freshen` binary.
//!
//! Argument parsing lives in the binary; the meaning of every flag lives in
//! [`freshen_lib::RunOptions`], so macOS, Linux and Windows share one
//! implementation.

use clap::{ArgAction, Parser};
use freshen_lib::{RunOptions, StepSelection};

/// Keep a developer machine up to date: package managers, language toolchains
/// and containers, on macOS, Linux and Windows.
#[derive(Debug, Parser)]
#[command(name = "freshen", version, about)]
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
}

impl Cli {
    /// Converts the parsed flags into the library's platform-independent
    /// options.
    #[must_use]
    pub fn run_options(&self) -> RunOptions {
        RunOptions::new()
            .with_dry_run(self.dry_run)
            .with_assume_yes(self.yes)
            .with_force_kill(self.force_kill)
            .with_selection(
                StepSelection::new()
                    .with_only(self.only.clone())
                    .with_skip(self.skip.clone()),
            )
    }
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
        assert!(!options.is_dry_run());
        assert!(!options.assume_yes());
        assert!(!options.force_kill());
        assert!(options.selection().is_empty());
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
        ]);
        let options = cli.run_options();

        assert!(cli.list_steps);
        assert!(cli.check);
        assert!(options.is_dry_run());
        assert!(options.assume_yes());
        assert!(options.force_kill());
        assert_eq!(options.selection().only(), ["brew"]);
        assert_eq!(options.selection().skip(), ["casks"]);
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
            "--only",
            "--skip",
            "--yes",
            "--force-kill",
        ] {
            assert!(help.contains(flag), "help does not mention {flag}: {help}");
        }
    }
}
