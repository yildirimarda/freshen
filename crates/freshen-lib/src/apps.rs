//! Quitting and reopening macOS applications around a cask upgrade.
//!
//! The zsh implementation reads the bundle id of every installed cask app,
//! quits the ones that are running, waits for them to exit and reopens exactly
//! those apps afterwards. It never quits the terminal that is running the
//! script, and only force-kills an app when the user asked for it.
//!
//! All operating-system access goes through [`AppSystem`], so the policy is
//! tested with a fake process table instead of the real machine.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::output::human;
use crate::policy::AppLists;
use crate::process::CommandSpec;

/// Seconds `osascript` waits for an app before the quit command gives up.
const APPLESCRIPT_TIMEOUT_SECS: u64 = 3;

/// The error number macOS reports for a denied Automation request.
const AUTOMATION_DENIED_ERROR: &str = "-1743";

/// Default seconds to wait for an app to quit, matching `QUIT_TIMEOUT`.
const DEFAULT_QUIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Default time between checks while waiting for an app to quit.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Default wait after a force kill, matching the zsh script.
const DEFAULT_FORCE_KILL_GRACE: Duration = Duration::from_secs(2);

/// Default seconds to wait for an app that is updating itself.
const DEFAULT_UPDATE_TIMEOUT: Duration = Duration::from_secs(60);

/// How many times the window of a reopened app is put back where it was.
///
/// An app needs a moment to create its window after it was opened, so the
/// move is retried before it is given up on.
const RESTORE_ATTEMPTS: u32 = 3;

/// The wait between two attempts to put a window back.
const RESTORE_RETRY_DELAY: Duration = Duration::from_millis(500);

/// How long the AppleScript that reads the state of an app may take.
///
/// macOS answers at once, but the command can wait for a permission dialog
/// that nobody answers, and the run must not hang on it.
const SESSION_STATE_TIMEOUT: Duration = Duration::from_secs(10);

/// An application that is running.
///
/// Most apps are known by their `.app` bundle path. A cask without an `.app`
/// artifact, such as a pkg installer or a menu-bar helper, is known only by
/// the bundle id in its `uninstall` stanza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningApp {
    path: Option<PathBuf>,
    bundle_id: String,
    name: String,
}

impl RunningApp {
    /// Creates a running app from its bundle path and bundle identifier.
    #[must_use]
    pub fn new(path: PathBuf, bundle_id: impl Into<String>) -> Self {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        Self {
            path: Some(path),
            bundle_id: bundle_id.into(),
            name,
        }
    }

    /// Creates a running app known only by its bundle identifier.
    ///
    /// Used for casks that install something other than an `.app` bundle, so
    /// there is no path to search for or inspect.
    #[must_use]
    pub fn from_bundle_id(bundle_id: impl Into<String>) -> Self {
        let bundle_id = bundle_id.into();
        Self {
            path: None,
            name: bundle_id.clone(),
            bundle_id,
        }
    }

    /// Path of the `.app` bundle, when the app has one.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Bundle identifier used for the AppleScript quit and the reopen.
    #[must_use]
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    /// File name shown in the "Quitting:" line, e.g. `Firefox.app`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    fn is_running<S: AppSystem + ?Sized>(&self, system: &S) -> bool {
        match self.path() {
            Some(path) => system.is_running(path),
            None => system.is_bundle_running(&self.bundle_id),
        }
    }

    fn pids<S: AppSystem + ?Sized>(&self, system: &S) -> Vec<u32> {
        match self.path() {
            Some(path) => system.pids(path),
            None => system.bundle_pids(&self.bundle_id),
        }
    }

    /// What the app is doing about its own updates.
    fn update_state<S: AppSystem + ?Sized>(&self, system: &S) -> UpdateState {
        system.update_state(self.path(), &self.bundle_id)
    }
}

/// The place of an app's window on the screen.
///
/// macOS only puts a window back where it was for the apps that save their
/// own window state; the other apps come back in the middle of the main
/// display. Remembering the place lets freshen put the window back on the
/// display the app was on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    position: (i32, i32),
    size: (u32, u32),
}

impl WindowState {
    /// Creates a window state from the position and the size of a window.
    #[must_use]
    pub fn new(position: (i32, i32), size: (u32, u32)) -> Self {
        Self { position, size }
    }

    /// The position of the window on the screen.
    ///
    /// The position is negative when the window is on a display that is left
    /// of or above the main display.
    #[must_use]
    pub fn position(&self) -> (i32, i32) {
        self.position
    }

    /// The size of the window.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
}

/// How an app looked before it was quit.
///
/// macOS does not bring an app back the way it was: an app that was hidden
/// comes back visible, and a window that was on a second display comes back
/// on the main one. freshen reads the state of an app before it quits it and
/// restores it when the app is reopened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionState {
    hidden: bool,
    window: Option<WindowState>,
}

impl SessionState {
    /// The state of an app that is visible and keeps no window.
    #[must_use]
    pub fn visible() -> Self {
        Self::default()
    }

    /// The state of an app that was hidden.
    #[must_use]
    pub fn hidden() -> Self {
        Self {
            hidden: true,
            window: None,
        }
    }

    /// Records the place of the app's window.
    #[must_use]
    pub fn with_window(mut self, window: WindowState) -> Self {
        self.window = Some(window);
        self
    }

    /// Returns `true` when the app was hidden.
    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// The place of the app's window, when it had one.
    #[must_use]
    pub fn window(&self) -> Option<WindowState> {
        self.window
    }
}

/// An app that was quit and has to be reopened.
///
/// The bundle id is what `open` needs; the session state is how the app
/// looked before the quit, so it comes back the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reopen {
    bundle_id: String,
    session: SessionState,
}

impl Reopen {
    /// Creates a reopen entry for an app.
    #[must_use]
    pub fn new(bundle_id: impl Into<String>, session: SessionState) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            session,
        }
    }

    /// The bundle id of the app.
    #[must_use]
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    /// How the app looked before it was quit.
    #[must_use]
    pub fn session(&self) -> SessionState {
        self.session
    }
}

/// The framework an app uses to update itself.
///
/// An app that updates itself replaces its own bundle while it runs, so
/// quitting it in the middle of that leaves a half-updated app behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfUpdater {
    /// The Sparkle framework, used by many native macOS apps.
    Sparkle,
    /// The updater of an Electron app, which installs the new version with a
    /// `ShipIt` helper.
    Electron,
}

impl SelfUpdater {
    /// The name of the updater, as shown to the user.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SelfUpdater::Sparkle => "Sparkle",
            SelfUpdater::Electron => "Electron auto-update",
        }
    }
}

/// What an app is doing about its own updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpdateState {
    updater: Option<SelfUpdater>,
    in_progress: bool,
}

impl UpdateState {
    /// The app does not update itself.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The app updates itself, but not at the moment.
    #[must_use]
    pub fn idle(updater: SelfUpdater) -> Self {
        Self {
            updater: Some(updater),
            in_progress: false,
        }
    }

    /// The app is updating itself right now.
    #[must_use]
    pub fn updating(updater: SelfUpdater) -> Self {
        Self {
            updater: Some(updater),
            in_progress: true,
        }
    }

    /// The updater the app uses, when it updates itself.
    #[must_use]
    pub fn updater(self) -> Option<SelfUpdater> {
        self.updater
    }

    /// Returns `true` when the app is updating itself right now.
    #[must_use]
    pub fn in_progress(self) -> bool {
        self.in_progress
    }
}

/// The result of inspecting the apps a cask upgrade will replace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppScan {
    running: Vec<RunningApp>,
    protected: Vec<String>,
    never_quit: Vec<String>,
    updating: Vec<RunningApp>,
}

impl AppScan {
    /// Installed apps that are currently running and may be quit.
    #[must_use]
    pub fn running(&self) -> &[RunningApp] {
        &self.running
    }

    /// Installed apps that were not quit because they are the host terminal.
    #[must_use]
    pub fn protected(&self) -> &[String] {
        &self.protected
    }

    /// Installed apps that were not quit because they are on the never-quit
    /// list.
    #[must_use]
    pub fn never_quit(&self) -> &[String] {
        &self.never_quit
    }

    /// Installed apps that are updating themselves right now.
    ///
    /// These apps are waited for instead of being quit, because quitting them
    /// in the middle of their own update would leave a broken app behind.
    #[must_use]
    pub fn updating(&self) -> &[RunningApp] {
        &self.updating
    }

    /// Returns `true` when there is nothing running to quit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    /// Creates a scan that holds the given running apps.
    #[cfg(test)]
    #[must_use]
    pub fn with_running(running: Vec<RunningApp>) -> Self {
        Self {
            running,
            protected: Vec::new(),
            never_quit: Vec::new(),
            updating: Vec::new(),
        }
    }
}

/// How long to wait for a graceful quit and whether a stuck app may be killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuitPolicy {
    timeout: Duration,
    poll_interval: Duration,
    force_kill_grace: Duration,
    force_kill: bool,
    update_timeout: Duration,
}

impl QuitPolicy {
    /// Creates a policy with the default poll interval and force-kill grace.
    #[must_use]
    pub fn new(timeout: Duration, force_kill: bool) -> Self {
        Self {
            timeout,
            poll_interval: DEFAULT_POLL_INTERVAL,
            force_kill_grace: DEFAULT_FORCE_KILL_GRACE,
            force_kill,
            update_timeout: DEFAULT_UPDATE_TIMEOUT,
        }
    }

    /// Reads `QUIT_TIMEOUT` (seconds) like the zsh script, falling back to 20.
    ///
    /// `UPDATE_TIMEOUT` (seconds) bounds the wait for an app that is updating
    /// itself, and falls back to 60.
    #[must_use]
    pub fn from_env(force_kill: bool) -> Self {
        let timeout = env::var("QUIT_TIMEOUT")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_QUIT_TIMEOUT);
        let update_timeout = env::var("UPDATE_TIMEOUT")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_UPDATE_TIMEOUT);
        Self::new(timeout, force_kill).with_update_timeout(update_timeout)
    }

    /// Replaces the interval between two "is it still running?" checks.
    #[must_use]
    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = if poll_interval.is_zero() {
            Duration::from_millis(1)
        } else {
            poll_interval
        };
        self
    }

    /// Replaces the wait after a force kill.
    #[must_use]
    pub fn with_force_kill_grace(mut self, grace: Duration) -> Self {
        self.force_kill_grace = grace;
        self
    }

    /// Replaces the wait for an app that is updating itself.
    #[must_use]
    pub fn with_update_timeout(mut self, timeout: Duration) -> Self {
        self.update_timeout = timeout;
        self
    }

    /// How long to wait for a graceful quit.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// How long to wait for an app that is updating itself.
    #[must_use]
    pub fn update_timeout(&self) -> Duration {
        self.update_timeout
    }

    /// Returns `true` when an app that does not quit may be killed.
    #[must_use]
    pub fn force_kill(&self) -> bool {
        self.force_kill
    }
}

/// The answer to the "quit these apps?" prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitChoice {
    /// Quit the apps and upgrade the casks.
    Quit,
    /// Upgrade the casks without quitting anything.
    SkipQuit,
    /// Skip the cask upgrade entirely.
    SkipCasks,
}

/// Parses a prompt answer with the same choices as the zsh script.
///
/// An empty answer means yes, as pressing Return does in `update-all.sh`.
#[must_use]
pub fn quit_choice(answer: &str) -> QuitChoice {
    match answer.trim().to_ascii_lowercase().as_str() {
        "" | "y" | "yes" => QuitChoice::Quit,
        "s" | "skip" => QuitChoice::SkipQuit,
        _ => QuitChoice::SkipCasks,
    }
}

/// What happened when an app was asked to quit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuitOutcome {
    /// The quit request was delivered to the app.
    #[default]
    Quit,
    /// macOS denied the request to control the app.
    ///
    /// The Automation permission was never granted, so the app never saw the
    /// request. This is a missing permission, not an app that refuses to quit.
    Denied,
    /// The quit request failed for another reason.
    Failed,
}

/// Returns `true` when the `osascript` output describes a denied Automation
/// request.
///
/// macOS refuses to send Apple events to an app the user has not allowed, and
/// reports it as error -1743. The message also names the permission, which
/// is what a localized or a rewritten error may keep.
#[must_use]
pub fn is_automation_denied(output: &str) -> bool {
    let text = output.to_ascii_lowercase();

    text.contains(AUTOMATION_DENIED_ERROR) || text.contains("not authorized to send apple event")
}

/// The message shown when macOS denied the request to control an app.
///
/// The fix is a permission in System Settings, so the message names the pane
/// instead of reporting the app as one that cannot be quit.
#[must_use]
pub fn automation_denied_message(app_name: &str) -> String {
    format!(
        "{app_name} could not be quit: macOS denied the request to control it (error -1743). \
         Allow the terminal that runs freshen to control it in \
         System Settings > Privacy & Security > Automation, then run freshen again."
    )
}

/// The operating-system operations the quit/reopen policy needs.
///
/// The real implementation is [`MacAppSystem`]. Tests provide a fake process
/// table and record the quit, kill and open calls.
pub trait AppSystem: Send + Sync {
    /// Reads `CFBundleIdentifier` from the bundle's `Contents/Info`.
    fn bundle_id(&self, app: &Path) -> Option<String>;

    /// Returns `true` when the bundle's main executable is running.
    fn is_running(&self, app: &Path) -> bool;

    /// Pids of processes running the bundle's main executable.
    fn pids(&self, app: &Path) -> Vec<u32>;

    /// Returns `true` when an app with this bundle id is running.
    ///
    /// Casks without an `.app` artifact are known only by the bundle id in
    /// their `uninstall` stanza, so there is no path to inspect.
    fn is_bundle_running(&self, _bundle_id: &str) -> bool {
        false
    }

    /// Pids of the processes with this bundle id, for a force kill.
    fn bundle_pids(&self, _bundle_id: &str) -> Vec<u32> {
        Vec::new()
    }

    /// Asks the application to quit gracefully.
    ///
    /// The outcome says whether the request was delivered or denied by
    /// macOS, so a denial can be explained instead of being reported as an
    /// app that does not quit.
    fn quit(&self, bundle_id: &str) -> QuitOutcome;

    /// Force-kills the given pids.
    fn force_kill(&self, pids: &[u32]);

    /// Reads how the app looks right now.
    ///
    /// The state says whether the app is hidden and where its window is, so
    /// the app can come back the way it was after the upgrade. An
    /// implementation that cannot read the state reports a visible app
    /// without a window.
    fn session_state(&self, _app: Option<&Path>, _bundle_id: &str) -> SessionState {
        SessionState::visible()
    }

    /// Reopens the application with `open -g -b`.
    ///
    /// An app that was hidden is reopened hidden, so it does not show up in
    /// front of the user after the upgrade.
    fn open(&self, bundle_id: &str, hidden: bool) -> bool;

    /// Puts the window of the app back where it was.
    ///
    /// Returns `true` when the window was moved.
    fn restore_window(&self, _bundle_id: &str, _window: &WindowState) -> bool {
        false
    }

    /// What the app is doing about its own updates.
    ///
    /// `app` is the path of the bundle when the app has one; an app that is
    /// known only by its bundle id is inspected through that id alone.
    fn update_state(&self, _app: Option<&Path>, _bundle_id: &str) -> UpdateState {
        UpdateState::none()
    }
}

/// The updater an app uses, detected from the files in its bundle.
///
/// Sparkle apps ship the framework in `Contents/Frameworks`, and an Electron
/// app that updates itself ships `app-update.yml` in `Contents/Resources`.
#[must_use]
pub fn self_updater_from_bundle(app: &Path) -> Option<SelfUpdater> {
    if app.join("Contents/Frameworks/Sparkle.framework").exists() {
        Some(SelfUpdater::Sparkle)
    } else if app.join("Contents/Resources/app-update.yml").exists()
        || app.join("Contents/Frameworks/Squirrel.framework").exists()
    {
        Some(SelfUpdater::Electron)
    } else {
        None
    }
}

/// The updater that is running for an app, when one is.
///
/// Sparkle runs its updater from inside the app bundle, so the command line
/// of the updater names the Sparkle framework of that app. The updater of an
/// Electron app runs a `ShipIt` helper whose path carries the bundle id of
/// the app it updates.
///
/// The lines are the output of `ps -axo command=`, which holds the command
/// line without a pid.
#[must_use]
pub fn running_updater(
    processes: &str,
    app: Option<&Path>,
    bundle_id: &str,
) -> Option<SelfUpdater> {
    let sparkle = app.map(|app| format!("{}/Contents/Frameworks/Sparkle.framework", app.display()));
    let electron = (!bundle_id.is_empty()).then_some(bundle_id);

    for line in processes.lines() {
        let command = line.trim_start();
        if sparkle
            .as_deref()
            .is_some_and(|needle| command.contains(needle))
        {
            return Some(SelfUpdater::Sparkle);
        }
        if electron.is_some_and(|bundle_id| {
            command.to_ascii_lowercase().contains("shipit") && command.contains(bundle_id)
        }) {
            return Some(SelfUpdater::Electron);
        }
    }

    None
}

/// Finds the running apps among `installed` bundles.
///
/// Apps without a bundle id are ignored, an app whose bundle id matches
/// `self_bundle_id` is reported in [`AppScan::protected`] instead of being
/// considered for quitting, and an app on the never-quit list is reported in
/// [`AppScan::never_quit`].
#[must_use]
pub fn find_running_apps<S: AppSystem + ?Sized>(
    system: &S,
    installed: &[PathBuf],
    self_bundle_id: Option<&str>,
    app_lists: &AppLists,
) -> AppScan {
    find_running_cask_apps(system, installed, &[], self_bundle_id, app_lists)
}

/// Finds the running apps among `installed` bundles and `bundle_ids`.
///
/// The bundle ids come from a cask's `uninstall` stanza, so a cask whose
/// artifacts are not an `.app` bundle (a pkg installer or a menu-bar helper)
/// still has its app quit before the upgrade. An app found through its path is
/// not added a second time when the same bundle id is also listed.
#[must_use]
pub fn find_running_cask_apps<S: AppSystem + ?Sized>(
    system: &S,
    installed: &[PathBuf],
    bundle_ids: &[String],
    self_bundle_id: Option<&str>,
    app_lists: &AppLists,
) -> AppScan {
    let mut scan = AppScan::default();
    let mut seen = Vec::new();

    for path in installed {
        let Some(bundle_id) = system.bundle_id(path) else {
            continue;
        };
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        seen.push(bundle_id.clone());
        if self_bundle_id.is_some_and(|self_id| self_id == bundle_id) {
            scan.protected.push(name);
            continue;
        }
        if !system.is_running(path) {
            continue;
        }
        if app_lists.is_never_quit(&bundle_id, &name) {
            scan.never_quit.push(name);
            continue;
        }
        let app = RunningApp::new(path.clone(), bundle_id);
        if app.update_state(system).in_progress() {
            scan.updating.push(app.clone());
        }
        scan.running.push(app);
    }

    for bundle_id in bundle_ids {
        if seen.iter().any(|seen_id| seen_id == bundle_id) {
            continue;
        }
        seen.push(bundle_id.clone());
        if self_bundle_id.is_some_and(|self_id| self_id == bundle_id) {
            scan.protected.push(bundle_id.clone());
            continue;
        }
        if !system.is_bundle_running(bundle_id) {
            continue;
        }
        if app_lists.is_never_quit(bundle_id, bundle_id) {
            scan.never_quit.push(bundle_id.clone());
            continue;
        }
        let app = RunningApp::from_bundle_id(bundle_id.clone());
        if app.update_state(system).in_progress() {
            scan.updating.push(app.clone());
        }
        scan.running.push(app);
    }

    scan
}

/// Quits every app gracefully and returns the apps to reopen.
///
/// A bundle id is recorded before the quit is attempted, exactly like the zsh
/// script, so an app that refuses to quit is still reopened (it never closed).
/// The session state of the app is read before the quit, so it can be
/// restored when the app comes back.
///
/// An app that is updating itself is waited for instead of being quit, and is
/// left alone when its update does not finish in time.
#[must_use]
pub fn quit_apps<S: AppSystem + ?Sized>(
    system: &S,
    apps: &[RunningApp],
    policy: &QuitPolicy,
) -> Vec<Reopen> {
    let mut reopen = Vec::with_capacity(apps.len());

    for app in apps {
        if !wait_for_self_update(system, app, policy) {
            continue;
        }
        human!("Quitting: {}", app.name());
        let session = system.session_state(app.path(), app.bundle_id());
        let quit = quit_one(system, app, policy);
        // An app that is still running keeps its own window and its own
        // hidden state, so it is reopened the way it is.
        let session = if quit {
            session
        } else {
            SessionState::visible()
        };
        reopen.push(Reopen::new(app.bundle_id(), session));
    }

    reopen
}

/// Waits for an app that is updating itself to finish its update.
///
/// An app that updates itself (Sparkle, or the updater of an Electron app)
/// replaces its own bundle while it runs. Quitting it in the middle of that
/// leaves a half-updated app behind, so freshen waits for the update to
/// finish instead of quitting the app.
///
/// Returns `true` when the app is not updating anymore and may be quit, and
/// `false` when the update did not finish in time. An app that was not quit is
/// not reopened either, because it never closed.
pub fn wait_for_self_update<S: AppSystem + ?Sized>(
    system: &S,
    app: &RunningApp,
    policy: &QuitPolicy,
) -> bool {
    wait_for_self_update_with(system, app, policy, crate::signals::requested)
}

/// Waits for an app that is updating itself, stopping early on an interrupt.
fn wait_for_self_update_with<S: AppSystem + ?Sized, F>(
    system: &S,
    app: &RunningApp,
    policy: &QuitPolicy,
    interrupt: F,
) -> bool
where
    F: Fn() -> bool,
{
    let mut waited = Duration::ZERO;
    let mut announced = false;

    loop {
        let state = app.update_state(system);
        if !state.in_progress() {
            return true;
        }

        if !announced {
            announced = true;
            let updater = state.updater().map_or("its own updater", SelfUpdater::name);
            human!(
                "Waiting up to {}s for {} to finish updating itself ({updater})...",
                policy.update_timeout().as_secs(),
                app.name()
            );
        }

        if interrupt() {
            warn(&format!(
                "interrupted while waiting for {} to finish updating itself, so it was not quit.",
                app.name()
            ));
            return false;
        }

        if waited >= policy.update_timeout() {
            warn(&format!(
                "{} is updating itself and did not finish, so it was not quit.",
                app.name()
            ));
            return false;
        }

        std::thread::sleep(policy.poll_interval);
        waited += policy.poll_interval;
    }
}

/// Reopens the apps that were quit, restoring how they looked before.
///
/// An app that was hidden is reopened hidden, and the window of an app that
/// was visible is put back where it was, so the app comes back on the
/// display it was on.
pub fn reopen_apps<S: AppSystem + ?Sized>(system: &S, apps: &[Reopen]) {
    for app in apps {
        let bundle_id = app.bundle_id();
        let session = app.session();

        if session.is_hidden() {
            human!("Opening: {bundle_id} (hidden)");
        } else {
            human!("Opening: {bundle_id}");
        }

        if !system.open(bundle_id, session.is_hidden()) {
            warn(&format!("Could not open: {bundle_id}"));
            continue;
        }

        if session.is_hidden() {
            continue;
        }

        if let Some(window) = session.window() {
            if !system.restore_window(bundle_id, &window) {
                warn(&format!(
                    "Could not put the window of {bundle_id} back where it was."
                ));
            }
        }
    }
}

fn quit_one<S: AppSystem + ?Sized>(system: &S, app: &RunningApp, policy: &QuitPolicy) -> bool {
    if system.quit(app.bundle_id()) == QuitOutcome::Denied {
        // The request never reached the app, so waiting for it to exit would
        // only burn the timeout.
        warn(&automation_denied_message(app.name()));
        if policy.force_kill() {
            kill_app(system, app, policy);
        }
        return false;
    }

    let mut running = app.is_running(system);
    let mut waited = Duration::ZERO;
    while running && waited < policy.timeout {
        std::thread::sleep(policy.poll_interval);
        waited += policy.poll_interval;
        running = app.is_running(system);
    }

    if !running {
        return true;
    }

    if policy.force_kill {
        kill_app(system, app, policy);
        return true;
    }

    warn(&format!(
        "{} did not quit (use --force-kill to force). Upgrade will be attempted anyway.",
        app.name
    ));
    false
}

/// Kills the app's main executable and waits for it to disappear.
fn kill_app<S: AppSystem + ?Sized>(system: &S, app: &RunningApp, policy: &QuitPolicy) {
    warn(&format!("{} did not quit, force-killing", app.name));
    let pids = app.pids(system);
    if !pids.is_empty() {
        system.force_kill(&pids);
        std::thread::sleep(policy.force_kill_grace);
    }
}

fn warn(message: &str) {
    human!("warning: {message}");
}

fn executable_prefix(app: &Path) -> String {
    format!("{}/Contents/MacOS/", app.display())
}

/// Pids whose command line runs `app`'s main executable.
///
/// Helper and agent processes that share the bundle id but live in another
/// executable are not matched, which is why the zsh script checks
/// `Contents/MacOS/` instead of AppleScript's "is running".
#[must_use]
pub fn pids_from_processes(processes: &str, app: &Path) -> Vec<u32> {
    let needle = executable_prefix(app);
    processes
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let mut fields = line.splitn(2, char::is_whitespace);
            let pid = fields.next()?.parse::<u32>().ok()?;
            let command = fields.next().unwrap_or_default();
            command.contains(&needle).then_some(pid)
        })
        .collect()
}

/// Pids from AppleScript's list output, such as `123, 456`.
///
/// System Events prints the unix ids separated by commas, and an app that is
/// not running prints `missing value`.
#[must_use]
pub fn pids_from_apple_script(output: &str) -> Vec<u32> {
    output
        .split(|character: char| !character.is_ascii_digit())
        .filter(|token| !token.is_empty())
        .filter_map(|token| token.parse().ok())
        .collect()
}

/// The hidden state in the output of the System Events query.
///
/// The output is `true` or `false`, and anything else means the state could
/// not be read.
#[must_use]
pub fn hidden_from_script(output: &str) -> Option<bool> {
    match output.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// The place of a window in the output of the System Events query.
///
/// The output looks like `{{0, 0}, {1440, 900}}`: the position of the window
/// and its size. A window on a second display can have a negative position.
#[must_use]
pub fn window_state_from_script(output: &str) -> Option<WindowState> {
    let numbers = script_numbers(output);
    match numbers.as_slice() {
        [x, y, width, height] => Some(WindowState::new(
            (i32::try_from(*x).ok()?, i32::try_from(*y).ok()?),
            (u32::try_from(*width).ok()?, u32::try_from(*height).ok()?),
        )),
        _ => None,
    }
}

/// The numbers in the output of an AppleScript command.
fn script_numbers(output: &str) -> Vec<i64> {
    output
        .split(|character: char| !character.is_ascii_digit() && character != '-')
        .filter(|token| !token.is_empty() && *token != "-")
        .filter_map(|token| token.parse().ok())
        .collect()
}

fn bundle_id_command(app: &Path) -> CommandSpec {
    CommandSpec::new("defaults")
        .arg("read")
        .arg(app.join("Contents/Info"))
        .arg("CFBundleIdentifier")
        .neutral_locale()
}

fn process_list_command() -> CommandSpec {
    CommandSpec::new("ps")
        .args(["-axo", "command="])
        .neutral_locale()
}

fn pid_list_command() -> CommandSpec {
    CommandSpec::new("ps")
        .args(["-axo", "pid=,command="])
        .neutral_locale()
}

fn bundle_running_command(bundle_id: &str) -> CommandSpec {
    CommandSpec::new("osascript")
        .arg("-e")
        .arg(format!("application id \"{bundle_id}\" is running"))
        .neutral_locale()
}

fn bundle_pids_command(bundle_id: &str) -> CommandSpec {
    CommandSpec::new("osascript")
        .arg("-e")
        .arg(format!(
            "tell application \"System Events\" to get unix id of \
             (every process whose bundle identifier is \"{bundle_id}\")"
        ))
        .neutral_locale()
}

/// The AppleScript that tells whether the app is hidden.
///
/// `AXHidden` is the attribute macOS uses for an app that was hidden, and it
/// is read through System Events because an app cannot report it by itself.
fn hidden_command(bundle_id: &str) -> CommandSpec {
    CommandSpec::new("osascript")
        .arg("-e")
        .arg("tell application \"System Events\"")
        .arg("-e")
        .arg(format!(
            "set appProcess to first process whose bundle identifier is \"{bundle_id}\""
        ))
        .arg("-e")
        .arg("get value of attribute \"AXHidden\" of appProcess")
        .arg("-e")
        .arg("end tell")
        .neutral_locale()
        .timeout(SESSION_STATE_TIMEOUT)
}

/// The AppleScript that reads the place of the window of the app.
///
/// The first window of the app is used, because an app that is reopened comes
/// back with a window in the middle of the main display.
fn window_state_command(bundle_id: &str) -> CommandSpec {
    CommandSpec::new("osascript")
        .arg("-e")
        .arg("tell application \"System Events\"")
        .arg("-e")
        .arg(format!(
            "set appProcess to first process whose bundle identifier is \"{bundle_id}\""
        ))
        .arg("-e")
        .arg("tell appProcess")
        .arg("-e")
        .arg("get {position of window 1, size of window 1}")
        .arg("-e")
        .arg("end tell")
        .arg("-e")
        .arg("end tell")
        .neutral_locale()
        .timeout(SESSION_STATE_TIMEOUT)
}

/// The AppleScript that puts the window of the app back where it was.
fn restore_window_command(bundle_id: &str, window: &WindowState) -> CommandSpec {
    let (x, y) = window.position();
    let (width, height) = window.size();
    CommandSpec::new("osascript")
        .arg("-e")
        .arg("tell application \"System Events\"")
        .arg("-e")
        .arg(format!(
            "set appProcess to first process whose bundle identifier is \"{bundle_id}\""
        ))
        .arg("-e")
        .arg("tell appProcess")
        .arg("-e")
        .arg(format!("set position of window 1 to {{{x}, {y}}}"))
        .arg("-e")
        .arg(format!("set size of window 1 to {{{width}, {height}}}"))
        .arg("-e")
        .arg("end tell")
        .arg("-e")
        .arg("end tell")
        .neutral_locale()
        .timeout(SESSION_STATE_TIMEOUT)
}

fn quit_command(bundle_id: &str) -> CommandSpec {
    CommandSpec::new("osascript")
        .arg("-e")
        .arg(format!(
            "with timeout of {APPLESCRIPT_TIMEOUT_SECS} seconds"
        ))
        .arg("-e")
        .arg(format!("tell application id \"{bundle_id}\" to quit"))
        .arg("-e")
        .arg("end timeout")
}

fn kill_command(pids: &[u32]) -> CommandSpec {
    let mut spec = CommandSpec::new("kill").arg("-9");
    for pid in pids {
        spec = spec.arg(pid.to_string());
    }
    spec
}

fn open_command(bundle_id: &str, hidden: bool) -> CommandSpec {
    let mut spec = CommandSpec::new("open").arg("-g");
    if hidden {
        spec = spec.arg("-j");
    }
    spec.args(["-b"]).arg(bundle_id)
}

/// The real macOS implementation, using the same commands as the zsh script.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacAppSystem;

impl MacAppSystem {
    fn process_list(&self) -> String {
        process_list_command()
            .run()
            .map(|output| output.stdout().to_owned())
            .unwrap_or_default()
    }

    /// Whether the app is hidden right now.
    ///
    /// A state that cannot be read (a denied permission, or an app that does
    /// not answer) counts as visible, so the app is reopened as it was before
    /// the upgrade rather than staying hidden by accident.
    fn is_hidden(&self, bundle_id: &str) -> bool {
        hidden_command(bundle_id)
            .run()
            .ok()
            .and_then(|output| hidden_from_script(output.stdout()))
            .unwrap_or(false)
    }

    /// Where the window of the app is right now.
    fn window_state(&self, bundle_id: &str) -> Option<WindowState> {
        window_state_command(bundle_id)
            .run()
            .ok()
            .and_then(|output| window_state_from_script(output.stdout()))
    }
}

impl AppSystem for MacAppSystem {
    fn bundle_id(&self, app: &Path) -> Option<String> {
        let output = bundle_id_command(app).run().ok()?;
        let bundle_id = output.stdout().trim();
        if bundle_id.is_empty() {
            None
        } else {
            Some(bundle_id.to_owned())
        }
    }

    fn is_running(&self, app: &Path) -> bool {
        let needle = executable_prefix(app);
        self.process_list().contains(&needle)
    }

    fn pids(&self, app: &Path) -> Vec<u32> {
        let output = pid_list_command().run();
        match output {
            Ok(output) => pids_from_processes(output.stdout(), app),
            Err(_) => Vec::new(),
        }
    }

    fn is_bundle_running(&self, bundle_id: &str) -> bool {
        bundle_running_command(bundle_id)
            .run()
            .is_ok_and(|output| output.stdout().trim().eq_ignore_ascii_case("true"))
    }

    fn bundle_pids(&self, bundle_id: &str) -> Vec<u32> {
        match bundle_pids_command(bundle_id).run() {
            Ok(output) => pids_from_apple_script(output.stdout()),
            Err(_) => Vec::new(),
        }
    }

    fn quit(&self, bundle_id: &str) -> QuitOutcome {
        match quit_command(bundle_id).run() {
            Ok(_) => QuitOutcome::Quit,
            Err(error) => {
                let output = error
                    .output()
                    .map(|output| format!("{}{}", output.stdout(), output.stderr()))
                    .unwrap_or_default();
                if is_automation_denied(&output) {
                    QuitOutcome::Denied
                } else {
                    QuitOutcome::Failed
                }
            }
        }
    }

    fn force_kill(&self, pids: &[u32]) {
        if !pids.is_empty() {
            let _ = kill_command(pids).run();
        }
    }

    fn session_state(&self, _app: Option<&Path>, bundle_id: &str) -> SessionState {
        let mut session = SessionState::visible();
        if self.is_hidden(bundle_id) {
            session = SessionState::hidden();
        }
        if let Some(window) = self.window_state(bundle_id) {
            session = session.with_window(window);
        }
        session
    }

    fn open(&self, bundle_id: &str, hidden: bool) -> bool {
        open_command(bundle_id, hidden).run().is_ok()
    }

    fn restore_window(&self, bundle_id: &str, window: &WindowState) -> bool {
        for attempt in 0..RESTORE_ATTEMPTS {
            if restore_window_command(bundle_id, window).run().is_ok() {
                return true;
            }
            if attempt + 1 < RESTORE_ATTEMPTS {
                std::thread::sleep(RESTORE_RETRY_DELAY);
            }
        }
        false
    }

    fn update_state(&self, app: Option<&Path>, bundle_id: &str) -> UpdateState {
        let updater = app.and_then(self_updater_from_bundle);
        if updater.is_none() && app.is_some() {
            // The app does not update itself, so no updater can be running.
            return UpdateState::none();
        }

        match running_updater(&self.process_list(), app, bundle_id) {
            Some(running) => UpdateState::updating(running),
            None => match updater {
                Some(updater) => UpdateState::idle(updater),
                None => UpdateState::none(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};
    use std::time::Duration;

    use super::{
        automation_denied_message, bundle_id_command, find_running_apps, find_running_cask_apps,
        hidden_command, hidden_from_script, is_automation_denied, kill_command, open_command,
        pid_list_command, pids_from_apple_script, pids_from_processes, process_list_command,
        quit_apps, quit_choice, quit_command, reopen_apps, restore_window_command, running_updater,
        self_updater_from_bundle, window_state_command, window_state_from_script, AppSystem,
        QuitChoice, QuitOutcome, QuitPolicy, Reopen, SelfUpdater, SessionState, UpdateState,
        WindowState, SESSION_STATE_TIMEOUT,
    };
    use crate::policy::AppLists;

    #[derive(Default)]
    struct FakeSystem {
        bundle_ids: Mutex<HashMap<PathBuf, String>>,
        running: Mutex<HashMap<PathBuf, Vec<u32>>>,
        running_bundles: Mutex<HashMap<String, Vec<u32>>>,
        quit_stops_apps: bool,
        quit_outcome: QuitOutcome,
        open_succeeds: bool,
        quit_calls: Mutex<Vec<String>>,
        killed: Mutex<Vec<Vec<u32>>>,
        opened: Mutex<Vec<(String, bool)>>,
        restored: Mutex<Vec<(String, WindowState)>>,
        running_checks: Mutex<usize>,
        updating: Mutex<HashMap<String, usize>>,
        sessions: Mutex<HashMap<String, SessionState>>,
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    impl FakeSystem {
        fn new() -> Self {
            Self {
                open_succeeds: true,
                ..Self::default()
            }
        }

        fn with_app(self, app: &str, bundle_id: &str, pids: &[u32]) -> Self {
            let path = PathBuf::from(app);
            lock(&self.bundle_ids).insert(path.clone(), bundle_id.to_owned());
            lock(&self.running).insert(path, pids.to_vec());
            self
        }

        fn with_bundle_id(self, app: &str, bundle_id: &str) -> Self {
            lock(&self.bundle_ids).insert(PathBuf::from(app), bundle_id.to_owned());
            self
        }

        fn with_running_bundle(self, bundle_id: &str, pids: &[u32]) -> Self {
            lock(&self.running_bundles).insert(bundle_id.to_owned(), pids.to_vec());
            self
        }

        fn open_fails(mut self) -> Self {
            self.open_succeeds = false;
            self
        }

        fn denies_automation(mut self) -> Self {
            self.quit_outcome = QuitOutcome::Denied;
            self
        }

        /// The app updates itself for the next `checks` checks.
        ///
        /// The counter stands for an update that finishes after a while: every
        /// check of the update state uses one of them.
        fn with_updating(self, bundle_id: &str, checks: usize) -> Self {
            lock(&self.updating).insert(bundle_id.to_owned(), checks);
            self
        }

        /// The app looks this way before it is quit.
        fn with_session(self, bundle_id: &str, session: SessionState) -> Self {
            lock(&self.sessions).insert(bundle_id.to_owned(), session);
            self
        }
    }

    impl AppSystem for FakeSystem {
        fn bundle_id(&self, app: &std::path::Path) -> Option<String> {
            lock(&self.bundle_ids).get(app).cloned()
        }

        fn is_running(&self, app: &std::path::Path) -> bool {
            let mut checks = lock(&self.running_checks);
            *checks += 1;
            lock(&self.running)
                .get(app)
                .is_some_and(|pids| !pids.is_empty())
        }

        fn pids(&self, app: &std::path::Path) -> Vec<u32> {
            lock(&self.running).get(app).cloned().unwrap_or_default()
        }

        fn is_bundle_running(&self, bundle_id: &str) -> bool {
            let mut checks = lock(&self.running_checks);
            *checks += 1;
            lock(&self.running_bundles)
                .get(bundle_id)
                .is_some_and(|pids| !pids.is_empty())
        }

        fn bundle_pids(&self, bundle_id: &str) -> Vec<u32> {
            lock(&self.running_bundles)
                .get(bundle_id)
                .cloned()
                .unwrap_or_default()
        }

        fn quit(&self, bundle_id: &str) -> QuitOutcome {
            lock(&self.quit_calls).push(bundle_id.to_owned());
            if self.quit_stops_apps {
                lock(&self.running).clear();
                lock(&self.running_bundles).clear();
            }
            self.quit_outcome
        }

        fn force_kill(&self, pids: &[u32]) {
            lock(&self.killed).push(pids.to_vec());
            lock(&self.running).clear();
            lock(&self.running_bundles).clear();
        }

        fn session_state(&self, _app: Option<&std::path::Path>, bundle_id: &str) -> SessionState {
            lock(&self.sessions)
                .get(bundle_id)
                .copied()
                .unwrap_or_else(SessionState::visible)
        }

        fn open(&self, bundle_id: &str, hidden: bool) -> bool {
            lock(&self.opened).push((bundle_id.to_owned(), hidden));
            self.open_succeeds
        }

        fn restore_window(&self, bundle_id: &str, window: &WindowState) -> bool {
            lock(&self.restored).push((bundle_id.to_owned(), *window));
            true
        }

        fn update_state(&self, _app: Option<&std::path::Path>, bundle_id: &str) -> UpdateState {
            let mut updating = lock(&self.updating);
            match updating.get_mut(bundle_id) {
                Some(remaining) if *remaining > 0 => {
                    *remaining -= 1;
                    UpdateState::updating(SelfUpdater::Sparkle)
                }
                Some(_) => {
                    updating.remove(bundle_id);
                    UpdateState::idle(SelfUpdater::Sparkle)
                }
                None => UpdateState::none(),
            }
        }
    }

    fn installed(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    /// The bundle ids of the apps that have to be reopened.
    fn bundle_ids(apps: &[Reopen]) -> Vec<&str> {
        apps.iter().map(Reopen::bundle_id).collect()
    }

    /// The apps that were opened, with the hidden state they were opened in.
    fn opened(system: &FakeSystem) -> Vec<(String, bool)> {
        lock(&system.opened).clone()
    }

    fn running(system: &FakeSystem, app: &str) -> bool {
        system.is_running(&PathBuf::from(app))
    }

    fn fast_policy(force_kill: bool) -> QuitPolicy {
        QuitPolicy::new(Duration::ZERO, force_kill)
            .with_poll_interval(Duration::from_millis(1))
            .with_force_kill_grace(Duration::ZERO)
    }

    /// Creates a fake app bundle that holds the given entries.
    ///
    /// An entry ending in `.yml` is a file, every other entry a directory.
    fn fake_bundle(name: &str, entries: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("freshen-apps-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for entry in entries {
            let path = root.join(entry);
            if entry.ends_with(".yml") {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap_or_else(|error| {
                        panic!("failed to create {}: {error}", parent.display())
                    });
                }
                std::fs::write(&path, "provider: generic\n")
                    .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
            } else {
                std::fs::create_dir_all(&path)
                    .unwrap_or_else(|error| panic!("failed to create {}: {error}", path.display()));
            }
        }
        root
    }

    #[test]
    fn finds_running_apps_and_protects_the_host_terminal() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .with_app("/Applications/Terminal.app", "com.terminal", &[20])
            .with_bundle_id("/Applications/Editor.app", "com.editor");
        let apps = installed(&[
            "/Applications/Firefox.app",
            "/Applications/Terminal.app",
            "/Applications/Editor.app",
        ]);

        let scan = find_running_apps(&system, &apps, Some("com.terminal"), &AppLists::new());

        assert_eq!(scan.running().len(), 1);
        assert_eq!(scan.running()[0].name(), "Firefox.app");
        assert_eq!(scan.running()[0].bundle_id(), "com.firefox");
        assert_eq!(scan.protected(), ["Terminal.app"]);
        assert!(lock(&system.quit_calls).is_empty());
    }

    #[test]
    fn apps_without_a_bundle_id_are_ignored() {
        let system = FakeSystem::new();
        let mut running_apps = lock(&system.running);
        running_apps.insert(PathBuf::from("/Applications/Unknown.app"), vec![7]);
        drop(running_apps);

        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Unknown.app"]),
            None,
            &AppLists::new(),
        );

        assert!(scan.is_empty());
        assert!(scan.protected().is_empty());
    }

    #[test]
    fn apps_that_are_not_running_are_not_quit() {
        let system = FakeSystem::new().with_bundle_id("/Applications/Editor.app", "com.editor");

        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Editor.app"]),
            None,
            &AppLists::new(),
        );

        assert!(scan.is_empty());
    }

    #[test]
    fn a_running_bundle_id_without_an_app_path_is_quit() {
        let system = FakeSystem::new().with_running_bundle("com.example.menubar", &[42]);

        let scan = find_running_cask_apps(
            &system,
            &[],
            &["com.example.menubar".to_owned()],
            None,
            &AppLists::new(),
        );

        assert_eq!(scan.running().len(), 1);
        assert_eq!(scan.running()[0].bundle_id(), "com.example.menubar");
        assert_eq!(scan.running()[0].name(), "com.example.menubar");
        assert_eq!(scan.running()[0].path(), None);
    }

    #[test]
    fn a_bundle_id_already_found_through_its_app_path_is_not_added_twice() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .with_running_bundle("com.firefox", &[10]);

        let scan = find_running_cask_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            &["com.firefox".to_owned()],
            None,
            &AppLists::new(),
        );

        assert_eq!(scan.running().len(), 1);
        assert_eq!(scan.running()[0].name(), "Firefox.app");
    }

    #[test]
    fn a_bundle_id_only_app_can_be_protected_or_never_quit() {
        let system = FakeSystem::new()
            .with_running_bundle("com.terminal", &[10])
            .with_running_bundle("com.docker", &[20]);
        let lists = AppLists::new().with_never_quit(["com.docker"]);

        let scan = find_running_cask_apps(
            &system,
            &[],
            &["com.terminal".to_owned(), "com.docker".to_owned()],
            Some("com.terminal"),
            &lists,
        );

        assert!(scan.is_empty());
        assert_eq!(scan.protected(), ["com.terminal"]);
        assert_eq!(scan.never_quit(), ["com.docker"]);
    }

    #[test]
    fn a_bundle_id_only_app_is_quit_and_reopened_by_its_bundle_id() {
        let system = FakeSystem {
            quit_stops_apps: true,
            ..FakeSystem::new().with_running_bundle("com.example.helper", &[42])
        };
        let scan = find_running_cask_apps(
            &system,
            &[],
            &["com.example.helper".to_owned()],
            None,
            &AppLists::new(),
        );

        let reopened = quit_apps(&system, scan.running(), &fast_policy(false));

        assert_eq!(lock(&system.quit_calls).as_slice(), ["com.example.helper"]);
        assert_eq!(bundle_ids(&reopened), ["com.example.helper"]);
        assert_eq!(
            *lock(&system.running_checks),
            2,
            "the quit should be followed by a running check"
        );
    }

    #[test]
    fn an_app_on_the_never_quit_list_is_not_quit() {
        let system = FakeSystem::new().with_app("/Applications/Docker.app", "com.docker", &[10]);
        let lists = AppLists::new().with_never_quit(["com.docker"]);

        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Docker.app"]),
            None,
            &lists,
        );

        assert!(scan.is_empty());
        assert_eq!(scan.never_quit(), ["Docker.app"]);
        assert!(lock(&system.quit_calls).is_empty());
    }

    #[test]
    fn an_app_on_the_never_quit_list_is_named_by_its_app_name_too() {
        let system = FakeSystem::new().with_app("/Applications/Docker.app", "com.docker", &[10]);
        let lists = AppLists::new().with_never_quit(["Docker"]);

        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Docker.app"]),
            None,
            &lists,
        );

        assert!(scan.is_empty());
        assert_eq!(scan.never_quit(), ["Docker.app"]);
    }

    #[test]
    fn an_app_that_is_not_on_the_never_quit_list_is_still_quit() {
        let system = FakeSystem::new().with_app("/Applications/Docker.app", "com.docker", &[10]);
        let lists = AppLists::new().with_never_quit(["com.other"]);

        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Docker.app"]),
            None,
            &lists,
        );

        assert_eq!(scan.running().len(), 1);
        assert!(scan.never_quit().is_empty());
    }

    #[test]
    fn a_graceful_quit_waits_until_the_app_exits_without_killing() {
        let system = FakeSystem::new().with_app("/Applications/Firefox.app", "com.firefox", &[10]);
        let system = FakeSystem {
            quit_stops_apps: true,
            ..system
        };
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let reopen = quit_apps(
            &system,
            scan.running(),
            &QuitPolicy::new(Duration::from_secs(20), false),
        );

        assert_eq!(bundle_ids(&reopen), ["com.firefox"]);
        assert_eq!(lock(&system.quit_calls).as_slice(), ["com.firefox"]);
        assert!(lock(&system.killed).is_empty());
        assert!(!running(&system, "/Applications/Firefox.app"));
    }

    #[test]
    fn a_timeout_without_force_kill_leaves_the_app_running() {
        let system = FakeSystem::new().with_app("/Applications/Firefox.app", "com.firefox", &[10]);
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let reopen = quit_apps(&system, scan.running(), &fast_policy(false));

        assert_eq!(bundle_ids(&reopen), ["com.firefox"]);
        assert!(lock(&system.killed).is_empty());
        assert!(running(&system, "/Applications/Firefox.app"));
    }

    #[test]
    fn a_timeout_with_force_kill_kills_the_main_executable_pids() {
        let system =
            FakeSystem::new().with_app("/Applications/Firefox.app", "com.firefox", &[10, 11]);
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let reopen = quit_apps(&system, scan.running(), &fast_policy(true));

        assert_eq!(bundle_ids(&reopen), ["com.firefox"]);
        assert_eq!(lock(&system.killed).as_slice(), [vec![10, 11]]);
    }

    #[test]
    fn a_denied_automation_request_is_not_treated_as_an_app_that_cannot_quit() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .denies_automation();
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let reopen = quit_apps(&system, scan.running(), &fast_policy(false));

        assert_eq!(bundle_ids(&reopen), ["com.firefox"]);
        assert_eq!(lock(&system.quit_calls).as_slice(), ["com.firefox"]);
        assert!(lock(&system.killed).is_empty());
        assert!(running(&system, "/Applications/Firefox.app"));
    }

    #[test]
    fn a_denied_automation_request_does_not_wait_for_the_app_to_exit() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .denies_automation();
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );
        // The scan checks the process table once, which does not count.
        *lock(&system.running_checks) = 0;

        let reopen = quit_apps(
            &system,
            scan.running(),
            &QuitPolicy::new(Duration::from_secs(20), false),
        );

        assert_eq!(bundle_ids(&reopen), ["com.firefox"]);

        assert_eq!(
            *lock(&system.running_checks),
            0,
            "the app was waited for even though the request never reached it"
        );
    }

    #[test]
    fn a_denied_automation_request_is_force_killed_when_the_policy_allows_it() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .denies_automation();
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let reopen = quit_apps(&system, scan.running(), &fast_policy(true));

        assert_eq!(bundle_ids(&reopen), ["com.firefox"]);
        assert_eq!(lock(&system.killed).as_slice(), [vec![10]]);
    }

    #[test]
    fn detects_a_denied_automation_request_in_the_osascript_output() {
        for message in [
            "execution error: Not authorized to send Apple events to \"com.example.firefox\". (-1743)",
            "osascript: execution error: Not authorized to send Apple events to com.example.firefox. (-1743)",
            "Not authorized to send Apple events to com.example.firefox. (-1743)",
            "Not authorized to send Apple events to com.example.firefox.",
        ] {
            assert!(is_automation_denied(message), "{message}");
        }

        for message in [
            "",
            "execution error: Application isn't running. (-600)",
            "osascript: syntax error",
        ] {
            assert!(!is_automation_denied(message), "{message}");
        }
    }

    #[test]
    fn the_automation_denial_message_names_the_system_settings_pane() {
        let message = automation_denied_message("Firefox.app");

        assert!(message.contains("Firefox.app"), "{message}");
        assert!(
            message.contains("System Settings > Privacy & Security > Automation"),
            "{message}"
        );
        assert!(message.contains("-1743"), "{message}");
        assert!(!message.contains("did not quit"), "{message}");
    }

    #[test]
    fn reopens_only_the_bundle_ids_that_were_quit() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .with_app("/Applications/Terminal.app", "com.terminal", &[20]);
        let system = FakeSystem {
            quit_stops_apps: true,
            ..system
        };
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app", "/Applications/Terminal.app"]),
            Some("com.terminal"),
            &AppLists::new(),
        );

        let reopen = quit_apps(&system, scan.running(), &fast_policy(false));
        reopen_apps(&system, &reopen);

        assert_eq!(bundle_ids(&reopen), ["com.firefox"]);
        assert_eq!(opened(&system), [("com.firefox".to_owned(), false)]);
    }

    #[test]
    fn a_failed_reopen_is_reported_but_does_not_stop_the_others() {
        let system = FakeSystem::new().open_fails();

        reopen_apps(
            &system,
            &[
                Reopen::new("com.first", SessionState::visible()),
                Reopen::new("com.second", SessionState::visible()),
            ],
        );

        assert_eq!(
            opened(&system),
            [
                ("com.first".to_owned(), false),
                ("com.second".to_owned(), false)
            ]
        );
    }

    #[test]
    fn an_app_that_was_hidden_is_reopened_hidden() {
        let system = FakeSystem::new();

        reopen_apps(
            &system,
            &[Reopen::new("com.firefox", SessionState::hidden())],
        );

        assert_eq!(opened(&system), [("com.firefox".to_owned(), true)]);
    }

    #[test]
    fn a_visible_app_is_reopened_in_the_background() {
        let system = FakeSystem::new();

        reopen_apps(
            &system,
            &[Reopen::new("com.firefox", SessionState::visible())],
        );

        assert_eq!(opened(&system), [("com.firefox".to_owned(), false)]);
    }

    #[test]
    fn the_window_of_an_app_is_put_back_where_it_was() {
        let system = FakeSystem::new();
        let window = WindowState::new((10, 20), (1440, 900));

        reopen_apps(
            &system,
            &[Reopen::new(
                "com.firefox",
                SessionState::visible().with_window(window),
            )],
        );

        assert_eq!(
            lock(&system.restored).as_slice(),
            [("com.firefox".to_owned(), window)]
        );
    }

    #[test]
    fn the_window_of_a_hidden_app_is_not_moved() {
        let system = FakeSystem::new();
        let window = WindowState::new((10, 20), (1440, 900));

        reopen_apps(
            &system,
            &[Reopen::new(
                "com.firefox",
                SessionState::hidden().with_window(window),
            )],
        );

        assert!(
            lock(&system.restored).is_empty(),
            "the window of a hidden app was moved"
        );
    }

    #[test]
    fn an_app_that_cannot_be_opened_is_not_moved_back() {
        let system = FakeSystem::new().open_fails();
        let window = WindowState::new((10, 20), (1440, 900));

        reopen_apps(
            &system,
            &[Reopen::new(
                "com.firefox",
                SessionState::visible().with_window(window),
            )],
        );

        assert!(
            lock(&system.restored).is_empty(),
            "the window of an app that did not open was moved"
        );
    }

    #[test]
    fn the_session_state_of_an_app_is_read_before_it_is_quit() {
        let system = FakeSystem {
            quit_stops_apps: true,
            ..FakeSystem::new()
                .with_app("/Applications/Firefox.app", "com.firefox", &[10])
                .with_session(
                    "com.firefox",
                    SessionState::visible().with_window(WindowState::new((10, 20), (1440, 900))),
                )
        };
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let reopen = quit_apps(&system, scan.running(), &fast_policy(false));

        assert_eq!(reopen.len(), 1);
        assert!(!reopen[0].session().is_hidden());
        assert_eq!(
            reopen[0].session().window().map(|window| window.position()),
            Some((10, 20))
        );
    }

    #[test]
    fn an_app_without_a_session_state_is_reopened_as_it_was_visible() {
        let system = FakeSystem {
            quit_stops_apps: true,
            ..FakeSystem::new().with_app("/Applications/Firefox.app", "com.firefox", &[10])
        };
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let reopen = quit_apps(&system, scan.running(), &fast_policy(false));
        reopen_apps(&system, &reopen);

        assert_eq!(opened(&system), [("com.firefox".to_owned(), false)]);
        assert!(lock(&system.restored).is_empty());
    }

    #[test]
    fn an_app_that_did_not_quit_keeps_its_own_state() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .with_session("com.firefox", SessionState::hidden());
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let reopen = quit_apps(&system, scan.running(), &fast_policy(false));
        reopen_apps(&system, &reopen);

        assert!(
            running(&system, "/Applications/Firefox.app"),
            "the app was quit"
        );
        assert_eq!(
            opened(&system),
            [("com.firefox".to_owned(), false)],
            "an app that is still running was reopened hidden"
        );
    }

    #[test]
    fn the_hidden_state_is_read_from_the_system_events_output() {
        assert_eq!(hidden_from_script("true\n"), Some(true));
        assert_eq!(hidden_from_script("false\n"), Some(false));
        assert_eq!(hidden_from_script("TRUE\n"), Some(true));
        assert_eq!(hidden_from_script("missing value"), None);
        assert_eq!(hidden_from_script(""), None);
    }

    #[test]
    fn the_place_of_a_window_is_read_from_the_system_events_output() {
        let window = window_state_from_script("{{0, 0}, {1440, 900}}");

        assert_eq!(window.map(|window| window.position()), Some((0, 0)));
        assert_eq!(window.map(|window| window.size()), Some((1440, 900)));
    }

    #[test]
    fn a_window_on_a_second_display_has_a_negative_position() {
        let window = window_state_from_script("{{-1920, 100}, {1280, 800}}");

        assert_eq!(window.map(|window| window.position()), Some((-1920, 100)));
        assert_eq!(window.map(|window| window.size()), Some((1280, 800)));
    }

    #[test]
    fn a_window_without_a_place_is_not_read() {
        assert_eq!(window_state_from_script("missing value"), None);
        assert_eq!(window_state_from_script("{{0, 0}}"), None);
        assert_eq!(window_state_from_script(""), None);
    }

    #[test]
    fn an_app_that_updates_itself_is_waited_for_before_it_is_quit() {
        let system = FakeSystem {
            quit_stops_apps: true,
            ..FakeSystem::new()
                .with_app("/Applications/Firefox.app", "com.firefox", &[10])
                .with_updating("com.firefox", 3)
        };
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        assert_eq!(scan.updating().len(), 1);
        assert_eq!(scan.updating()[0].name(), "Firefox.app");

        let reopen = quit_apps(
            &system,
            scan.running(),
            &fast_policy(false).with_update_timeout(Duration::from_secs(5)),
        );

        assert_eq!(bundle_ids(&reopen), ["com.firefox"]);
        assert_eq!(lock(&system.quit_calls).as_slice(), ["com.firefox"]);
    }

    #[test]
    fn an_app_that_does_not_finish_updating_is_not_quit() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .with_updating("com.firefox", usize::MAX);
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let reopen = quit_apps(
            &system,
            scan.running(),
            &fast_policy(false).with_update_timeout(Duration::ZERO),
        );

        assert!(reopen.is_empty(), "the app was reopened: {reopen:?}");
        assert!(lock(&system.quit_calls).is_empty(), "the app was quit");
        assert!(running(&system, "/Applications/Firefox.app"));
    }

    #[test]
    fn an_app_that_does_not_update_itself_is_quit_without_waiting() {
        let system = FakeSystem {
            quit_stops_apps: true,
            ..FakeSystem::new().with_app("/Applications/Firefox.app", "com.firefox", &[10])
        };
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        assert!(scan.updating().is_empty());

        let reopen = quit_apps(&system, scan.running(), &fast_policy(false));

        assert_eq!(bundle_ids(&reopen), ["com.firefox"]);
        assert_eq!(lock(&system.quit_calls).as_slice(), ["com.firefox"]);
    }

    #[test]
    fn the_wait_for_an_update_stops_when_the_run_is_interrupted() {
        let system = FakeSystem::new()
            .with_app("/Applications/Firefox.app", "com.firefox", &[10])
            .with_updating("com.firefox", usize::MAX);
        let scan = find_running_apps(
            &system,
            &installed(&["/Applications/Firefox.app"]),
            None,
            &AppLists::new(),
        );

        let finished = super::wait_for_self_update_with(
            &system,
            &scan.running()[0],
            &fast_policy(false).with_update_timeout(Duration::from_secs(60)),
            || true,
        );

        assert!(!finished, "the wait ignored the interrupt");
        assert!(lock(&system.quit_calls).is_empty(), "the app was quit");
    }

    #[test]
    fn the_update_state_says_whether_the_app_is_updating_itself() {
        assert!(!UpdateState::none().in_progress());
        assert_eq!(UpdateState::none().updater(), None);
        assert!(!UpdateState::idle(SelfUpdater::Sparkle).in_progress());
        assert_eq!(
            UpdateState::idle(SelfUpdater::Sparkle).updater(),
            Some(SelfUpdater::Sparkle)
        );
        assert!(UpdateState::updating(SelfUpdater::Electron).in_progress());
        assert_eq!(
            UpdateState::updating(SelfUpdater::Electron).updater(),
            Some(SelfUpdater::Electron)
        );
    }

    #[test]
    fn a_sparkle_updater_running_for_an_app_is_detected() {
        let processes = "\
/Applications/Firefox.app/Contents/MacOS/firefox
/Applications/Firefox.app/Contents/Frameworks/Sparkle.framework/Versions/B/Autoupdate";

        let updater = running_updater(
            processes,
            Some(std::path::Path::new("/Applications/Firefox.app")),
            "com.firefox",
        );

        assert_eq!(updater, Some(SelfUpdater::Sparkle));
    }

    #[test]
    fn the_electron_updater_of_an_app_is_detected_by_its_bundle_id() {
        let processes = "/Users/test/Library/Caches/com.example.editor/ShipIt/ShipIt";

        let updater = running_updater(processes, None, "com.example.editor");

        assert_eq!(updater, Some(SelfUpdater::Electron));
    }

    #[test]
    fn the_updater_of_another_app_is_not_mistaken_for_this_one() {
        let processes = "/Users/test/Library/Caches/com.example.editor/ShipIt/ShipIt";

        let updater = running_updater(
            processes,
            Some(std::path::Path::new("/Applications/Other.app")),
            "com.example.other",
        );

        assert_eq!(updater, None);
    }

    #[test]
    fn an_app_that_is_not_updating_itself_has_no_running_updater() {
        let processes = "/Applications/Firefox.app/Contents/MacOS/firefox";

        let updater = running_updater(
            processes,
            Some(std::path::Path::new("/Applications/Firefox.app")),
            "com.firefox",
        );

        assert_eq!(updater, None);
    }

    #[test]
    fn the_updater_of_an_app_is_detected_from_its_bundle() {
        let sparkle = fake_bundle("sparkle", &["Contents/Frameworks/Sparkle.framework"]);
        assert_eq!(
            self_updater_from_bundle(&sparkle),
            Some(SelfUpdater::Sparkle)
        );
        let _ = std::fs::remove_dir_all(&sparkle);

        let electron = fake_bundle("electron", &["Contents/Resources/app-update.yml"]);
        assert_eq!(
            self_updater_from_bundle(&electron),
            Some(SelfUpdater::Electron)
        );
        let _ = std::fs::remove_dir_all(&electron);

        let plain = fake_bundle("plain", &["Contents/MacOS/app"]);
        assert_eq!(self_updater_from_bundle(&plain), None);
        let _ = std::fs::remove_dir_all(&plain);
    }

    #[test]
    fn prompt_answers_match_the_zsh_script() {
        for answer in ["", "y", "Y", "yes", "YES", " yes "] {
            assert_eq!(quit_choice(answer), QuitChoice::Quit, "answer {answer:?}");
        }
        for answer in ["s", "S", "skip", "SKIP"] {
            assert_eq!(
                quit_choice(answer),
                QuitChoice::SkipQuit,
                "answer {answer:?}"
            );
        }
        for answer in ["n", "no", "anything else"] {
            assert_eq!(
                quit_choice(answer),
                QuitChoice::SkipCasks,
                "answer {answer:?}"
            );
        }
    }

    #[test]
    fn pids_only_match_the_main_executable_not_helpers() {
        let processes = "\
  100 /Applications/Firefox.app/Contents/MacOS/firefox
  101 /Applications/Firefox.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper
  102 /Applications/Firefox.app/Contents/MacOS/firefox --background
  103 /Applications/Other.app/Contents/MacOS/firefox";

        let pids =
            pids_from_processes(processes, std::path::Path::new("/Applications/Firefox.app"));

        assert_eq!(pids, [100, 102]);
    }

    #[test]
    fn pids_are_read_from_the_apple_script_bundle_id_list() {
        assert_eq!(pids_from_apple_script("123, 456\n"), [123, 456]);
        assert_eq!(pids_from_apple_script("missing value"), Vec::<u32>::new());
    }

    /// The arguments of a command in the form the zsh script writes them.
    ///
    /// A path argument is joined with the separator of the platform, so it is
    /// normalized to the POSIX form before it is compared with the transcript.
    fn transcript(spec: &super::CommandSpec) -> Vec<String> {
        spec.arguments()
            .iter()
            .map(|argument| {
                argument
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/")
            })
            .collect()
    }

    #[test]
    fn command_transcripts_match_the_zsh_script() {
        let app = std::path::Path::new("/Applications/Firefox.app");

        assert_eq!(
            transcript(&bundle_id_command(app)),
            [
                "read",
                "/Applications/Firefox.app/Contents/Info",
                "CFBundleIdentifier"
            ]
        );
        assert_eq!(transcript(&process_list_command()), ["-axo", "command="]);
        assert_eq!(transcript(&pid_list_command()), ["-axo", "pid=,command="]);
        assert_eq!(
            transcript(&quit_command("com.example.firefox")),
            [
                "-e",
                "with timeout of 3 seconds",
                "-e",
                "tell application id \"com.example.firefox\" to quit",
                "-e",
                "end timeout",
            ]
        );
        assert_eq!(transcript(&kill_command(&[10, 11])), ["-9", "10", "11"]);
        assert_eq!(
            transcript(&open_command("com.example.firefox", false)),
            ["-g", "-b", "com.example.firefox"]
        );
        assert_eq!(
            transcript(&open_command("com.example.firefox", true)),
            ["-g", "-j", "-b", "com.example.firefox"]
        );
        assert_eq!(
            hidden_command("com.example.firefox").timeout_limit(),
            Some(SESSION_STATE_TIMEOUT),
            "a permission dialog must not hold up the run"
        );
        assert_eq!(
            transcript(&hidden_command("com.example.firefox")),
            [
                "-e",
                "tell application \"System Events\"",
                "-e",
                "set appProcess to first process whose bundle identifier is \"com.example.firefox\"",
                "-e",
                "get value of attribute \"AXHidden\" of appProcess",
                "-e",
                "end tell",
            ]
        );
        assert_eq!(
            transcript(&window_state_command("com.example.firefox")),
            [
                "-e",
                "tell application \"System Events\"",
                "-e",
                "set appProcess to first process whose bundle identifier is \"com.example.firefox\"",
                "-e",
                "tell appProcess",
                "-e",
                "get {position of window 1, size of window 1}",
                "-e",
                "end tell",
                "-e",
                "end tell",
            ]
        );
        assert_eq!(
            transcript(&restore_window_command(
                "com.example.firefox",
                &WindowState::new((10, 20), (1440, 900))
            )),
            [
                "-e",
                "tell application \"System Events\"",
                "-e",
                "set appProcess to first process whose bundle identifier is \"com.example.firefox\"",
                "-e",
                "tell appProcess",
                "-e",
                "set position of window 1 to {10, 20}",
                "-e",
                "set size of window 1 to {1440, 900}",
                "-e",
                "end tell",
                "-e",
                "end tell",
            ]
        );
    }
}
