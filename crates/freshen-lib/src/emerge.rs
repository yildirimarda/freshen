//! The emerge step: `emerge --sync` followed by `emerge -uDN @world`, skipped
//! when emerge is absent.
//!
//! The step syncs the Gentoo portage tree (`emerge --sync`) and then upgrades
//! the world set (`emerge -uDN @world`) whenever `emerge` is available. It is
//! skipped entirely when emerge is absent, so it is safe to register on every
//! machine; on macOS and Windows it is never detected.
//!
//! In `--check` mode the step reports what `emerge -puDN @world` would upgrade
//! without changing anything, so the report comes first and the actual upgrade
//! only happens in run mode.

use crate::executable::command_exists;
use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The emerge package manager step (Gentoo).
pub const EMERGE: StepId = "emerge";

/// emerge only runs on Linux.
const EMERGE_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the emerge step, after the apk step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(EmergeStep)
}

struct EmergeStep;

impl EmergeStep {
    /// Runs `emerge --sync` and then `emerge -uDN @world`, collecting the
    /// first failure so both commands are attempted even when sync fails.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("emerge --sync");
        if let Err(error) = CommandSpec::new("emerge")
            .args(["--sync"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(EMERGE, &error));
        }

        human!("emerge -uDN @world");
        if let Err(error) = CommandSpec::new("emerge")
            .args(["-uDN", "@world"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(EMERGE, &error));
        }
    }
}

impl Step for EmergeStep {
    fn id(&self) -> StepId {
        EMERGE
    }

    fn display_name(&self) -> &str {
        "emerge"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_emerge())
    }

    /// Reports what `emerge -puDN @world` would upgrade without changing
    /// anything.
    fn check(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("emerge")
            .args(["-puDN", "@world"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(EMERGE, &error))?;
        Ok(CheckReport::from_items(parse_emerge_pretend_upgrade(
            output.stdout(),
        )))
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &EMERGE_PLATFORMS
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the `emerge` binary is discoverable on `PATH`.
fn detect_emerge() -> Detection {
    if command_exists("emerge") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses the output of `emerge -puDN @world`.
///
/// Each line that would update a package looks like
/// `[ebuild  U     ] cat/pkg-ver::repo [oldver::repo]`; only the package name,
/// its current version and the version it would be upgraded to are reported.
/// Lines that do not match that shape are ignored.
fn parse_emerge_pretend_upgrade(output: &str) -> Vec<OutdatedItem> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("[ebuild") {
                return None;
            }
            let after_bracket = line.find(']')?;
            // The action column sits between `[ebuild` and `]`; only `U`
            // (update) lines report an outdated package.
            let action = line[7..after_bracket].split_whitespace().next();
            if action != Some("U") {
                return None;
            }
            let rest = line[after_bracket + 1..].trim_start();
            let mut parts = rest.split_whitespace();
            let atom = parts.next()?;
            // atom is `cat/pkg-ver::repo`; drop the repository suffix.
            let (pkg_ver, _repository) = atom.split_once("::").unwrap_or((atom, ""));
            let (name, version) = split_pkg_version(pkg_ver);
            if name.is_empty() {
                return None;
            }
            let mut item = OutdatedItem::new(name);
            if let Some(latest) = version {
                item = item.with_latest(latest);
            }
            // The bracketed token, when present, holds the currently installed
            // version: `[oldver::repo]`.
            if let Some(token) = parts.next() {
                let token = token.trim_start_matches('[').trim_end_matches(']');
                let (version, _repository) = token.split_once("::").unwrap_or((token, ""));
                if !version.is_empty() {
                    item = item.with_current(version);
                }
            }
            Some(item)
        })
        .collect()
}

/// Splits `cat/pkg-ver` into `cat/pkg` and `ver`. The version is the part
/// after the last `-` that follows the final `/` and starts with a digit;
/// otherwise there is no version to report. This keeps hyphens inside a
/// category or package name (for example `sys-apps` or `linux-firmware`) from
/// being mistaken for a version separator.
fn split_pkg_version(atom: &str) -> (&str, Option<&str>) {
    let Some(slash) = atom.rfind('/') else {
        return (atom, None);
    };
    let package = &atom[slash + 1..];
    // The version is after the last `-` whose suffix starts with a digit, so
    // hyphens inside a category or package name are not mistaken for a
    // version separator.
    let dash = package
        .match_indices('-')
        .rev()
        .find(|(i, _)| package[i + 1..].starts_with(char::is_numeric))
        .map(|(i, _)| i);
    let Some(dash) = dash else {
        return (atom, None);
    };
    let version = &package[dash + 1..];
    let name = &atom[..=slash + dash];
    (name, Some(version))
}

#[cfg(test)]
mod tests {
    use super::{parse_emerge_pretend_upgrade, split_pkg_version};

    #[test]
    fn parses_pretend_upgrade_lines() {
        let items = parse_emerge_pretend_upgrade(
            "Calculating dependencies... done!\n\
             [ebuild  U     ] app-editors/vim-9.0.1000::gentoo [9.0.999::gentoo] USE=\"X\"\n\
             [ebuild  N     ] app-misc/hello-2.12.1::gentoo\n\
             [ebuild  R     ] sys-kernel/gentoo-sources-6.1.52::gentoo\n\
             [ebuild  U     ] sys-apps/portage-3.0.52::gentoo [3.0.51::gentoo]\n\
             [ebuild   U    ] dev-lang/rust-1.75.0::gentoo [1.74.0::gentoo]\n\
             All done.\n",
        );

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name(), "app-editors/vim");
        assert_eq!(items[0].current(), Some("9.0.999"));
        assert_eq!(items[0].latest(), Some("9.0.1000"));
        assert_eq!(items[1].name(), "sys-apps/portage");
        assert_eq!(items[1].current(), Some("3.0.51"));
        assert_eq!(items[1].latest(), Some("3.0.52"));
        assert_eq!(items[2].name(), "dev-lang/rust");
        assert_eq!(items[2].current(), Some("1.74.0"));
        assert_eq!(items[2].latest(), Some("1.75.0"));
    }

    #[test]
    fn ignores_non_update_lines() {
        let items = parse_emerge_pretend_upgrade(
            "Calculating dependencies... done!\n\
             [ebuild  N     ] app-misc/hello-2.12.1::gentoo\n\
             These are the packages that would be merged, in order:\n",
        );
        assert!(items.is_empty());
    }

    #[test]
    fn splits_package_version_at_the_last_digit_hyphen() {
        assert_eq!(
            split_pkg_version("app-editors/vim-9.0.1000"),
            ("app-editors/vim", Some("9.0.1000"))
        );
        assert_eq!(
            split_pkg_version("sys-apps/portage"),
            ("sys-apps/portage", None)
        );
        assert_eq!(
            split_pkg_version("virtual/linux-firmware-20240101"),
            ("virtual/linux-firmware", Some("20240101"))
        );
    }
}
