//! The optional TOML configuration file.
//!
//! freshen reads a TOML file from the location that is conventional for the
//! operating system it runs on:
//!
//! | Platform | Location |
//! |---|---|
//! | macOS, Linux | `$XDG_CONFIG_HOME/freshen/config.toml`, else `~/.config/freshen/config.toml` |
//! | Windows | `%APPDATA%\freshen\config.toml` |
//!
//! The file is optional: when it is missing freshen runs with its built-in
//! defaults, and when it is present every key in it is optional too. Unknown
//! keys and out-of-range values are refused with an error that names the file
//! and points at the offending line, so a typo is found without a run.
//!
//! The schema is documented in `docs/config.md`; the section structs below are
//! the same schema in code.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::platform::Platform;
use crate::policy::{AppLists, ItemFilters, NameFilter};

/// Name of the directory that holds the configuration file.
pub const CONFIG_DIR_NAME: &str = "freshen";

/// Name of the configuration file inside that directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// A problem that prevented the configuration from being used.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("could not read the configuration file {path}: {source}")]
    Read {
        /// Path of the configuration file.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// The configuration file does not exist.
    #[error("the configuration file {path} does not exist")]
    Missing {
        /// Path of the configuration file.
        path: PathBuf,
    },

    /// The configuration file is not valid freshen configuration.
    ///
    /// The position is the line and column of the offending value, counted
    /// from 1, so the message points at the key that has to be fixed.
    #[error("{path}:{line}:{column}: {message}")]
    Invalid {
        /// Path of the configuration file.
        path: PathBuf,
        /// Line of the offending value.
        line: usize,
        /// Column of the offending value.
        column: usize,
        /// What is wrong with the value.
        message: String,
    },
}

/// The whole configuration file.
///
/// Every section and every key is optional: a file that sets a single value
/// is valid, and a file that does not exist is not an error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The checks that run before an update starts.
    #[serde(default)]
    pub preflight: PreflightSection,

    /// The check for another package manager that is running.
    #[serde(default)]
    pub package_manager: PackageManagerSection,

    /// The timeouts that stop a step or the whole run.
    #[serde(default)]
    pub timeouts: TimeoutSection,

    /// How the run itself behaves.
    #[serde(default)]
    pub run: RunSection,

    /// Which steps may run.
    #[serde(default)]
    pub steps: BTreeMap<String, bool>,

    /// The items that are always in scope.
    #[serde(default)]
    pub include: FilterSection,

    /// The items that are never in scope.
    #[serde(default)]
    pub exclude: FilterSection,

    /// The apps that are quit around a cask upgrade.
    #[serde(default)]
    pub apps: AppSection,
}

/// The `[preflight]` section: the checks that run before an update.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightSection {
    /// Whether the network reachability check runs (default: `true`).
    pub network: Option<bool>,

    /// Whether the free disk space check runs (default: `true`).
    pub disk_space: Option<bool>,

    /// Whether the battery check runs (default: `true`).
    pub battery: Option<bool>,

    /// Whether the metered connection check runs (default: `true`).
    pub metered_connection: Option<bool>,

    /// Minimum free disk space in GiB before an update starts (default: `1`).
    pub min_free_disk_gib: Option<u64>,

    /// Battery percentage below which an update on battery is refused
    /// (default: `20`, maximum: `100`).
    #[serde(default, deserialize_with = "deserialize_battery_threshold")]
    pub battery_threshold: Option<u8>,
}

/// The `[package_manager]` section: what happens when another package
/// manager is running.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManagerSection {
    /// Whether freshen waits for another package manager (default: `true`).
    pub enabled: Option<bool>,

    /// Seconds to wait for another package manager before aborting
    /// (default: `30`).
    pub wait_seconds: Option<u64>,
}

/// The `[timeouts]` section: how long a step and the whole run may take.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeoutSection {
    /// Seconds a single step may take; `0` disables the timeout (default: `0`).
    pub step_seconds: Option<u64>,

    /// Seconds the whole run may take; `0` disables the timeout (default: `0`).
    pub total_seconds: Option<u64>,
}

/// The `[run]` section: how the run itself behaves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSection {
    /// How many read-only checks `--check` queries at the same time
    /// (default: `4`, minimum: `1`).
    #[serde(default, deserialize_with = "deserialize_jobs")]
    pub jobs: Option<usize>,

    /// How many pre-upgrade snapshot files to keep; `0` keeps every snapshot
    /// (default: `10`).
    pub snapshot_retention: Option<usize>,
}

/// The `[include]` and `[exclude]` sections: the items a run may touch.
///
/// Every list is optional. An empty include list means "everything of this
/// kind", and the exclude list always wins over the include list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterSection {
    /// Homebrew casks.
    pub casks: Option<Vec<String>>,

    /// Packages, that is Homebrew formulae and uv tools.
    pub packages: Option<Vec<String>>,

    /// Standalone containers.
    pub containers: Option<Vec<String>>,

    /// Compose projects.
    pub projects: Option<Vec<String>>,

    /// Mac App Store apps, by App Store id.
    pub appstore: Option<Vec<String>>,
}

/// The `[apps]` section: the applications that are quit around a cask upgrade.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSection {
    /// Applications that are never quit.
    pub never_quit: Option<Vec<String>>,

    /// Applications that are always quit, without asking.
    pub always_quit: Option<Vec<String>>,
}

impl Config {
    /// Parses configuration text.
    ///
    /// The path is only used to name the file in error messages.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Invalid`] when the text is not valid freshen
    /// configuration; the error points at the offending line.
    pub fn parse(path: &Path, text: &str) -> Result<Self, ConfigError> {
        toml::from_str::<Self>(text).map_err(|error| invalid(path, text, &error))
    }

    /// Loads the configuration from `path`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Missing`] when the file does not exist and
    /// [`ConfigError::Invalid`] when it is not valid configuration.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                ConfigError::Missing {
                    path: path.to_owned(),
                }
            } else {
                ConfigError::Read {
                    path: path.to_owned(),
                    source,
                }
            }
        })?;

        Self::parse(path, &text)
    }

    /// Loads the configuration from `path`, returning `None` when the file
    /// does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Invalid`] when the file is not valid
    /// configuration.
    pub fn load_optional(path: &Path) -> Result<Option<Self>, ConfigError> {
        match Self::load(path) {
            Ok(config) => Ok(Some(config)),
            Err(ConfigError::Missing { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The include and exclude lists of the file.
    #[must_use]
    pub fn item_filters(&self) -> ItemFilters {
        ItemFilters::new()
            .with_casks(filter(&self.include.casks, &self.exclude.casks))
            .with_packages(filter(&self.include.packages, &self.exclude.packages))
            .with_containers(filter(&self.include.containers, &self.exclude.containers))
            .with_projects(filter(&self.include.projects, &self.exclude.projects))
            .with_appstore(filter(&self.include.appstore, &self.exclude.appstore))
    }

    /// The app lists of the file.
    #[must_use]
    pub fn app_lists(&self) -> AppLists {
        AppLists::new()
            .with_never_quit(self.apps.never_quit.clone().unwrap_or_default())
            .with_always_quit(self.apps.always_quit.clone().unwrap_or_default())
    }

    /// The step ids the file enables or disables.
    ///
    /// A step that is not named keeps its default, which is enabled.
    #[must_use]
    pub fn step_overrides(&self) -> BTreeMap<String, bool> {
        self.steps.clone()
    }
}

/// Builds the filter of one kind of item from its include and exclude lists.
fn filter(include: &Option<Vec<String>>, exclude: &Option<Vec<String>>) -> NameFilter {
    NameFilter::new()
        .with_include(include.clone().unwrap_or_default())
        .with_exclude(exclude.clone().unwrap_or_default())
}

/// The environment values that decide where the configuration file lives.
///
/// Reading them from a struct instead of from the process environment keeps
/// the location rules testable on every platform.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigEnvironment {
    /// `%APPDATA%`: the roaming application data directory on Windows.
    pub appdata: Option<PathBuf>,

    /// `XDG_CONFIG_HOME`: the XDG configuration directory.
    pub xdg_config_home: Option<PathBuf>,

    /// The home directory of the user.
    pub home: Option<PathBuf>,
}

impl ConfigEnvironment {
    /// Reads the values from the environment of the running process.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            appdata: env_path("APPDATA"),
            xdg_config_home: env_path("XDG_CONFIG_HOME"),
            home: env_path("HOME").or_else(|| env_path("USERPROFILE")),
        }
    }

    /// The directory that holds the configuration file on `platform`.
    #[must_use]
    pub fn config_dir(&self, platform: Platform) -> PathBuf {
        let base = match platform {
            Platform::Windows => self
                .appdata
                .clone()
                .or_else(|| {
                    self.home
                        .as_ref()
                        .map(|home| home.join("AppData").join("Roaming"))
                })
                .unwrap_or_else(env::temp_dir),
            Platform::Macos | Platform::Linux => self
                .xdg_config_home
                .clone()
                .or_else(|| self.home.as_ref().map(|home| home.join(".config")))
                .unwrap_or_else(env::temp_dir),
        };

        base.join(CONFIG_DIR_NAME)
    }

    /// The configuration file on `platform`.
    #[must_use]
    pub fn config_path(&self, platform: Platform) -> PathBuf {
        self.config_dir(platform).join(CONFIG_FILE_NAME)
    }
}

/// The configuration file of the running platform.
///
/// Returns `None` when freshen does not support the operating system it was
/// built for.
#[must_use]
pub fn config_path() -> Option<PathBuf> {
    Platform::current().map(|platform| ConfigEnvironment::from_env().config_path(platform))
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = env::var_os(key)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn invalid(path: &Path, text: &str, error: &toml::de::Error) -> ConfigError {
    let message = error
        .message()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let (line, column) = error
        .span()
        .map_or((1, 1), |span| line_and_column(text, span.start));

    ConfigError::Invalid {
        path: path.to_owned(),
        line,
        column,
        message,
    }
}

fn line_and_column(text: &str, offset: usize) -> (usize, usize) {
    let mut start = offset.min(text.len());
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }

    let before = &text[..start];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(1, |line| line.chars().count() + 1);

    (line, column)
}

fn deserialize_battery_threshold<'de, D>(deserializer: D) -> Result<Option<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<u8>::deserialize(deserializer)?;
    if let Some(value) = value {
        if value > 100 {
            return Err(D::Error::custom(format!(
                "battery_threshold must be between 0 and 100, got {value}"
            )));
        }
    }

    Ok(value)
}

fn deserialize_jobs<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<usize>::deserialize(deserializer)?;
    if let Some(value) = value {
        if value == 0 {
            return Err(D::Error::custom("jobs must be at least 1"));
        }
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigEnvironment, ConfigError, CONFIG_DIR_NAME, CONFIG_FILE_NAME};
    use crate::platform::Platform;
    use std::path::{Path, PathBuf};

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::parse(Path::new("config.toml"), text)
    }

    fn parsed(text: &str) -> Config {
        match parse(text) {
            Ok(config) => config,
            Err(error) => panic!("expected a valid configuration, got {error}"),
        }
    }

    fn invalid(error: Option<ConfigError>) -> (PathBuf, usize, usize, String) {
        match error {
            Some(ConfigError::Invalid {
                path,
                line,
                column,
                message,
            }) => (path, line, column, message),
            Some(other) => panic!("expected an invalid configuration error, got {other}"),
            None => panic!("expected an invalid configuration error"),
        }
    }

    #[test]
    fn an_empty_file_is_a_valid_configuration() {
        let config = parsed("");

        assert_eq!(config, Config::default());
    }

    #[test]
    fn the_step_flags_are_read() {
        let config = parsed(
            r#"
[steps]
brew-casks = false
docker = true
"#,
        );

        assert_eq!(config.steps.get("brew-casks"), Some(&false));
        assert_eq!(config.steps.get("docker"), Some(&true));
        assert_eq!(config.step_overrides().len(), 2);
    }

    #[test]
    fn the_include_and_exclude_lists_are_read() {
        let config = parsed(
            r#"
[include]
casks = ["firefox"]
packages = ["node"]
containers = ["web"]
projects = ["stack"]

[exclude]
casks = ["dropbox"]
packages = ["python"]
containers = ["old"]
projects = ["legacy"]
"#,
        );

        let filters = config.item_filters();

        assert_eq!(filters.casks().include(), ["firefox"]);
        assert_eq!(filters.casks().exclude(), ["dropbox"]);
        assert_eq!(filters.packages().include(), ["node"]);
        assert_eq!(filters.packages().exclude(), ["python"]);
        assert_eq!(filters.containers().include(), ["web"]);
        assert_eq!(filters.containers().exclude(), ["old"]);
        assert_eq!(filters.projects().include(), ["stack"]);
        assert_eq!(filters.projects().exclude(), ["legacy"]);
    }

    #[test]
    fn the_app_lists_are_read() {
        let config = parsed(
            r#"
[apps]
never_quit = ["com.docker", "Terminal.app"]
always_quit = ["com.spotify"]
"#,
        );

        let lists = config.app_lists();

        assert_eq!(lists.never_quit(), ["com.docker", "Terminal.app"]);
        assert_eq!(lists.always_quit(), ["com.spotify"]);
    }

    #[test]
    fn a_configuration_without_the_new_sections_filters_nothing() {
        let config = parsed("[run]\njobs = 2\n");

        assert!(config.item_filters().is_empty());
        assert!(config.app_lists().is_empty());
        assert!(config.step_overrides().is_empty());
    }

    #[test]
    fn an_unknown_key_in_a_list_section_points_at_its_line() {
        let error = parse("[include]\ncask = [\"firefox\"]\n").err();

        let (_, line, _, message) = invalid(error);

        assert_eq!(line, 2);
        assert!(
            message.contains("unknown field `cask`"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn an_unknown_key_in_the_app_section_points_at_its_line() {
        let error = parse("[apps]\nnever_quit_app = [\"com.docker\"]\n").err();

        let (_, line, _, message) = invalid(error);

        assert_eq!(line, 2);
        assert!(
            message.contains("unknown field `never_quit_app`"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn a_list_that_is_not_a_list_of_names_points_at_its_line() {
        let error = parse("[include]\ncasks = \"firefox\"\n").err();

        let (_, line, _, message) = invalid(error);

        assert_eq!(line, 2);
        assert!(
            message.contains("invalid type"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn every_section_is_read() {
        let config = parsed(
            r#"
[preflight]
network = false
disk_space = false
battery = false
metered_connection = false
min_free_disk_gib = 5
battery_threshold = 35

[package_manager]
enabled = false
wait_seconds = 10

[timeouts]
step_seconds = 120
total_seconds = 3600

[run]
jobs = 8
snapshot_retention = 3
"#,
        );

        assert_eq!(config.preflight.network, Some(false));
        assert_eq!(config.preflight.disk_space, Some(false));
        assert_eq!(config.preflight.battery, Some(false));
        assert_eq!(config.preflight.metered_connection, Some(false));
        assert_eq!(config.preflight.min_free_disk_gib, Some(5));
        assert_eq!(config.preflight.battery_threshold, Some(35));
        assert_eq!(config.package_manager.enabled, Some(false));
        assert_eq!(config.package_manager.wait_seconds, Some(10));
        assert_eq!(config.timeouts.step_seconds, Some(120));
        assert_eq!(config.timeouts.total_seconds, Some(3600));
        assert_eq!(config.run.jobs, Some(8));
        assert_eq!(config.run.snapshot_retention, Some(3));
    }

    #[test]
    fn an_unknown_key_points_at_its_line() {
        let error = parse("[preflight]\nnetwork = true\nnetwrok = false\n").err();

        let (path, line, column, message) = invalid(error);

        assert_eq!(path, PathBuf::from("config.toml"));
        assert_eq!(line, 3);
        assert_eq!(column, 1);
        assert!(
            message.contains("unknown field `netwrok`"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn an_unknown_section_points_at_its_line() {
        let error = parse("[preflight]\nnetwork = true\n\n[prefllight]\n").err();

        let (_, line, _, message) = invalid(error);

        assert_eq!(line, 4);
        assert!(
            message.contains("unknown field `prefllight`"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn a_wrong_type_points_at_its_line() {
        let error = parse("[run]\njobs = \"many\"\n").err();

        let (_, line, column, message) = invalid(error);

        assert_eq!(line, 2);
        assert_eq!(column, 8);
        assert!(
            message.contains("invalid type"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn a_syntax_error_points_at_its_line() {
        let error = parse("[run]\njobs = \n").err();

        let (_, line, _, message) = invalid(error);

        assert_eq!(line, 2);
        assert!(!message.is_empty());
    }

    #[test]
    fn a_battery_threshold_above_the_maximum_points_at_its_line() {
        let error = parse("[preflight]\nbattery_threshold = 200\n").err();

        let (_, line, column, message) = invalid(error);

        assert_eq!(line, 2);
        assert_eq!(column, 21);
        assert!(
            message.contains("battery_threshold must be between 0 and 100"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn zero_jobs_is_refused_with_its_line() {
        let error = parse("[run]\njobs = 0\n").err();

        let (_, line, _, message) = invalid(error);

        assert_eq!(line, 2);
        assert!(
            message.contains("jobs must be at least 1"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn the_error_message_names_the_file_and_the_position() {
        let error = Config::parse(
            Path::new("/home/user/.config/freshen/config.toml"),
            "[run]\njobs = 0\n",
        )
        .err();

        let message = match error {
            Some(error) => error.to_string(),
            None => panic!("expected an invalid configuration error"),
        };

        assert!(
            message.starts_with("/home/user/.config/freshen/config.toml:2:"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn the_configuration_lives_in_the_dot_config_directory_on_unix() {
        let environment = ConfigEnvironment {
            home: Some(PathBuf::from("/home/user")),
            ..ConfigEnvironment::default()
        };

        for platform in [Platform::Macos, Platform::Linux] {
            assert_eq!(
                environment.config_path(platform),
                PathBuf::from("/home/user/.config/freshen/config.toml"),
                "{platform}"
            );
        }
    }

    #[test]
    fn xdg_config_home_wins_over_the_home_directory() {
        let environment = ConfigEnvironment {
            xdg_config_home: Some(PathBuf::from("/home/user/cfg")),
            home: Some(PathBuf::from("/home/user")),
            ..ConfigEnvironment::default()
        };

        assert_eq!(
            environment.config_path(Platform::Linux),
            PathBuf::from("/home/user/cfg/freshen/config.toml")
        );
    }

    #[test]
    fn the_configuration_lives_in_appdata_on_windows() {
        let environment = ConfigEnvironment {
            appdata: Some(PathBuf::from(r"C:\Users\user\AppData\Roaming")),
            home: Some(PathBuf::from(r"C:\Users\user")),
            ..ConfigEnvironment::default()
        };

        assert_eq!(
            environment.config_path(Platform::Windows),
            PathBuf::from(r"C:\Users\user\AppData\Roaming")
                .join(CONFIG_DIR_NAME)
                .join(CONFIG_FILE_NAME)
        );
    }

    #[test]
    fn windows_falls_back_to_the_roaming_directory_of_the_home_directory() {
        let environment = ConfigEnvironment {
            home: Some(PathBuf::from(r"C:\Users\user")),
            ..ConfigEnvironment::default()
        };

        assert_eq!(
            environment.config_path(Platform::Windows),
            PathBuf::from(r"C:\Users\user")
                .join("AppData")
                .join("Roaming")
                .join(CONFIG_DIR_NAME)
                .join(CONFIG_FILE_NAME)
        );
    }

    #[test]
    fn the_directory_and_the_file_have_stable_names() {
        let environment = ConfigEnvironment {
            home: Some(PathBuf::from("/home/user")),
            ..ConfigEnvironment::default()
        };

        assert_eq!(
            environment.config_dir(Platform::Linux),
            PathBuf::from("/home/user/.config").join(CONFIG_DIR_NAME)
        );
        assert!(environment
            .config_path(Platform::Linux)
            .ends_with(CONFIG_FILE_NAME));
    }

    #[test]
    fn the_configuration_file_of_the_running_platform_is_found() {
        let expected =
            Platform::current().map(|platform| ConfigEnvironment::from_env().config_path(platform));

        assert_eq!(super::config_path(), expected);
    }

    #[test]
    fn a_missing_file_is_not_an_error_when_it_is_optional() {
        let path = Path::new("this-file-does-not-exist.toml");

        let config = Config::load_optional(path);

        assert!(matches!(config, Ok(None)), "unexpected result: {config:?}");
    }

    #[test]
    fn a_missing_file_is_an_error_when_it_was_asked_for() {
        let path = Path::new("this-file-does-not-exist.toml");

        let error = Config::load(path).err();

        assert!(
            matches!(error, Some(ConfigError::Missing { .. })),
            "unexpected error: {error:?}"
        );
    }
}
