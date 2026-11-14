//! User-activity guards for scheduled runs.
//!
//! A scheduled run (`freshen --unattended`) should not start while the user
//! is in a meeting or presentation (the frontmost app is full screen) or
//! while the user has been active recently. The facts are read through
//! [`UserActivityProbe`] so tests can inject fixed facts instead of reading
//! the real machine. Every check is best effort: a fact the host cannot
//! determine never blocks a run.

use std::fmt;

/// Default idle window, in minutes: a scheduled run is skipped when the user
/// was active within this many minutes.
pub const DEFAULT_IDLE_MINUTES: u64 = 15;

/// Why a scheduled run was skipped because of user activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserActivitySkipReason {
    /// The frontmost app is full screen (a meeting or presentation).
    FrontmostAppFullScreen,
    /// The user was active within the idle window.
    RecentlyActive {
        /// Minutes since the last user input.
        idle_minutes: u64,
        /// The configured idle window, in minutes.
        configured: u64,
    },
}

impl fmt::Display for UserActivitySkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrontmostAppFullScreen => write!(
                f,
                "the frontmost app is full screen (a meeting or presentation)"
            ),
            Self::RecentlyActive {
                idle_minutes,
                configured,
            } => write!(
                f,
                "the user was active {idle_minutes} minutes ago, within the {configured}-minute idle window"
            ),
        }
    }
}

/// The outcome of assessing user activity for a scheduled run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserActivityOutcome {
    /// The run may proceed.
    Ok,
    /// The run should be skipped.
    Skip(UserActivitySkipReason),
}

/// Facts about the user's presence that a scheduled run needs.
///
/// Tests provide fixed facts through a fake probe; [`HostUserActivityProbe`]
/// reads the real machine. Returning `false` / `None` means "cannot determine",
/// which never blocks a run.
pub trait UserActivityProbe {
    /// Returns `true` when the frontmost app is full screen, for example a
    /// video call in full-screen mode or a presentation.
    fn frontmost_app_is_fullscreen(&self) -> bool;

    /// Minutes since the last user input, or `None` when the host cannot tell.
    fn user_idle_minutes(&self) -> Option<u64>;
}

/// Configuration for the user-activity guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserActivityConfig {
    /// Whether the guard runs (default: `true`).
    pub enabled: bool,
    /// Minutes of user inactivity required before a scheduled run may start
    /// (default: 15).
    pub idle_minutes: u64,
}

impl UserActivityConfig {
    /// Creates the default configuration: the guard enabled with a 15-minute
    /// idle window.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the guard is enabled.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The configured idle window, in minutes.
    #[must_use]
    pub fn idle_minutes(&self) -> u64 {
        self.idle_minutes
    }
}

impl Default for UserActivityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_minutes: DEFAULT_IDLE_MINUTES,
        }
    }
}

/// Assesses the user-activity facts and returns whether a scheduled run may
/// proceed.
///
/// The frontmost-app-full-screen check takes precedence: a presentation is
/// always deferred, even when the user has been idle for a long time.
#[must_use]
pub fn assess(probe: &dyn UserActivityProbe, config: &UserActivityConfig) -> UserActivityOutcome {
    if !config.enabled() {
        return UserActivityOutcome::Ok;
    }

    if probe.frontmost_app_is_fullscreen() {
        return UserActivityOutcome::Skip(UserActivitySkipReason::FrontmostAppFullScreen);
    }

    if let Some(idle_minutes) = probe.user_idle_minutes() {
        if idle_minutes < config.idle_minutes() {
            return UserActivityOutcome::Skip(UserActivitySkipReason::RecentlyActive {
                idle_minutes,
                configured: config.idle_minutes(),
            });
        }
    }

    UserActivityOutcome::Ok
}

/// Reads user-activity facts from the real machine (best effort).
#[derive(Debug, Default, Clone, Copy)]
pub struct HostUserActivityProbe;

impl UserActivityProbe for HostUserActivityProbe {
    fn frontmost_app_is_fullscreen(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            macos_frontmost_app_is_fullscreen()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    fn user_idle_minutes(&self) -> Option<u64> {
        #[cfg(target_os = "macos")]
        {
            macos_user_idle_minutes()
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_frontmost_app_is_fullscreen() -> bool {
    // Best effort: ask the frontmost app whether its main window is zoomed
    // (full screen). A failure means "cannot determine", which never blocks.
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to \
             tell (first process whose frontmost is true) to \
             get value of attribute \"AXFullScreen\" of first window",
        ])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim().eq_ignore_ascii_case("true")
}

#[cfg(target_os = "macos")]
fn macos_user_idle_minutes() -> Option<u64> {
    // Best effort: `ioreg -c IOHIDSystem` reports HIDIdleTime in nanoseconds.
    let output = std::process::Command::new("ioreg")
        .args(["-c", "IOHIDSystem"])
        .output();
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut idle_ns: Option<u64> = None;
    for line in text.lines() {
        let Some((key, rest)) = line.split_once('"') else {
            continue;
        };
        let key = key.trim();
        if !key.ends_with("HIDIdleTime") {
            continue;
        }
        let value = rest
            .split_once('=')?
            .1
            .split(|c: char| !c.is_ascii_digit())
            .next()?;
        let Ok(ns) = value.trim().parse::<u64>() else {
            continue;
        };
        idle_ns = Some(ns);
    }
    let ns = idle_ns?;
    Some(ns / 1_000_000_000 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProbe {
        fullscreen: bool,
        idle: Option<u64>,
    }

    impl FakeProbe {
        fn new(fullscreen: bool, idle: Option<u64>) -> Self {
            Self { fullscreen, idle }
        }
    }

    impl UserActivityProbe for FakeProbe {
        fn frontmost_app_is_fullscreen(&self) -> bool {
            self.fullscreen
        }

        fn user_idle_minutes(&self) -> Option<u64> {
            self.idle
        }
    }

    #[test]
    fn fullscreen_frontmost_app_always_defers_a_scheduled_run() {
        let config = UserActivityConfig::new();
        let idle_for_hours = FakeProbe::new(true, Some(120));
        let not_idle_at_all = FakeProbe::new(true, None);

        assert!(matches!(
            assess(&idle_for_hours, &config),
            UserActivityOutcome::Skip(UserActivitySkipReason::FrontmostAppFullScreen)
        ));
        assert!(matches!(
            assess(&not_idle_at_all, &config),
            UserActivityOutcome::Skip(UserActivitySkipReason::FrontmostAppFullScreen)
        ));
    }

    #[test]
    fn recently_active_user_defers_a_scheduled_run() {
        let config = UserActivityConfig::new(); // 15-minute window
        let active_five_minutes_ago = FakeProbe::new(false, Some(5));

        assert!(matches!(
            assess(&active_five_minutes_ago, &config),
            UserActivityOutcome::Skip(UserActivitySkipReason::RecentlyActive {
                idle_minutes: 5,
                configured: 15,
            })
        ));
    }

    #[test]
    fn idle_user_within_but_not_below_the_window_proceeds() {
        let config = UserActivityConfig::new();
        let idle_fifteen_minutes = FakeProbe::new(false, Some(15));
        let idle_twenty_minutes = FakeProbe::new(false, Some(20));

        assert!(matches!(
            assess(&idle_fifteen_minutes, &config),
            UserActivityOutcome::Ok
        ));
        assert!(matches!(
            assess(&idle_twenty_minutes, &config),
            UserActivityOutcome::Ok
        ));
    }

    #[test]
    fn unknown_idle_time_never_blocks() {
        let config = UserActivityConfig::new();
        let unknown = FakeProbe::new(false, None);

        assert!(matches!(assess(&unknown, &config), UserActivityOutcome::Ok));
    }

    #[test]
    fn a_custom_idle_window_is_applied() {
        let config = UserActivityConfig {
            enabled: true,
            idle_minutes: 5,
        };
        let active_two_minutes_ago = FakeProbe::new(false, Some(2));
        let idle_ten_minutes = FakeProbe::new(false, Some(10));

        assert!(matches!(
            assess(&active_two_minutes_ago, &config),
            UserActivityOutcome::Skip(UserActivitySkipReason::RecentlyActive {
                idle_minutes: 2,
                configured: 5,
            })
        ));
        assert!(matches!(
            assess(&idle_ten_minutes, &config),
            UserActivityOutcome::Ok
        ));
    }

    #[test]
    fn disabling_the_guard_never_skips() {
        let config = UserActivityConfig {
            enabled: false,
            idle_minutes: DEFAULT_IDLE_MINUTES,
        };
        let active_now = FakeProbe::new(true, Some(0));

        assert!(matches!(
            assess(&active_now, &config),
            UserActivityOutcome::Ok
        ));
    }

    #[test]
    fn skip_reason_messages_are_human_readable() {
        let fullscreen = UserActivitySkipReason::FrontmostAppFullScreen;
        assert_eq!(
            fullscreen.to_string(),
            "the frontmost app is full screen (a meeting or presentation)"
        );

        let active = UserActivitySkipReason::RecentlyActive {
            idle_minutes: 5,
            configured: 15,
        };
        assert_eq!(
            active.to_string(),
            "the user was active 5 minutes ago, within the 15-minute idle window"
        );
    }
}
