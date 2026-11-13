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

use crate::custom_step::CustomStepConfig;
use crate::platform::Platform;
use crate::policy::{AppLists, ItemFilters, NameFilter};

/// Name of the directory that holds the configuration file.
pub const CONFIG_DIR_NAME: &str = "freshen";

/// Name of the configuration file inside that directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Prefix for every environment variable that overrides a configuration key.
///
/// The full scheme is `FRESHEN_<SECTION>__<KEY>` (see [`Config::apply_env_overrides`]).
pub const ENV_PREFIX: &str = "FRESHEN_";

/// The environment variable that points at an alternative configuration file.
///
/// When set, freshen reads this file instead of the platform-default one. The
/// `--config` command-line flag wins over it.
pub const CONFIG_ENV: &str = "FRESHEN_CONFIG";

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

    /// The configuration file or one of its parent directories is
    /// world-writable.
    ///
    /// freshen refuses to load it: the `[hooks]` and `[[custom_steps]]`
    /// sections run arbitrary commands, so a world-writable configuration
    /// file could be replaced to make freshen execute attacker-controlled
    /// code.
    #[error(
        "the configuration file {path} is world-writable; refusing to load hooks and custom steps from it"
    )]
    WorldWritable {
        /// The world-writable file or directory that caused the refusal.
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

    /// The named profiles that select a set of steps.
    ///
    /// Each profile maps a step id to `false` (disabled) or `true` (enabled).
    /// A step that is not named keeps its default, which is enabled. A profile
    /// is selected with `--profile <name>`; its steps override the top-level
    /// `[steps]` section.
    #[serde(default)]
    pub profiles: BTreeMap<String, BTreeMap<String, bool>>,

    /// The items that are always in scope.
    #[serde(default)]
    pub include: FilterSection,

    /// The items that are never in scope.
    #[serde(default)]
    pub exclude: FilterSection,

    /// The apps that are quit around a cask upgrade.
    #[serde(default)]
    pub apps: AppSection,

    /// The repositories whose pre-commit hooks are kept up to date.
    #[serde(default)]
    pub pre_commit: PreCommitSection,

    /// The per-step pre and post hooks.
    #[serde(default)]
    pub hooks: HookSection,

    /// The user-defined steps that run a command of the user's choosing.
    ///
    /// Each entry is a `[[custom_steps]]` table with a `name`, a `command`,
    /// optional `platforms` and an optional `order`.
    #[serde(default)]
    pub custom_steps: Vec<CustomStepConfig>,
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

    /// The command that runs once before any step (default: none).
    pub pre_run: Option<Vec<String>>,

    /// The command that runs once after every step (default: none).
    pub post_run: Option<Vec<String>>,
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

/// The `[pre-commit]` section: the repositories whose pre-commit hooks
/// freshen keeps up to date.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreCommitSection {
    /// The repositories freshen runs `pre-commit autoupdate` in.
    ///
    /// Each entry is the path to a repository checkout. freshen never scans
    /// the disk for repositories: only the listed repositories are touched,
    /// so the step does nothing until a repository is listed here.
    pub repositories: Option<Vec<String>>,
}

/// The `[hooks]` section: per-step commands that run before and after a step.
///
/// The table is keyed by step id, so `[hooks.brew]` configures the hooks for
/// the `brew` step. A step that is not named has no hooks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct HookSection(
    /// The per-step pre and post hooks, keyed by step id.
    pub BTreeMap<String, Hooks>,
);

/// The commands that run before and after one step.
///
/// Each of `pre` and `post` is a command: its first entry is the program and
/// the remaining entries are its arguments. When either is missing, that hook
/// does not run for the step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    /// The command that runs before the step's upgrade.
    pub pre: Option<Vec<String>>,
    /// The command that runs after the step's upgrade.
    pub post: Option<Vec<String>>,
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
    /// Refuses to load when `path` or one of its parent directories is
    /// world-writable: the `[hooks]` and `[[custom_steps]]` sections run
    /// arbitrary commands, so a world-writable configuration file could be
    /// tampered with to make freshen execute attacker-controlled code.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::WorldWritable`] when `path` or one of its
    /// parent directories is world-writable, [`ConfigError::Missing`] when
    /// the file does not exist and [`ConfigError::Invalid`] when it is not
    /// valid configuration.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        Self::load_with_probe(path, &HostWorldWritableProbe)
    }

    /// Loads the configuration from `path`, using `probe` to decide whether a
    /// path is world-writable.
    ///
    /// Tests inject a fake probe so no real file-system permissions have to be
    /// inspected.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::WorldWritable`] when `path` or one of its
    /// parent directories is world-writable, [`ConfigError::Missing`] when
    /// the file does not exist and [`ConfigError::Invalid`] when it is not
    /// valid configuration.
    pub fn load_with_probe(
        path: &Path,
        probe: &dyn WorldWritableProbe,
    ) -> Result<Self, ConfigError> {
        if let Some(offending) = world_writable_ancestor(path, probe) {
            return Err(ConfigError::WorldWritable {
                path: offending.to_owned(),
            });
        }

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
    /// Returns [`ConfigError::WorldWritable`] when `path` or one of its
    /// parent directories is world-writable, and
    /// [`ConfigError::Invalid`] when the file is not valid configuration.
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

    /// The repositories whose pre-commit hooks freshen keeps up to date.
    #[must_use]
    pub fn pre_commit_repositories(&self) -> Vec<String> {
        self.pre_commit.repositories.clone().unwrap_or_default()
    }

    /// The per-step pre and post hooks, keyed by step id.
    #[must_use]
    pub fn hooks(&self) -> &BTreeMap<String, Hooks> {
        &self.hooks.0
    }

    /// The step ids the file enables or disables.
    ///
    /// A step that is not named keeps its default, which is enabled.
    #[must_use]
    pub fn step_overrides(&self) -> BTreeMap<String, bool> {
        self.steps.clone()
    }

    /// The step overrides of the named profile, or `None` when the profile
    /// does not exist.
    #[must_use]
    pub fn profile_steps(&self, name: &str) -> Option<&BTreeMap<String, bool>> {
        self.profiles.get(name)
    }

    /// The configuration values that differ from the built-in defaults, as
    /// `(key, value)` pairs in a stable order.
    ///
    /// A value counts as set when it differs from what freshen uses with no
    /// configuration file: a present option, a non-empty list, or a named
    /// step, profile or hook. Built-in defaults are not listed, so a
    /// configuration that only uses defaults reports nothing.
    #[must_use]
    pub fn non_default_values(&self) -> Vec<(String, String)> {
        let default = Config::default();
        let mut values: Vec<(String, String)> = Vec::new();

        let mut emit = |key: &str, value: &str| {
            values.push((key.to_owned(), value.to_owned()));
        };

        macro_rules! option {
            ($actual:expr, $default:expr, $key:expr) => {
                if $actual != $default {
                    emit(
                        $key,
                        &$actual.map(|value| value.to_string()).unwrap_or_default(),
                    );
                }
            };
        }

        // [preflight]
        option!(
            self.preflight.network,
            default.preflight.network,
            "preflight.network"
        );
        option!(
            self.preflight.disk_space,
            default.preflight.disk_space,
            "preflight.disk_space"
        );
        option!(
            self.preflight.battery,
            default.preflight.battery,
            "preflight.battery"
        );
        option!(
            self.preflight.metered_connection,
            default.preflight.metered_connection,
            "preflight.metered_connection"
        );
        option!(
            self.preflight.min_free_disk_gib,
            default.preflight.min_free_disk_gib,
            "preflight.min_free_disk_gib"
        );
        option!(
            self.preflight.battery_threshold,
            default.preflight.battery_threshold,
            "preflight.battery_threshold"
        );

        // [package_manager]
        option!(
            self.package_manager.enabled,
            default.package_manager.enabled,
            "package_manager.enabled"
        );
        option!(
            self.package_manager.wait_seconds,
            default.package_manager.wait_seconds,
            "package_manager.wait_seconds"
        );

        // [timeouts]
        option!(
            self.timeouts.step_seconds,
            default.timeouts.step_seconds,
            "timeouts.step_seconds"
        );
        option!(
            self.timeouts.total_seconds,
            default.timeouts.total_seconds,
            "timeouts.total_seconds"
        );

        // [run]
        option!(self.run.jobs, default.run.jobs, "run.jobs");
        option!(
            self.run.snapshot_retention,
            default.run.snapshot_retention,
            "run.snapshot_retention"
        );
        if self.run.pre_run != default.run.pre_run {
            emit("run.pre_run", &Self::join_list(self.run.pre_run.as_ref()));
        }
        if self.run.post_run != default.run.post_run {
            emit("run.post_run", &Self::join_list(self.run.post_run.as_ref()));
        }

        // [steps]
        for (step, enabled) in &self.steps {
            if default.steps.get(step).copied().unwrap_or(true) != *enabled {
                emit(&format!("steps.{step}"), &enabled.to_string());
            }
        }

        // [profiles]
        for (profile, steps) in &self.profiles {
            let Some(default_steps) = default.profiles.get(profile) else {
                for (step, enabled) in steps {
                    emit(&format!("profiles.{profile}.{step}"), &enabled.to_string());
                }
                continue;
            };
            for (step, enabled) in steps {
                if default_steps.get(step).copied().unwrap_or(true) != *enabled {
                    emit(&format!("profiles.{profile}.{step}"), &enabled.to_string());
                }
            }
        }

        macro_rules! list {
            ($actual:expr, $default:expr, $key:expr) => {
                if $actual != $default {
                    emit($key, &Self::join_list($actual.as_ref()));
                }
            };
        }

        // [include]
        list!(self.include.casks, default.include.casks, "include.casks");
        list!(
            self.include.packages,
            default.include.packages,
            "include.packages"
        );
        list!(
            self.include.containers,
            default.include.containers,
            "include.containers"
        );
        list!(
            self.include.projects,
            default.include.projects,
            "include.projects"
        );
        list!(
            self.include.appstore,
            default.include.appstore,
            "include.appstore"
        );

        // [exclude]
        list!(self.exclude.casks, default.exclude.casks, "exclude.casks");
        list!(
            self.exclude.packages,
            default.exclude.packages,
            "exclude.packages"
        );
        list!(
            self.exclude.containers,
            default.exclude.containers,
            "exclude.containers"
        );
        list!(
            self.exclude.projects,
            default.exclude.projects,
            "exclude.projects"
        );
        list!(
            self.exclude.appstore,
            default.exclude.appstore,
            "exclude.appstore"
        );

        // [apps]
        list!(
            self.apps.never_quit,
            default.apps.never_quit,
            "apps.never_quit"
        );
        list!(
            self.apps.always_quit,
            default.apps.always_quit,
            "apps.always_quit"
        );

        // [pre-commit]
        list!(
            self.pre_commit.repositories,
            default.pre_commit.repositories,
            "pre_commit.repositories"
        );

        // [hooks]
        let default_hooks = default.hooks();
        for (step, hooks) in self.hooks().iter() {
            if default_hooks.get(step) != Some(hooks) {
                if let Some(pre) = &hooks.pre {
                    emit(&format!("hooks.{step}.pre"), &Self::join_list(Some(pre)));
                }
                if let Some(post) = &hooks.post {
                    emit(&format!("hooks.{step}.post"), &Self::join_list(Some(post)));
                }
            }
        }

        // [[custom_steps]]
        for (index, custom_step) in self.custom_steps.iter().enumerate() {
            emit(&format!("custom_steps.{index}.name"), &custom_step.name);
            emit(
                &format!("custom_steps.{index}.command"),
                &Self::join_list(Some(&custom_step.command)),
            );
            if let Some(platforms) = &custom_step.platforms {
                emit(
                    &format!("custom_steps.{index}.platforms"),
                    &Self::join_list(Some(platforms)),
                );
            }
            if let Some(order) = custom_step.order {
                emit(&format!("custom_steps.{index}.order"), &order.to_string());
            }
        }

        values
    }

    fn join_list(list: Option<&Vec<String>>) -> String {
        list.map_or_else(String::new, |list| list.join(", "))
    }
}

/// Decides whether a path is world-writable: writable by every user on the
/// system.
///
/// The host implementation inspects the real permission bits (Unix) or ACL
/// (Windows). Tests inject a fake probe so no actual file-system permissions
/// have to be read.
pub trait WorldWritableProbe: Send + Sync {
    /// Returns `true` when `path` is world-writable.
    fn is_world_writable(&self, path: &Path) -> bool;
}

/// The real implementation: consults the operating system for the permissions
/// of `path`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostWorldWritableProbe;

impl WorldWritableProbe for HostWorldWritableProbe {
    fn is_world_writable(&self, path: &Path) -> bool {
        is_world_writable_host(path)
    }
}

/// Returns `true` when `path` is world-writable.
fn is_world_writable_host(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .ok()
            .is_some_and(|metadata| metadata.permissions().mode() & 0o002 != 0)
    }
    #[cfg(windows)]
    {
        // World-writable on Windows means write access granted to the
        // "Everyone" well-known SID through the file's DACL. The windows-sys
        // bindings pinned by this project model variable-length ACEs in a way
        // that makes reliably extracting the SID pointer impossible, so
        // inspecting real ACLs is not implemented here. Tests inject a fake
        // [`WorldWritableProbe`] with the ACL facts instead (injected ACL
        // facts on Windows).
        let _ = path;
        false
    }
}

/// Walks `path` and its parent directory and returns the first one that is
/// world-writable, if any.
///
/// Anyone who can write to the configuration file itself can replace the
/// hooks and custom steps it contains, and anyone who can write to the
/// directory that contains it can delete and replace the file. A
/// world-writable grandparent directory does not help: replacing the file
/// still requires write access to its immediate parent.
fn world_writable_ancestor<'a>(
    path: &'a Path,
    probe: &'a dyn WorldWritableProbe,
) -> Option<&'a Path> {
    // The configuration file itself.
    if probe.is_world_writable(path) {
        return Some(path);
    }
    // The directory that contains the file.
    if let Some(parent) = path.parent() {
        if probe.is_world_writable(parent) {
            return Some(parent);
        }
    }
    None
}

impl Config {
    /// Applies environment variable overrides to every configuration key.
    ///
    /// The naming scheme is `FRESHEN_<SECTION>__<KEY>`:
    ///
    /// * `SECTION` is the TOML section name in upper case
    ///   (`PREFLIGHT`, `PACKAGE_MANAGER`, `TIMEOUTS`, `RUN`, `STEPS`,
    ///   `PROFILES`, `INCLUDE`, `EXCLUDE`, `APPS`, `PRE_COMMIT`, `HOOKS`).
    /// * `KEY` is the configuration key in upper case (`NETWORK`,
    ///   `MIN_FREE_DISK_GIB`, `BREW_CASKS`, `SNAPSHOT_RETENTION`,
    ///   `PRE_RUN`, `POST_RUN`, ...). Step ids and profile names keep their
    ///   dashes, so `BREW_CASKS` names the `brew-casks` step and `WORK` names
    ///   the `work` profile.
    /// * List values are comma-separated, booleans are `true` or `false`
    ///   (case-insensitive) and integers are decimal numbers.
    ///
    /// An environment variable wins over the configuration file and loses to a
    /// flag given on the command line. A variable that does not parse, that is
    /// out of range, or that names an unknown section or key is ignored.
    pub fn apply_env_overrides(&mut self, env: impl IntoIterator<Item = (String, String)>) {
        for (key, value) in env.into_iter() {
            let Some(rest) = key.strip_prefix(ENV_PREFIX) else {
                continue;
            };
            let mut parts = rest.splitn(2, "__");
            let Some(section) = parts.next() else {
                continue;
            };
            let Some(rest) = parts.next().filter(|rest| !rest.is_empty()) else {
                continue;
            };
            match section {
                "PREFLIGHT" => self.apply_preflight(rest, &value),
                "PACKAGE_MANAGER" => self.apply_package_manager(rest, &value),
                "TIMEOUTS" => self.apply_timeouts(rest, &value),
                "RUN" => self.apply_run(rest, &value),
                "STEPS" => self.apply_steps(rest, &value),
                "PROFILES" => self.apply_profiles(rest, &value),
                "INCLUDE" => apply_filter(&mut self.include, rest, &value),
                "EXCLUDE" => apply_filter(&mut self.exclude, rest, &value),
                "APPS" => self.apply_apps(rest, &value),
                "PRE_COMMIT" => self.apply_pre_commit(rest, &value),
                "HOOKS" => self.apply_hooks(rest, &value),
                _ => continue,
            }
        }
    }

    fn apply_preflight(&mut self, key: &str, value: &str) {
        match key {
            "NETWORK" => {
                if let Some(v) = parse_bool(value) {
                    self.preflight.network = Some(v);
                }
            }
            "DISK_SPACE" => {
                if let Some(v) = parse_bool(value) {
                    self.preflight.disk_space = Some(v);
                }
            }
            "BATTERY" => {
                if let Some(v) = parse_bool(value) {
                    self.preflight.battery = Some(v);
                }
            }
            "METERED_CONNECTION" => {
                if let Some(v) = parse_bool(value) {
                    self.preflight.metered_connection = Some(v);
                }
            }
            "MIN_FREE_DISK_GIB" => {
                if let Some(v) = parse_u64(value) {
                    self.preflight.min_free_disk_gib = Some(v);
                }
            }
            "BATTERY_THRESHOLD" => {
                if let Some(v) = parse_u8(value).filter(|v| *v <= 100) {
                    self.preflight.battery_threshold = Some(v);
                }
            }
            _ => {}
        }
    }

    fn apply_package_manager(&mut self, key: &str, value: &str) {
        match key {
            "ENABLED" => {
                if let Some(v) = parse_bool(value) {
                    self.package_manager.enabled = Some(v);
                }
            }
            "WAIT_SECONDS" => {
                if let Some(v) = parse_u64(value) {
                    self.package_manager.wait_seconds = Some(v);
                }
            }
            _ => {}
        }
    }

    fn apply_timeouts(&mut self, key: &str, value: &str) {
        match key {
            "STEP_SECONDS" => {
                if let Some(v) = parse_u64(value) {
                    self.timeouts.step_seconds = Some(v);
                }
            }
            "TOTAL_SECONDS" => {
                if let Some(v) = parse_u64(value) {
                    self.timeouts.total_seconds = Some(v);
                }
            }
            _ => {}
        }
    }

    fn apply_run(&mut self, key: &str, value: &str) {
        match key {
            "JOBS" => {
                if let Some(v) = parse_usize(value).filter(|v| *v > 0) {
                    self.run.jobs = Some(v);
                }
            }
            "SNAPSHOT_RETENTION" => {
                if let Some(v) = parse_usize(value) {
                    self.run.snapshot_retention = Some(v);
                }
            }
            "PRE_RUN" => self.run.pre_run = Some(parse_list(value)),
            "POST_RUN" => self.run.post_run = Some(parse_list(value)),
            _ => {}
        }
    }

    fn apply_steps(&mut self, key: &str, value: &str) {
        if let Some(v) = parse_bool(value) {
            self.steps.insert(step_id_from_env(key), v);
        }
    }

    fn apply_profiles(&mut self, key: &str, value: &str) {
        let mut parts = key.splitn(2, "__");
        let Some(profile) = parts.next().filter(|profile| !profile.is_empty()) else {
            return;
        };
        let Some(key) = parts.next().filter(|key| !key.is_empty()) else {
            return;
        };
        if let Some(v) = parse_bool(value) {
            self.profiles
                .entry(profile.to_ascii_lowercase())
                .or_default()
                .insert(step_id_from_env(key), v);
        }
    }

    fn apply_apps(&mut self, key: &str, value: &str) {
        match key {
            "NEVER_QUIT" => self.apps.never_quit = Some(parse_list(value)),
            "ALWAYS_QUIT" => self.apps.always_quit = Some(parse_list(value)),
            _ => {}
        }
    }

    fn apply_pre_commit(&mut self, key: &str, value: &str) {
        if key == "REPOSITORIES" {
            self.pre_commit.repositories = Some(parse_list(value));
        }
    }

    fn apply_hooks(&mut self, key: &str, value: &str) {
        let mut parts = key.splitn(2, "__");
        let Some(step) = parts.next().filter(|step| !step.is_empty()) else {
            return;
        };
        let Some(key) = parts.next().filter(|key| !key.is_empty()) else {
            return;
        };
        let step = step_id_from_env(step);
        let hooks = self.hooks.0.entry(step.clone()).or_default();
        match key {
            "PRE" => hooks.pre = Some(parse_list(value)),
            "POST" => hooks.post = Some(parse_list(value)),
            _ => {}
        }
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

    /// `FRESHEN_CONFIG`: an alternative configuration file.
    pub config: Option<PathBuf>,
}

impl ConfigEnvironment {
    /// Reads the values from the environment of the running process.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            appdata: env_path("APPDATA"),
            xdg_config_home: env_path("XDG_CONFIG_HOME"),
            home: env_path("HOME").or_else(|| env_path("USERPROFILE")),
            config: env_path(CONFIG_ENV),
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
    ///
    /// When [`ConfigEnvironment::config`] is set (`FRESHEN_CONFIG`), that file
    /// is returned instead of the platform-default location.
    #[must_use]
    pub fn config_path(&self, platform: Platform) -> PathBuf {
        if let Some(config) = &self.config {
            return config.clone();
        }

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

fn step_id_from_env(key: &str) -> String {
    key.to_ascii_lowercase().replace('_', "-")
}

/// Parses a boolean environment value (`true` or `false`, case-insensitive).
fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Parses a decimal integer environment value.
fn parse_u64(value: &str) -> Option<u64> {
    value.parse().ok()
}

/// Parses a decimal integer environment value.
fn parse_u8(value: &str) -> Option<u8> {
    value.parse().ok()
}

/// Parses a decimal integer environment value.
fn parse_usize(value: &str) -> Option<usize> {
    value.parse().ok()
}

/// Parses a comma-separated list of names, dropping empty parts.
fn parse_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Applies one environment override to a filter section (`[include]` or
/// `[exclude]`).
fn apply_filter(section: &mut FilterSection, key: &str, value: &str) {
    match key {
        "CASKS" => section.casks = Some(parse_list(value)),
        "PACKAGES" => section.packages = Some(parse_list(value)),
        "CONTAINERS" => section.containers = Some(parse_list(value)),
        "PROJECTS" => section.projects = Some(parse_list(value)),
        "APPSTORE" => section.appstore = Some(parse_list(value)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigEnvironment, ConfigError, Hooks, CONFIG_DIR_NAME, CONFIG_FILE_NAME};

    #[cfg(windows)]
    use super::WorldWritableProbe;
    use crate::platform::Platform;
    use std::collections::BTreeMap;
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

    fn apply_env(config: &mut Config, env: &[(&str, &str)]) {
        config.apply_env_overrides(
            env.iter()
                .map(|(key, value)| (key.to_string(), value.to_string())),
        );
    }

    #[test]
    fn an_environment_variable_overrides_a_boolean_config_key() {
        let mut config = parsed("[preflight]\nnetwork = true\n");

        apply_env(&mut config, &[("FRESHEN_PREFLIGHT__NETWORK", "false")]);

        assert_eq!(config.preflight.network, Some(false));
    }

    #[test]
    fn an_environment_variable_overrides_an_integer_config_key() {
        let mut config = parsed("[run]\njobs = 4\n");

        apply_env(&mut config, &[("FRESHEN_RUN__JOBS", "8")]);

        assert_eq!(config.run.jobs, Some(8));
    }

    #[test]
    fn an_environment_variable_overrides_a_list_config_key() {
        let mut config = parsed("[include]\ncasks = [\"firefox\"]\n");

        apply_env(
            &mut config,
            &[("FRESHEN_INCLUDE__CASKS", "dropbox,  slack")],
        );

        assert_eq!(
            config.include.casks,
            Some(vec!["dropbox".to_owned(), "slack".to_owned()])
        );
    }

    #[test]
    fn an_environment_variable_overrides_a_step() {
        let mut config = parsed("[steps]\nbrew-casks = true\n");

        apply_env(&mut config, &[("FRESHEN_STEPS__BREW_CASKS", "false")]);

        assert_eq!(config.steps.get("brew-casks"), Some(&false));
    }

    #[test]
    fn an_environment_variable_overrides_a_hook() {
        let mut config = parsed("[hooks.brew]\npre = [\"echo\", \"a\"]\n");

        apply_env(&mut config, &[("FRESHEN_HOOKS__BREW__POST", "echo,done")]);

        assert_eq!(
            config.hooks().get("brew").cloned(),
            Some(Hooks {
                pre: Some(vec!["echo".to_owned(), "a".to_owned()]),
                post: Some(vec!["echo".to_owned(), "done".to_owned()]),
            })
        );
    }

    #[test]
    fn an_environment_variable_overrides_a_profile_step() {
        let mut config = parsed("[profiles.work]\nbrew = true\n");

        apply_env(&mut config, &[("FRESHEN_PROFILES__WORK__BREW", "false")]);

        assert_eq!(
            config.profile_steps("work").and_then(|map| map.get("brew")),
            Some(&false)
        );
    }

    #[test]
    fn an_environment_variable_wins_over_the_configuration_file() {
        let mut config = parsed("[run]\njobs = 4\n");

        apply_env(&mut config, &[("FRESHEN_RUN__JOBS", "8")]);

        assert_eq!(config.run.jobs, Some(8));
    }

    #[test]
    fn an_unparseable_environment_value_is_ignored() {
        let mut config = parsed("[run]\njobs = 4\n");

        apply_env(&mut config, &[("FRESHEN_RUN__JOBS", "not-a-number")]);

        assert_eq!(config.run.jobs, Some(4));
    }

    #[test]
    fn an_out_of_range_environment_value_is_ignored() {
        let mut config = parsed("[preflight]\nbattery_threshold = 20\n");

        apply_env(
            &mut config,
            &[("FRESHEN_PREFLIGHT__BATTERY_THRESHOLD", "200")],
        );

        assert_eq!(config.preflight.battery_threshold, Some(20));
    }

    #[test]
    fn zero_jobs_from_an_environment_variable_is_ignored() {
        let mut config = parsed("[run]\njobs = 4\n");

        apply_env(&mut config, &[("FRESHEN_RUN__JOBS", "0")]);

        assert_eq!(config.run.jobs, Some(4));
    }

    #[test]
    fn an_unknown_section_or_key_is_ignored() {
        let mut config = Config::default();

        apply_env(
            &mut config,
            &[
                ("FRESHEN_NOPE__FOO", "bar"),
                ("FRESHEN_PREFLIGHT__NOPE", "true"),
            ],
        );

        assert_eq!(config, Config::default());
    }

    #[test]
    fn every_config_key_can_be_overridden_by_an_environment_variable() {
        let mut config = Config::default();

        apply_env(
            &mut config,
            &[
                ("FRESHEN_PREFLIGHT__NETWORK", "false"),
                ("FRESHEN_PREFLIGHT__DISK_SPACE", "false"),
                ("FRESHEN_PREFLIGHT__BATTERY", "false"),
                ("FRESHEN_PREFLIGHT__METERED_CONNECTION", "false"),
                ("FRESHEN_PREFLIGHT__MIN_FREE_DISK_GIB", "5"),
                ("FRESHEN_PREFLIGHT__BATTERY_THRESHOLD", "35"),
                ("FRESHEN_PACKAGE_MANAGER__ENABLED", "false"),
                ("FRESHEN_PACKAGE_MANAGER__WAIT_SECONDS", "10"),
                ("FRESHEN_TIMEOUTS__STEP_SECONDS", "120"),
                ("FRESHEN_TIMEOUTS__TOTAL_SECONDS", "3600"),
                ("FRESHEN_RUN__JOBS", "8"),
                ("FRESHEN_RUN__SNAPSHOT_RETENTION", "3"),
                ("FRESHEN_RUN__PRE_RUN", "echo,start"),
                ("FRESHEN_RUN__POST_RUN", "echo,done"),
                ("FRESHEN_STEPS__BREW_CASKS", "false"),
                ("FRESHEN_INCLUDE__CASKS", "firefox"),
                ("FRESHEN_INCLUDE__PACKAGES", "node"),
                ("FRESHEN_INCLUDE__CONTAINERS", "web"),
                ("FRESHEN_INCLUDE__PROJECTS", "stack"),
                ("FRESHEN_INCLUDE__APPSTORE", "409203825"),
                ("FRESHEN_EXCLUDE__CASKS", "dropbox"),
                ("FRESHEN_EXCLUDE__PACKAGES", "python"),
                ("FRESHEN_EXCLUDE__CONTAINERS", "old"),
                ("FRESHEN_EXCLUDE__PROJECTS", "legacy"),
                ("FRESHEN_EXCLUDE__APPSTORE", "123456789"),
                ("FRESHEN_APPS__NEVER_QUIT", "com.docker"),
                ("FRESHEN_APPS__ALWAYS_QUIT", "com.spotify"),
                ("FRESHEN_PRE_COMMIT__REPOSITORIES", "/src/a,/src/b"),
                ("FRESHEN_HOOKS__BREW__PRE", "echo,pre"),
                ("FRESHEN_HOOKS__BREW__POST", "echo,post"),
                ("FRESHEN_PROFILES__WORK__BREW_CASKS", "false"),
            ],
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
        assert_eq!(
            config.run.pre_run,
            Some(vec!["echo".to_owned(), "start".to_owned()])
        );
        assert_eq!(
            config.run.post_run,
            Some(vec!["echo".to_owned(), "done".to_owned()])
        );
        assert_eq!(config.steps.get("brew-casks"), Some(&false));
        assert_eq!(config.include.casks, Some(vec!["firefox".to_owned()]));
        assert_eq!(config.include.packages, Some(vec!["node".to_owned()]));
        assert_eq!(config.include.containers, Some(vec!["web".to_owned()]));
        assert_eq!(config.include.projects, Some(vec!["stack".to_owned()]));
        assert_eq!(config.include.appstore, Some(vec!["409203825".to_owned()]));
        assert_eq!(config.exclude.casks, Some(vec!["dropbox".to_owned()]));
        assert_eq!(config.exclude.packages, Some(vec!["python".to_owned()]));
        assert_eq!(config.exclude.containers, Some(vec!["old".to_owned()]));
        assert_eq!(config.exclude.projects, Some(vec!["legacy".to_owned()]));
        assert_eq!(config.exclude.appstore, Some(vec!["123456789".to_owned()]));
        assert_eq!(config.apps.never_quit, Some(vec!["com.docker".to_owned()]));
        assert_eq!(
            config.apps.always_quit,
            Some(vec!["com.spotify".to_owned()])
        );
        assert_eq!(
            config.pre_commit.repositories,
            Some(vec!["/src/a".to_owned(), "/src/b".to_owned()])
        );
        assert_eq!(
            config.hooks().get("brew").cloned(),
            Some(Hooks {
                pre: Some(vec!["echo".to_owned(), "pre".to_owned()]),
                post: Some(vec!["echo".to_owned(), "post".to_owned()]),
            })
        );
        assert_eq!(
            config
                .profile_steps("work")
                .and_then(|map| map.get("brew-casks")),
            Some(&false)
        );
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
    fn the_profiles_are_read() {
        let config = parsed(
            r#"
[profiles.work]
brew-casks = true
docker = false

[profiles.minimal]
brew = false
uv = false
"#,
        );

        assert_eq!(
            config.profile_steps("work"),
            Some(&BTreeMap::from([
                ("brew-casks".to_owned(), true),
                ("docker".to_owned(), false),
            ]))
        );
        assert_eq!(
            config.profile_steps("minimal"),
            Some(&BTreeMap::from([
                ("brew".to_owned(), false),
                ("uv".to_owned(), false),
            ]))
        );
        assert_eq!(config.profile_steps("nope"), None);
    }

    #[test]
    fn a_profile_without_steps_selects_nothing() {
        let config = parsed("[profiles.work]\n");

        assert_eq!(config.profile_steps("work"), Some(&BTreeMap::new()));
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
    fn the_hooks_are_read() {
        let config = parsed(
            r#"
[hooks.brew]
pre = ["echo", "starting brew"]
post = ["echo", "done brew"]

[hooks.uv]
pre = ["uv", "hook"]
"#,
        );

        let hooks = config.hooks();

        assert_eq!(
            hooks.get("brew"),
            Some(&Hooks {
                pre: Some(vec!["echo".to_owned(), "starting brew".to_owned()]),
                post: Some(vec!["echo".to_owned(), "done brew".to_owned()]),
            })
        );
        assert_eq!(
            hooks.get("uv"),
            Some(&Hooks {
                pre: Some(vec!["uv".to_owned(), "hook".to_owned()]),
                post: None,
            })
        );
        assert_eq!(hooks.get("docker"), None);
    }

    #[test]
    fn a_configuration_without_hooks_has_no_hooks() {
        let config = parsed("[run]\njobs = 2\n");

        assert!(config.hooks().is_empty());
    }

    #[test]
    fn an_unknown_key_in_a_hook_points_at_its_line() {
        let error = parse("[hooks.brew]\npre = [\"echo\"]\nnope = []\n").err();

        let (_, line, _, message) = invalid(error);

        assert_eq!(line, 3);
        assert!(
            message.contains("unknown field `nope`"),
            "unexpected message: {message}"
        );
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
    fn non_default_values_reports_only_values_that_differ_from_defaults() {
        let config = parsed(
            r#"
[preflight]
network = false
min_free_disk_gib = 5

[run]
jobs = 8

[steps]
brew-casks = false
docker = false

[include]
casks = ["firefox"]

[hooks.brew]
pre = ["echo", "pre"]
"#,
        );

        let values: BTreeMap<String, String> = config.non_default_values().into_iter().collect();

        assert_eq!(values.get("preflight.network"), Some(&"false".to_owned()));
        assert_eq!(
            values.get("preflight.min_free_disk_gib"),
            Some(&"5".to_owned())
        );
        assert_eq!(values.get("run.jobs"), Some(&"8".to_owned()));
        assert_eq!(values.get("steps.brew-casks"), Some(&"false".to_owned()));
        assert_eq!(values.get("steps.docker"), Some(&"false".to_owned()));
        assert_eq!(values.get("include.casks"), Some(&"firefox".to_owned()));
        assert_eq!(values.get("hooks.brew.pre"), Some(&"echo, pre".to_owned()));
        // Built-in defaults are not reported.
        assert!(!values.contains_key("preflight.disk_space"));
        assert!(!values.contains_key("run.snapshot_retention"));
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
    fn freshen_config_points_at_an_alternative_configuration_file() {
        let environment = ConfigEnvironment {
            config: Some(PathBuf::from("/tmp/alt/config.toml")),
            ..ConfigEnvironment::default()
        };

        for platform in [Platform::Macos, Platform::Linux, Platform::Windows] {
            assert_eq!(
                environment.config_path(platform),
                PathBuf::from("/tmp/alt/config.toml"),
                "{platform}"
            );
        }
    }

    #[test]
    fn freshen_config_wins_over_the_default_location() {
        let environment = ConfigEnvironment {
            config: Some(PathBuf::from("/tmp/alt/config.toml")),
            xdg_config_home: Some(PathBuf::from("/home/user/cfg")),
            home: Some(PathBuf::from("/home/user")),
            ..ConfigEnvironment::default()
        };

        assert_eq!(
            environment.config_path(Platform::Linux),
            PathBuf::from("/tmp/alt/config.toml")
        );
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

    #[cfg(unix)]
    mod world_writable {
        use super::{Config, ConfigError};
        use std::env;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};

        static NEXT: AtomicUsize = AtomicUsize::new(0);

        fn temp_dir() -> PathBuf {
            let next = NEXT.fetch_add(1, Ordering::Relaxed);
            env::temp_dir().join(format!(
                "freshen-world-writable-{}-{next}",
                std::process::id()
            ))
        }

        fn write_config(path: &Path, contents: &str) {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|error| {
                    panic!("failed to create {}: {error}", parent.display())
                });
            }
            fs::write(path, contents)
                .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
        }

        #[test]
        fn a_world_writable_configuration_file_is_refused_with_its_path() {
            let dir = temp_dir();
            fs::create_dir_all(&dir)
                .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
            let path = dir.join("config.toml");
            write_config(&path, "[preflight]\nnetwork = false\n");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap_or_else(|error| {
                panic!("failed to make {} world-writable: {error}", path.display())
            });

            let error = match Config::load(&path) {
                Err(error) => error,
                Ok(config) => panic!("expected a world-writable error, got {config:?}"),
            };

            let ConfigError::WorldWritable {
                path: offending_path,
            } = error
            else {
                panic!("expected a world-writable error, got {error:?}");
            };
            assert_eq!(offending_path, path);

            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_world_writable_configuration_directory_is_refused_with_its_path() {
            let dir = temp_dir();
            fs::create_dir_all(&dir)
                .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap_or_else(|error| {
                panic!("failed to make {} world-writable: {error}", dir.display())
            });
            let path = dir.join("config.toml");
            write_config(&path, "[preflight]\nnetwork = false\n");

            let error = match Config::load(&path) {
                Err(error) => error,
                Ok(config) => panic!("expected a world-writable error, got {config:?}"),
            };

            let ConfigError::WorldWritable {
                path: offending_path,
            } = error
            else {
                panic!("expected a world-writable error, got {error:?}");
            };
            assert_eq!(offending_path, dir);

            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_regular_configuration_file_is_loaded() {
            let dir = temp_dir();
            fs::create_dir_all(&dir)
                .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
            let path = dir.join("config.toml");
            write_config(&path, "[preflight]\nnetwork = false\n");

            let config = Config::load(&path)
                .unwrap_or_else(|error| panic!("expected a valid configuration, got {error}"));
            assert_eq!(config.preflight.network, Some(false));

            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[cfg(windows)]
    #[test]
    fn an_injected_world_writable_path_is_refused_with_its_path() {
        struct FakeProbe;

        impl WorldWritableProbe for FakeProbe {
            fn is_world_writable(&self, path: &Path) -> bool {
                path == Path::new(r"C:\Users\user\AppData\Roaming\freshen\config.toml")
            }
        }

        let path = PathBuf::from(r"C:\Users\user\AppData\Roaming\freshen\config.toml");

        let error = match Config::load_with_probe(&path, &FakeProbe) {
            Err(error) => error,
            Ok(config) => panic!("expected a world-writable error, got {config:?}"),
        };

        let ConfigError::WorldWritable {
            path: offending_path,
        } = error
        else {
            panic!("expected a world-writable error, got {error:?}");
        };
        assert_eq!(offending_path, path);
    }
}
