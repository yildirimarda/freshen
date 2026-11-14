//! Scheduling unattended freshen runs.
//!
//! On macOS, [`install`] writes a per-user launchd agent property list to
//! `~/Library/LaunchAgents/com.freshen.schedule.plist` and loads it with
//! `launchctl`, so `freshen` runs on a repeating interval. On Linux, [`install`]
//! writes a per-user systemd service and timer unit to
//! `~/.config/systemd/user/` and enables the timer with `systemctl --user`.
//! [`remove`] unloads/deletes (macOS) or stops/disables and deletes (Linux) the
//! scheduled unit.
//!
//! The scheduled unit runs `freshen --check` so a scheduled run reports
//! available updates without changing anything. Scheduling is supported on
//! macOS and Linux: on every other platform [`install`] and [`remove`] report
//! that it is not supported.

use std::fmt;
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
use std::path::Path;
use std::path::PathBuf;

/// The label of the scheduled launchd agent.
pub const AGENT_LABEL: &str = "com.freshen.schedule";

/// The property-list file name of the agent.
#[cfg(any(test, target_os = "macos"))]
pub(crate) const PLIST_FILE_NAME: &str = "com.freshen.schedule.plist";

/// The timer unit file name on Linux.
#[cfg(any(test, target_os = "linux"))]
pub(crate) const TIMER_FILE_NAME: &str = "com.freshen.schedule.timer";

/// The service unit file name the timer runs on Linux.
#[cfg(any(test, target_os = "linux"))]
pub(crate) const SERVICE_FILE_NAME: &str = "com.freshen.schedule.service";

/// How often the scheduled agent runs by default, in hours.
#[cfg(test)]
pub(crate) const DEFAULT_EVERY_HOURS: u64 = 24;

/// A problem installing or removing the scheduled unit.
#[derive(Debug)]
pub enum ScheduleError {
    /// Scheduling is not supported on this operating system.
    NotSupported { platform: &'static str },
    /// A unit file could not be written.
    WriteUnit {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The timer could not be enabled and started.
    LoadFailed { source: std::io::Error },
    /// The timer could not be disabled and stopped.
    UnloadFailed { source: std::io::Error },
    /// A unit file could not be removed.
    RemoveUnit {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSupported { platform } => {
                write!(f, "scheduling is not supported on {platform}")
            }
            Self::WriteUnit { path, source } => write!(
                f,
                "could not write the scheduled timer unit file to {}: {source}",
                path.display()
            ),
            Self::LoadFailed { source } => {
                write!(
                    f,
                    "could not enable and start the scheduled timer with systemctl --user: {source}"
                )
            }
            Self::UnloadFailed { source } => {
                write!(
                    f,
                    "could not disable and stop the scheduled timer with systemctl --user: {source}"
                )
            }
            Self::RemoveUnit { path, source } => write!(
                f,
                "could not remove the scheduled timer unit file {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// The directory the agent property list lives in, derived from the home
/// directory.
#[cfg(any(test, target_os = "macos"))]
#[must_use]
pub(crate) fn launch_agents_dir(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents")
}

/// The path of the agent property list.
#[cfg(any(test, target_os = "macos"))]
#[must_use]
pub(crate) fn plist_path(home: &Path) -> PathBuf {
    launch_agents_dir(home).join(PLIST_FILE_NAME)
}

/// The XML property list that runs `freshen --check` every `every_hours`
/// hours.
#[cfg(any(test, target_os = "macos"))]
#[must_use]
pub(crate) fn plist_xml(binary: &Path, every_hours: u64) -> String {
    let seconds = every_hours * 60 * 60;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--check</string>
    </array>
    <key>StartInterval</key>
    <integer>{seconds}</integer>
    <key>RunAtLoad</key>
    <false/>
</dict>
</plist>"#,
        binary.display()
    )
}

/// The directory the systemd user units live in, derived from the home
/// directory.
#[cfg(any(test, target_os = "linux"))]
#[must_use]
pub(crate) fn systemd_user_dir(home: &Path) -> PathBuf {
    home.join(".config").join("systemd").join("user")
}

/// The path of the timer unit on Linux.
#[cfg(any(test, target_os = "linux"))]
#[must_use]
pub(crate) fn timer_path(home: &Path) -> PathBuf {
    systemd_user_dir(home).join(TIMER_FILE_NAME)
}

/// The path of the service unit the timer runs on Linux.
#[cfg(any(test, target_os = "linux"))]
#[must_use]
pub(crate) fn service_path(home: &Path) -> PathBuf {
    systemd_user_dir(home).join(SERVICE_FILE_NAME)
}

/// The systemd service unit that runs `freshen --check`.
#[cfg(any(test, target_os = "linux"))]
#[must_use]
pub(crate) fn service_unit(binary: &Path) -> String {
    format!(
        r#"[Unit]
Description=freshen scheduled update check

[Service]
Type=oneshot
ExecStart={} --check
"#,
        binary.display()
    )
}

/// The systemd timer unit that runs the service every `every_hours` hours.
#[cfg(any(test, target_os = "linux"))]
#[must_use]
pub(crate) fn timer_unit(every_hours: u64) -> String {
    let seconds = every_hours * 60 * 60;
    format!(
        r#"[Unit]
Description=freshen scheduled update check timer

[Timer]
OnUnitActiveSec={seconds}
Unit=com.freshen.schedule.service

[Install]
WantedBy=timers.target
"#
    )
}

/// Installs (writes and loads) the scheduled launchd agent.
///
/// On macOS this writes the property list and loads it with `launchctl`. On
/// every other platform it reports that scheduling is not supported.
pub fn install(every_hours: u64) -> Result<(), ScheduleError> {
    #[cfg(target_os = "macos")]
    {
        install_macos(every_hours)
    }
    #[cfg(target_os = "linux")]
    {
        install_linux(every_hours)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = every_hours;
        Err(ScheduleError::NotSupported {
            platform: std::env::consts::OS,
        })
    }
}

/// Removes (unloads and deletes) the scheduled launchd agent.
///
/// On macOS this unloads the agent with `launchctl` and deletes its property
/// list. On Linux this stops and disables the timer with `systemctl --user`
/// and deletes its unit files. On every other platform it reports that
/// scheduling is not supported.
pub fn remove() -> Result<(), ScheduleError> {
    #[cfg(target_os = "macos")]
    {
        remove_macos()
    }
    #[cfg(target_os = "linux")]
    {
        remove_linux()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(ScheduleError::NotSupported {
            platform: std::env::consts::OS,
        })
    }
}

/// Writes the agent property list and loads it with `launchctl`.
#[cfg(target_os = "macos")]
fn install_macos(every_hours: u64) -> Result<(), ScheduleError> {
    let home = std::env::var_os("HOME").ok_or(ScheduleError::NotSupported { platform: "macos" })?;
    let home = PathBuf::from(home);
    let directory = launch_agents_dir(&home);
    std::fs::create_dir_all(&directory).map_err(|source| ScheduleError::WriteUnit {
        path: plist_path(&home),
        source,
    })?;
    let binary = std::env::current_exe().map_err(|source| ScheduleError::WriteUnit {
        path: plist_path(&home),
        source,
    })?;
    let plist = plist_xml(&binary, every_hours);
    let path = plist_path(&home);
    std::fs::write(&path, plist).map_err(|source| ScheduleError::WriteUnit {
        path: path.clone(),
        source,
    })?;
    crate::process::CommandSpec::new("launchctl")
        .arg("load")
        .arg(path.to_string_lossy().into_owned())
        .run()
        .map_err(|source| ScheduleError::LoadFailed {
            source: std::io::Error::other(source),
        })?;
    Ok(())
}

/// Unloads the agent with `launchctl` and deletes its property list.
#[cfg(target_os = "macos")]
fn remove_macos() -> Result<(), ScheduleError> {
    let home = std::env::var_os("HOME").ok_or(ScheduleError::NotSupported { platform: "macos" })?;
    let home = PathBuf::from(home);
    let path = plist_path(&home);
    // An agent that was never loaded cannot be unloaded; ignore that error so
    // removing an absent agent still deletes any leftover property list.
    let _ = crate::process::CommandSpec::new("launchctl")
        .arg("unload")
        .arg(path.to_string_lossy().into_owned())
        .run();
    std::fs::remove_file(&path).map_err(|source| ScheduleError::RemoveUnit { path, source })?;
    Ok(())
}

/// Writes the service and timer unit files and enables the timer with
/// `systemctl --user`.
#[cfg(target_os = "linux")]
fn install_linux(every_hours: u64) -> Result<(), ScheduleError> {
    let home = std::env::var_os("HOME").ok_or(ScheduleError::NotSupported { platform: "linux" })?;
    let home = PathBuf::from(home);
    let directory = systemd_user_dir(&home);
    std::fs::create_dir_all(&directory).map_err(|source| ScheduleError::WriteUnit {
        path: timer_path(&home),
        source,
    })?;
    let binary = std::env::current_exe().map_err(|source| ScheduleError::WriteUnit {
        path: timer_path(&home),
        source,
    })?;
    let service = service_unit(&binary);
    let service_path = service_path(&home);
    std::fs::write(&service_path, service).map_err(|source| ScheduleError::WriteUnit {
        path: service_path.clone(),
        source,
    })?;
    let timer = timer_unit(every_hours);
    let path = timer_path(&home);
    std::fs::write(&path, timer).map_err(|source| ScheduleError::WriteUnit {
        path: path.clone(),
        source,
    })?;
    // Reload systemd so it picks up the new units.
    crate::process::CommandSpec::new("systemctl")
        .args(["--user", "daemon-reload"])
        .run()
        .map_err(|source| ScheduleError::LoadFailed {
            source: std::io::Error::other(source),
        })?;
    // Enable and start the timer so it runs on schedule.
    crate::process::CommandSpec::new("systemctl")
        .args(["--user", "enable", "--now", "com.freshen.schedule.timer"])
        .run()
        .map_err(|source| ScheduleError::LoadFailed {
            source: std::io::Error::other(source),
        })?;
    Ok(())
}

/// Stops and disables the timer with `systemctl --user` and deletes its unit
/// files.
#[cfg(target_os = "linux")]
fn remove_linux() -> Result<(), ScheduleError> {
    let home = std::env::var_os("HOME").ok_or(ScheduleError::NotSupported { platform: "linux" })?;
    let home = PathBuf::from(home);
    let path = timer_path(&home);
    // A timer that was never enabled cannot be disabled; ignore that error so
    // removing an absent timer still deletes any leftover unit files.
    let _ = crate::process::CommandSpec::new("systemctl")
        .args(["--user", "disable", "--now", "com.freshen.schedule.timer"])
        .run();
    let _ = crate::process::CommandSpec::new("systemctl")
        .args(["--user", "daemon-reload"])
        .run();
    std::fs::remove_file(&path).map_err(|source| ScheduleError::RemoveUnit { path, source })?;
    let service_path = service_path(&home);
    let _ = std::fs::remove_file(&service_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_path_is_under_library_launch_agents() {
        let home = PathBuf::from("/Users/developer");
        let path = plist_path(&home);
        assert_eq!(
            path,
            PathBuf::from("/Users/developer/Library/LaunchAgents/com.freshen.schedule.plist")
        );
        assert!(path.ends_with("com.freshen.schedule.plist"));
    }

    #[test]
    fn plist_xml_names_the_agent_and_runs_check_every_interval() {
        let binary = PathBuf::from("/usr/local/bin/freshen");
        let xml = plist_xml(&binary, 6);
        assert!(xml.contains("<string>com.freshen.schedule</string>"));
        assert!(xml.contains("/usr/local/bin/freshen"));
        assert!(xml.contains("--check"));
        assert!(xml.contains("<integer>21600</integer>"));
        assert!(xml.contains("<key>RunAtLoad</key>"));
        assert!(xml.contains("<false/>"));
    }

    #[test]
    fn plist_xml_uses_the_default_interval() {
        let binary = PathBuf::from("/usr/local/bin/freshen");
        let xml = plist_xml(&binary, DEFAULT_EVERY_HOURS);
        assert!(xml.contains("<integer>86400</integer>"));
    }

    #[test]
    fn launch_agents_dir_joins_library_launch_agents() {
        let home = PathBuf::from("/Users/developer");
        assert_eq!(
            launch_agents_dir(&home),
            PathBuf::from("/Users/developer/Library/LaunchAgents")
        );
    }

    #[test]
    fn timer_path_is_under_systemd_user_dir() {
        let home = PathBuf::from("/home/developer");
        let path = timer_path(&home);
        assert_eq!(
            path,
            PathBuf::from("/home/developer/.config/systemd/user/com.freshen.schedule.timer")
        );
        assert!(path.ends_with("com.freshen.schedule.timer"));
    }

    #[test]
    fn service_path_is_under_systemd_user_dir() {
        let home = PathBuf::from("/home/developer");
        let path = service_path(&home);
        assert_eq!(
            path,
            PathBuf::from("/home/developer/.config/systemd/user/com.freshen.schedule.service")
        );
    }

    #[test]
    fn service_unit_runs_freshen_check() {
        let binary = PathBuf::from("/usr/bin/freshen");
        let unit = service_unit(&binary);
        assert!(unit.contains("/usr/bin/freshen --check"));
        assert!(unit.contains("Type=oneshot"));
        assert!(unit.contains("Description=freshen scheduled update check"));
    }

    #[test]
    fn timer_unit_runs_every_configured_interval() {
        let unit = timer_unit(6);
        assert!(unit.contains("OnUnitActiveSec=21600"));
        assert!(unit.contains("Unit=com.freshen.schedule.service"));
        assert!(unit.contains("WantedBy=timers.target"));
    }

    #[test]
    fn timer_unit_uses_the_default_interval() {
        let unit = timer_unit(DEFAULT_EVERY_HOURS);
        assert!(unit.contains("OnUnitActiveSec=86400"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    #[test]
    fn install_reports_not_supported_on_other_platforms() {
        let error = match install(24) {
            Err(error) => error,
            Ok(()) => panic!("expected an error, got Ok"),
        };
        let ScheduleError::NotSupported { platform } = &error else {
            panic!("expected NotSupported, got {error:?}");
        };
        assert_eq!(*platform, std::env::consts::OS);
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    #[test]
    fn remove_reports_not_supported_on_other_platforms() {
        let error = match remove() {
            Err(error) => error,
            Ok(()) => panic!("expected an error, got Ok"),
        };
        let ScheduleError::NotSupported { platform } = &error else {
            panic!("expected NotSupported, got {error:?}");
        };
        assert_eq!(*platform, std::env::consts::OS);
    }

    #[test]
    fn schedule_error_display_messages() {
        let not_supported = ScheduleError::NotSupported { platform: "linux" };
        assert_eq!(
            not_supported.to_string(),
            "scheduling is not supported on linux"
        );

        let write_failed = ScheduleError::WriteUnit {
            path: PathBuf::from("/tmp/x.timer"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        assert!(write_failed.to_string().contains("/tmp/x.timer"));
        assert!(write_failed.to_string().contains("denied"));

        let load_failed = ScheduleError::LoadFailed {
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
        };
        assert!(load_failed.to_string().contains("scheduled timer"));

        let unload_failed = ScheduleError::UnloadFailed {
            source: std::io::Error::other("boom"),
        };
        assert!(unload_failed.to_string().contains("scheduled timer"));

        let remove_failed = ScheduleError::RemoveUnit {
            path: PathBuf::from("/tmp/x.timer"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
        };
        assert!(remove_failed.to_string().contains("/tmp/x.timer"));
    }
}
