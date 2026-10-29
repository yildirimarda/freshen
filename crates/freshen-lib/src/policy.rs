//! The lists that decide what a run touches.
//!
//! Three kinds of lists steer a run:
//!
//! * include and exclude lists per kind of item (casks, packages, containers
//!   and compose projects),
//! * the `NEVER_QUIT` and `ALWAYS_QUIT` app lists, which decide which
//!   applications are quit around a cask upgrade,
//! * the per-step enable/disable flags, which live in
//!   [`crate::options::RunOptions`] because they are part of the run plan.
//!
//! The lists are data only: the steps decide what to do with them, so the
//! policy can be tested without touching the system.

use std::env;

/// Environment variable that lists the apps that are never quit.
pub const NEVER_QUIT_ENV: &str = "NEVER_QUIT";

/// Environment variable that lists the apps that are always quit.
pub const ALWAYS_QUIT_ENV: &str = "ALWAYS_QUIT";

/// An include list and an exclude list that decide which items a step acts
/// on.
///
/// The exclude list wins over the include list, and an empty include list
/// means "everything". A name that is in neither list always passes the
/// filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameFilter {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl NameFilter {
    /// Creates a filter that lets every name through.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the names that are let through.
    #[must_use]
    pub fn with_include(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.include = names.into_iter().map(Into::into).collect();
        self
    }

    /// Replaces the names that are kept out.
    #[must_use]
    pub fn with_exclude(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.exclude = names.into_iter().map(Into::into).collect();
        self
    }

    /// The names that are let through.
    #[must_use]
    pub fn include(&self) -> &[String] {
        &self.include
    }

    /// The names that are kept out.
    #[must_use]
    pub fn exclude(&self) -> &[String] {
        &self.exclude
    }

    /// Returns `true` when neither list names anything, so the filter does
    /// not change anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// Returns `true` when `name` passes the filter.
    ///
    /// Names are matched exactly, the way the package managers spell them.
    #[must_use]
    pub fn includes(&self, name: &str) -> bool {
        !self.is_excluded(name) && self.is_included(name)
    }

    /// Returns `true` when the include list keeps `name`.
    ///
    /// An empty include list keeps every name.
    #[must_use]
    pub fn is_included(&self, name: &str) -> bool {
        self.include.is_empty() || self.include.iter().any(|included| included == name)
    }

    /// Returns `true` when the exclude list names `name`.
    #[must_use]
    pub fn is_excluded(&self, name: &str) -> bool {
        self.exclude.iter().any(|excluded| excluded == name)
    }
}

/// The include and exclude lists of every kind of item a run can touch.
///
/// The lists are per kind, so a cask named in the exclude list does not
/// exclude a container with the same name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemFilters {
    casks: NameFilter,
    packages: NameFilter,
    containers: NameFilter,
    projects: NameFilter,
    appstore: NameFilter,
}

impl ItemFilters {
    /// Creates filters that let every item through.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the filter for casks.
    #[must_use]
    pub fn with_casks(mut self, filter: NameFilter) -> Self {
        self.casks = filter;
        self
    }

    /// Replaces the filter for packages.
    #[must_use]
    pub fn with_packages(mut self, filter: NameFilter) -> Self {
        self.packages = filter;
        self
    }

    /// Replaces the filter for containers.
    #[must_use]
    pub fn with_containers(mut self, filter: NameFilter) -> Self {
        self.containers = filter;
        self
    }

    /// Replaces the filter for compose projects.
    #[must_use]
    pub fn with_projects(mut self, filter: NameFilter) -> Self {
        self.projects = filter;
        self
    }

    /// Replaces the filter for Mac App Store apps.
    #[must_use]
    pub fn with_appstore(mut self, filter: NameFilter) -> Self {
        self.appstore = filter;
        self
    }

    /// The filter for casks.
    #[must_use]
    pub fn casks(&self) -> &NameFilter {
        &self.casks
    }

    /// The filter for packages.
    #[must_use]
    pub fn packages(&self) -> &NameFilter {
        &self.packages
    }

    /// The filter for standalone containers.
    #[must_use]
    pub fn containers(&self) -> &NameFilter {
        &self.containers
    }

    /// The filter for compose projects.
    #[must_use]
    pub fn projects(&self) -> &NameFilter {
        &self.projects
    }

    /// The filter for Mac App Store apps.
    #[must_use]
    pub fn appstore(&self) -> &NameFilter {
        &self.appstore
    }

    /// Returns `true` when no list names anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.casks.is_empty()
            && self.packages.is_empty()
            && self.containers.is_empty()
            && self.projects.is_empty()
            && self.appstore.is_empty()
    }
}

/// The `NEVER_QUIT` and `ALWAYS_QUIT` app lists.
///
/// An entry names an application by its bundle identifier (`com.docker`) or by
/// its name (`Docker` or `Docker.app`). The comparison ignores case and the
/// `.app` suffix, so `docker`, `Docker` and `Docker.app` all name the same
/// application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppLists {
    never_quit: Vec<String>,
    always_quit: Vec<String>,
}

impl AppLists {
    /// Creates empty lists.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the apps that are never quit.
    #[must_use]
    pub fn with_never_quit(mut self, apps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.never_quit = apps.into_iter().map(Into::into).collect();
        self
    }

    /// Replaces the apps that are always quit.
    #[must_use]
    pub fn with_always_quit(mut self, apps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.always_quit = apps.into_iter().map(Into::into).collect();
        self
    }

    /// The apps that are never quit.
    #[must_use]
    pub fn never_quit(&self) -> &[String] {
        &self.never_quit
    }

    /// The apps that are always quit.
    #[must_use]
    pub fn always_quit(&self) -> &[String] {
        &self.always_quit
    }

    /// Returns `true` when neither list names anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.never_quit.is_empty() && self.always_quit.is_empty()
    }

    /// Adds the apps of `other` to both lists.
    ///
    /// The environment variables add to the lists in the configuration file
    /// instead of replacing them, so a name that must never be quit cannot be
    /// dropped by accident.
    #[must_use]
    pub fn merged_with(mut self, other: Self) -> Self {
        for app in other.never_quit {
            if !self.never_quit.contains(&app) {
                self.never_quit.push(app);
            }
        }
        for app in other.always_quit {
            if !self.always_quit.contains(&app) {
                self.always_quit.push(app);
            }
        }
        self
    }

    /// Reads the `NEVER_QUIT` and `ALWAYS_QUIT` environment variables.
    ///
    /// Both hold a comma-separated list of app names.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_values(
            env::var(NEVER_QUIT_ENV).ok().as_deref(),
            env::var(ALWAYS_QUIT_ENV).ok().as_deref(),
        )
    }

    /// Reads the app lists from the two environment values.
    ///
    /// The values are comma-separated lists of app names; an absent value
    /// means the list is empty.
    #[must_use]
    pub fn from_values(never_quit: Option<&str>, always_quit: Option<&str>) -> Self {
        Self {
            never_quit: never_quit.map(parse_list).unwrap_or_default(),
            always_quit: always_quit.map(parse_list).unwrap_or_default(),
        }
    }

    /// Returns `true` when the app with `bundle_id` and `name` is never quit.
    #[must_use]
    pub fn is_never_quit(&self, bundle_id: &str, name: &str) -> bool {
        self.never_quit
            .iter()
            .any(|entry| matches_app(entry, bundle_id, name))
    }

    /// Returns `true` when the app with `bundle_id` and `name` is always quit.
    #[must_use]
    pub fn is_always_quit(&self, bundle_id: &str, name: &str) -> bool {
        self.always_quit
            .iter()
            .any(|entry| matches_app(entry, bundle_id, name))
    }
}

/// Splits a comma-separated list of names into its parts.
///
/// Empty parts are dropped, so a trailing comma does not add an empty name.
#[must_use]
pub fn parse_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Returns `true` when `entry` names the app with `bundle_id` and `name`.
fn matches_app(entry: &str, bundle_id: &str, name: &str) -> bool {
    let entry = entry.trim();
    if entry.is_empty() {
        return false;
    }

    if entry.eq_ignore_ascii_case(bundle_id) || entry.eq_ignore_ascii_case(name) {
        return true;
    }

    let stem = name.strip_suffix(".app").unwrap_or(name);
    entry.eq_ignore_ascii_case(stem)
}

#[cfg(test)]
mod tests {
    use super::{parse_list, AppLists, ItemFilters, NameFilter, ALWAYS_QUIT_ENV, NEVER_QUIT_ENV};

    #[test]
    fn an_empty_filter_lets_everything_through() {
        let filter = NameFilter::new();

        assert!(filter.is_empty());
        assert!(filter.includes("firefox"));
        assert!(filter.includes("anything"));
    }

    #[test]
    fn an_include_list_keeps_only_the_names_it_lists() {
        let filter = NameFilter::new().with_include(["firefox", "docker"]);

        assert!(!filter.is_empty());
        assert!(filter.includes("firefox"));
        assert!(filter.includes("docker"));
        assert!(!filter.includes("google-chrome"));
    }

    #[test]
    fn an_exclude_list_keeps_out_the_names_it_lists() {
        let filter = NameFilter::new().with_exclude(["docker"]);

        assert!(filter.includes("firefox"));
        assert!(!filter.includes("docker"));
    }

    #[test]
    fn the_exclude_list_wins_over_the_include_list() {
        let filter = NameFilter::new()
            .with_include(["firefox", "docker"])
            .with_exclude(["docker"]);

        assert!(filter.includes("firefox"));
        assert!(!filter.includes("docker"));
    }

    #[test]
    fn names_are_matched_exactly() {
        let filter = NameFilter::new().with_include(["Firefox"]);

        assert!(filter.includes("Firefox"));
        assert!(!filter.includes("firefox"));
    }

    #[test]
    fn the_filters_of_the_kinds_are_independent() {
        let filters = ItemFilters::new()
            .with_casks(NameFilter::new().with_include(["firefox"]))
            .with_packages(NameFilter::new().with_include(["node"]))
            .with_containers(NameFilter::new().with_include(["web"]))
            .with_projects(NameFilter::new().with_include(["stack"]));

        assert!(filters.casks().includes("firefox"));
        assert!(!filters.casks().includes("node"));
        assert!(filters.packages().includes("node"));
        assert!(!filters.packages().includes("firefox"));
        assert!(filters.containers().includes("web"));
        assert!(!filters.containers().includes("stack"));
        assert!(filters.projects().includes("stack"));
        assert!(!filters.projects().includes("web"));
    }

    #[test]
    fn empty_filters_change_nothing() {
        let filters = ItemFilters::new();

        assert!(filters.is_empty());
        assert!(filters.casks().includes("firefox"));
        assert!(filters.packages().includes("node"));
        assert!(filters.containers().includes("web"));
        assert!(filters.projects().includes("stack"));
    }

    #[test]
    fn an_app_is_matched_by_its_bundle_id_or_its_name() {
        let lists = AppLists::new().with_never_quit(["com.docker", "Firefox"]);

        assert!(lists.is_never_quit("com.docker", "Docker.app"));
        assert!(lists.is_never_quit("com.docker", "Something else.app"));
        assert!(lists.is_never_quit("com.other", "Firefox.app"));
        assert!(lists.is_never_quit("com.other", "Firefox"));
        assert!(!lists.is_never_quit("com.other", "Docker.app"));
    }

    #[test]
    fn an_app_name_is_matched_without_the_app_suffix_and_without_case() {
        let lists = AppLists::new().with_always_quit(["docker"]);

        assert!(lists.is_always_quit("com.docker", "Docker.app"));
        assert!(lists.is_always_quit("com.docker", "docker.app"));
        assert!(!lists.is_always_quit("com.docker", "OrbStack.app"));
    }

    #[test]
    fn the_two_app_lists_are_independent() {
        let lists = AppLists::new()
            .with_never_quit(["com.docker"])
            .with_always_quit(["com.spotify"]);

        assert!(lists.is_never_quit("com.docker", "Docker.app"));
        assert!(!lists.is_always_quit("com.docker", "Docker.app"));
        assert!(lists.is_always_quit("com.spotify", "Spotify.app"));
        assert!(!lists.is_never_quit("com.spotify", "Spotify.app"));
    }

    #[test]
    fn merging_adds_the_names_of_both_lists() {
        let lists = AppLists::new()
            .with_never_quit(["com.docker"])
            .merged_with(AppLists::new().with_never_quit(["com.docker", "com.orbstack"]));

        assert_eq!(lists.never_quit(), ["com.docker", "com.orbstack"]);
    }

    #[test]
    fn a_comma_separated_list_is_split_into_its_parts() {
        assert_eq!(parse_list("a, b ,c"), ["a", "b", "c"]);
        assert_eq!(parse_list("a,,b,"), ["a", "b"]);
        assert!(parse_list("").is_empty());
    }

    #[test]
    fn the_environment_variables_name_the_app_lists() {
        assert_eq!(NEVER_QUIT_ENV, "NEVER_QUIT");
        assert_eq!(ALWAYS_QUIT_ENV, "ALWAYS_QUIT");
    }

    #[test]
    fn the_environment_values_are_read_as_comma_separated_lists() {
        let lists = AppLists::from_values(Some("com.docker, com.orbstack"), Some("com.spotify"));

        assert_eq!(lists.never_quit(), ["com.docker", "com.orbstack"]);
        assert_eq!(lists.always_quit(), ["com.spotify"]);
    }

    #[test]
    fn absent_environment_values_leave_the_lists_empty() {
        let lists = AppLists::from_values(None, None);

        assert!(lists.is_empty());
    }
}
