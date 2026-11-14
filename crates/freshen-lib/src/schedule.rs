//! Scheduling unattended freshen runs.
//!
//! On macOS, [`install`] writes a per-user launchd agent property list to
//! `~/Library/LaunchAgents/com.freshen.schedule.plist` and loads it with
//! `launchctl`, so `freshen` runs on a repeating interval. [`remove`] unloads
//! and deletes the agent.
//!
//! The agent runs `freshen --check` so a scheduled run reports available
//! updates without changing anything. Scheduling is a macOS-only feature: on
//! every other platform [`install`] and [`remove`] report that it is not
//! supported.

use std::fmt;
#[cfg(any(test, target_os = "macos"))]
use std::path::Path;
use std::path::PathBuf;

/// The label of the scheduled launchd agent.
pub const AGENT_LABEL: &str = "com.freshen.schedule";

/// The property-list file name of the agent.
#[cfg(any(test, target_os = "macos"))]
pub(crate) const PLIST_FILE_NAME: &str = "com.freshen.schedule.plist";

/// How often the scheduled agent runs by default, in hours.
#[cfg(any(test, target_os = "macos"))]
pub(crate) const DEFAULT_EVERY_HOURS: u64 = 24;

/// A problem installing or removing the scheduled agent.
#[derive(Debug)]
pub enum ScheduleError {
    /// Scheduling is not supported on this operating system.
    NotSupported { platform: &'static str },
    /// The agent property list could not be written.
    WritePlist {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The agent could not be loaded with `launchctl`.
    LoadFailed { source: std::io::Error },
    /// The agent could not be unloaded with `launchctl`.
    UnloadFailed { source: std::io::Error },
    /// The agent property list could not be removed.
    RemovePlist {
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
            Self::WritePlist { path, source } => write!(
                f,
                "could not write the scheduled agent property list to {}: {source}",
                path.display()
            ),
            Self::LoadFailed { source } => {
                write!(
                    f,
                    "could not load the scheduled agent with launchctl: {source}"
                )
            }
            Self::UnloadFailed { source } => {
                write!(
                    f,
                    "could not unload the scheduled agent with launchctl: {source}"
                )
            }
            Self::RemovePlist { path, source } => write!(
                f,
                "could not remove the scheduled agent property list {}: {source}",
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

/// Installs (writes and loads) the scheduled launchd agent.
///
/// On macOS this writes the property list and loads it with `launchctl`. On
/// every other platform it reports that scheduling is not supported.
pub fn install(every_hours: u64) -> Result<(), ScheduleError> {
    #[cfg(target_os = "macos")]
    {
        install_macos(every_hours)
    }
    #[cfg(not(target_os = "macos"))]
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
/// list. On every other platform it reports that scheduling is not supported.
pub fn remove() -> Result<(), ScheduleError> {
    #[cfg(target_os = "macos")]
    {
        remove_macos()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(ScheduleError::NotSupported {
            platform: std::env::consts::OS,
        })
    }
}

/// Writes the agent property list and loads it with `launchctl`.
#[cfg(target_os = "macos")]
fn install_macos(every_hours: u64) -> Result<(), ScheduleError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| ScheduleError::NotSupported { platform: "macos" })?;
    let home = PathBuf::from(home);
    let directory = launch_agents_dir(&home);
    std::fs::create_dir_all(&directory).map_err(|source| ScheduleError::WritePlist {
        path: plist_path(&home),
        source,
    })?;
    let binary = std::env::current_exe().map_err(|source| ScheduleError::WritePlist {
        path: plist_path(&home),
        source,
    })?;
    let plist = plist_xml(&binary, every_hours);
    let path = plist_path(&home);
    std::fs::write(&path, plist).map_err(|source| ScheduleError::WritePlist { path, source })?;
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
    let home = std::env::var_os("HOME")
        .ok_or_else(|| ScheduleError::NotSupported { platform: "macos" })?;
    let home = PathBuf::from(home);
    let path = plist_path(&home);
    // An agent that was never loaded cannot be unloaded; ignore that error so
    // removing an absent agent still deletes any leftover property list.
    let _ = crate::process::CommandSpec::new("launchctl")
        .arg("unload")
        .arg(path.to_string_lossy().into_owned())
        .run();
    std::fs::remove_file(&path).map_err(|source| ScheduleError::RemovePlist { path, source })?;
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

    #[cfg(not(target_os = "macos"))]
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

    #[cfg(not(target_os = "macos"))]
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

        let write_failed = ScheduleError::WritePlist {
            path: PathBuf::from("/tmp/x.plist"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        assert!(write_failed.to_string().contains("/tmp/x.plist"));
        assert!(write_failed.to_string().contains("denied"));

        let load_failed = ScheduleError::LoadFailed {
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
        };
        assert!(load_failed.to_string().contains("launchctl"));

        let unload_failed = ScheduleError::UnloadFailed {
            source: std::io::Error::other("boom"),
        };
        assert!(unload_failed.to_string().contains("launchctl"));

        let remove_failed = ScheduleError::RemovePlist {
            path: PathBuf::from("/tmp/x.plist"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
        };
        assert!(remove_failed.to_string().contains("/tmp/x.plist"));
    }
}
