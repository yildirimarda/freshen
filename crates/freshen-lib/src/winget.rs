//! The winget step: detection and a read-only check for Windows package updates.
//!
//! `winget` is the Windows Package Manager. Detection runs `winget --version`;
//! `check()` runs `winget upgrade` and parses the table of packages with
//! updates available. Neither command changes anything, so the step is safe in
//! `--check` mode. The step is only available on Windows.

use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The winget package manager step.
pub const WINGET: StepId = "winget";

/// winget only runs on Windows.
const WINGET_PLATFORMS: [Platform; 1] = [Platform::Windows];

/// Registers the winget step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(WingetStep)
}

struct WingetStep;

impl WingetStep {
    /// Reports the packages `winget upgrade` lists as outdated.
    fn check(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("winget")
            .args(["upgrade"])
            .neutral_locale()
            .run()
            .map_err(|error| StepError::check_from_process(WINGET, &error))?;
        let items = parse_winget_upgrade(output.stdout());
        Ok(CheckReport::from_items(items))
    }
}

impl Step for WingetStep {
    fn id(&self) -> StepId {
        WINGET
    }

    fn display_name(&self) -> &str {
        "winget"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        if CommandSpec::new("winget").args(["--version"]).run().is_ok() {
            Ok(Detection::Installed)
        } else {
            Ok(Detection::NotInstalled)
        }
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check()
    }

    fn run(&self) -> Result<(), StepError> {
        Err(StepError::run(
            WINGET,
            "winget upgrades are not yet implemented; use --check to report pending updates",
        ))
    }

    fn platforms(&self) -> &'static [Platform] {
        &WINGET_PLATFORMS
    }
}

/// Parses the output of `winget upgrade`.
///
/// Each row of the upgrade table is `Name Id Version Source`, where `Name`
/// may contain spaces. The last three whitespace-separated tokens are
/// `Source`, `Version` and `Id`; everything before `Id` is the package name.
/// The announcement, the column titles and the separator line are skipped.
/// `winget` does not report the currently installed version, so only the
/// available version is recorded.
fn parse_winget_upgrade(output: &str) -> Vec<OutdatedItem> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            // Skip the "Found the following packages available for upgrade:"
            // announcement and the "nothing to upgrade" message.
            if line.starts_with("Found the following packages")
                || line.starts_with("No applicable package upgrades found")
            {
                return None;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() < 4 {
                return None;
            }
            // Skip the column titles: `Name Id Version Source`.
            if tokens == ["Name", "Id", "Version", "Source"] {
                return None;
            }
            // Skip the separator line: runs of dashes separated by spaces.
            if tokens
                .iter()
                .all(|token| token.bytes().all(|byte| byte == b'-'))
            {
                return None;
            }
            let version = tokens.get(tokens.len() - 2).copied().unwrap_or("");
            let id = tokens.get(tokens.len() - 3).copied().unwrap_or("");
            let name = tokens[..tokens.len() - 3].join(" ");
            if name.is_empty() || id.is_empty() || version.is_empty() {
                return None;
            }
            Some(OutdatedItem::new(name).with_latest(version))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_winget_upgrade;

    #[test]
    fn parses_the_upgrade_table() {
        let items = parse_winget_upgrade(
            "Found the following packages available for upgrade:\n\
             Name                Id                          Version        Source\n\
             --------------------------------------------------------------------------------\n\
             7zip                7zip.7zip                   24.08        winget\n\
             Git                 Git.Git                     2.45.2       winget\n\
             Microsoft Visual C++ Redistributable  Microsoft.VC++.Redist  14.42.34512  winget\n",
        );

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name(), "7zip");
        assert_eq!(items[0].current(), None);
        assert_eq!(items[0].latest(), Some("24.08"));
        assert_eq!(items[1].name(), "Git");
        assert_eq!(items[1].latest(), Some("2.45.2"));
        assert_eq!(items[2].name(), "Microsoft Visual C++ Redistributable");
        assert_eq!(items[2].latest(), Some("14.42.34512"));
    }

    #[test]
    fn reports_nothing_when_no_upgrades_are_available() {
        let items = parse_winget_upgrade("No applicable package upgrades found.\n");
        assert!(items.is_empty());
    }

    #[test]
    fn skips_the_header_and_separator_lines() {
        let items = parse_winget_upgrade(
            "Found the following packages available for upgrade:\n\
             Name Id Version Source\n\
             ----------------------\n",
        );
        assert!(items.is_empty());
    }

    #[test]
    fn ignores_short_lines_that_do_not_form_a_row() {
        let items = parse_winget_upgrade("7zip\nwinget\n");
        assert!(items.is_empty());
    }
}
