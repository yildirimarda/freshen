//! A single-run lock so two freshen runs cannot change the machine at once.
//!
//! The lock is an advisory file lock held for the whole update, not just while
//! a step runs. The operating system releases it when the process exits, even
//! after a crash or a signal, so a stale file left behind never blocks a later
//! run. The file itself is kept (not removed) on purpose: unlinking it could
//! let a third process lock a fresh inode while another still waits on the old
//! one.
//!
//! The file records the PID of the run that holds it so the second run can
//! name the process in its error message. Windows locks are mandatory and
//! reject reads of the locked range, so the PID is also written to an
//! unlocked sidecar file that a contended run can read.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::Duration;

use fs2::FileExt;
use thiserror::Error;

/// Environment variable that overrides the directory holding the lock file.
pub const LOCK_DIR_ENV: &str = "FRESHEN_LOCK_DIR";

/// Name of the lock file inside the lock directory.
pub const LOCK_FILE_NAME: &str = "freshen.lock";

/// Name of the sidecar file that records the holder's PID.
pub const PID_FILE_NAME: &str = "freshen.lock.pid";

/// How many times a contended run retries reading the holder's PID.
const PID_READ_ATTEMPTS: usize = 20;

/// Delay between attempts to read the holder's PID.
const PID_READ_DELAY: Duration = Duration::from_millis(5);

/// Why a run could not take the lock.
#[derive(Debug, Error)]
pub enum LockError {
    /// Another run holds the lock and its PID is known.
    #[error("another freshen run is in progress (PID {pid} holds {path})")]
    Held {
        /// Process id of the run holding the lock.
        pid: u32,
        /// Path of the lock file.
        path: PathBuf,
    },

    /// Another run holds the lock but its PID could not be read.
    #[error("another freshen run is in progress ({path} is locked)")]
    HeldUnknown {
        /// Path of the lock file.
        path: PathBuf,
    },

    /// The lock directory could not be created.
    #[error("could not create the lock directory {path}: {source}")]
    CreateDirectory {
        /// Directory that could not be created.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },

    /// The lock file could not be opened.
    #[error("could not open the lock file {path}: {source}")]
    Open {
        /// Path of the lock file.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },

    /// The lock file could not be locked.
    #[error("could not lock {path}: {source}")]
    Lock {
        /// Path of the lock file.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },

    /// The lock was taken but the PID could not be written to the file.
    #[error("could not write the lock file {path}: {source}")]
    Write {
        /// Path of the lock file.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
}

/// The lock held by one update run.
///
/// Dropping the value releases the lock. The lock file is intentionally left
/// in place; the operating system lock, not the file's existence, decides
/// whether a run may start.
#[derive(Debug)]
#[must_use = "the lock is released when it is dropped"]
pub struct RunLock {
    path: PathBuf,
    pid_path: PathBuf,
    file: File,
}

impl RunLock {
    /// Takes the lock in the platform's default lock directory.
    ///
    /// # Errors
    ///
    /// Returns [`LockError`] when another run holds the lock or when the lock
    /// file cannot be created, opened or locked.
    pub fn acquire() -> Result<Self, LockError> {
        Self::acquire_in(lock_directory())
    }

    /// Takes the lock in `directory`, creating it when needed.
    ///
    /// # Errors
    ///
    /// Returns [`LockError`] when another run holds the lock or when the lock
    /// file cannot be created, opened or locked.
    pub fn acquire_in(directory: impl Into<PathBuf>) -> Result<Self, LockError> {
        let directory = directory.into();
        fs::create_dir_all(&directory).map_err(|source| LockError::CreateDirectory {
            path: directory.clone(),
            source,
        })?;

        let path = directory.join(LOCK_FILE_NAME);
        let pid_path = directory.join(PID_FILE_NAME);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| LockError::Open {
                path: path.clone(),
                source,
            })?;

        if let Err(error) = file.try_lock_exclusive() {
            return Err(if is_contended(&error) {
                match read_pid(&pid_path) {
                    Some(pid) => LockError::Held { pid, path },
                    None => LockError::HeldUnknown { path },
                }
            } else {
                LockError::Lock {
                    path,
                    source: error,
                }
            });
        }

        file.set_len(0).map_err(|source| LockError::Write {
            path: path.clone(),
            source,
        })?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| LockError::Write {
                path: path.clone(),
                source,
            })?;
        writeln!(file, "{}", process::id()).map_err(|source| LockError::Write {
            path: path.clone(),
            source,
        })?;
        file.flush().map_err(|source| LockError::Write {
            path: path.clone(),
            source,
        })?;

        fs::write(&pid_path, format!("{}\n", process::id())).map_err(|source| {
            LockError::Write {
                path: pid_path.clone(),
                source,
            }
        })?;

        Ok(Self {
            path,
            pid_path,
            file,
        })
    }

    /// Path of the lock file held by this run.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Path of the sidecar file recording this run's PID.
    #[must_use]
    pub fn pid_path(&self) -> &Path {
        &self.pid_path
    }

    /// The PID recorded in the lock file, read through the locked handle.
    ///
    /// Windows locks the byte range the run holds, so a second handle cannot
    /// read the file at all; the handle that took the lock can.
    #[must_use]
    pub fn recorded_pid(&self) -> Option<u32> {
        let mut file = &self.file;
        file.seek(SeekFrom::Start(0)).ok()?;
        let mut contents = String::new();
        file.read_to_string(&mut contents).ok()?;
        contents.trim().parse().ok()
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn is_contended(error: &io::Error) -> bool {
    let contended = fs2::lock_contended_error();
    error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error().is_some() && error.raw_os_error() == contended.raw_os_error()
}

fn read_pid(path: &Path) -> Option<u32> {
    for attempt in 0..PID_READ_ATTEMPTS {
        if attempt > 0 {
            thread::sleep(PID_READ_DELAY);
        }

        if let Ok(contents) = fs::read_to_string(path) {
            if let Ok(pid) = contents.trim().parse::<u32>() {
                return Some(pid);
            }
        }
    }

    None
}

fn lock_directory() -> PathBuf {
    if let Some(directory) = env::var_os(LOCK_DIR_ENV).filter(|directory| !directory.is_empty()) {
        return PathBuf::from(directory);
    }

    if cfg!(target_os = "macos") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/freshen");
        }
    } else if cfg!(windows) {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("freshen");
        }
        if let Some(profile) = env::var_os("USERPROFILE") {
            return PathBuf::from(profile).join("AppData/Local/freshen");
        }
    } else {
        if let Some(state) = env::var_os("XDG_STATE_HOME") {
            return PathBuf::from(state).join("freshen");
        }
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(".local/state/freshen");
        }
    }

    env::temp_dir().join("freshen")
}

#[cfg(test)]
mod tests {
    use super::{LockError, RunLock, LOCK_FILE_NAME, PID_FILE_NAME};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn temp_directory() -> PathBuf {
        let unique = format!(
            "freshen-lock-test-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn a_second_lock_names_the_pid_that_holds_it() {
        let directory = temp_directory();
        let first = RunLock::acquire_in(&directory)
            .unwrap_or_else(|error| panic!("failed to take the first lock: {error}"));
        assert_eq!(first.pid_path(), directory.join(PID_FILE_NAME));
        let recorded = fs::read_to_string(first.pid_path())
            .unwrap_or_else(|error| panic!("failed to read the PID sidecar: {error}"));
        assert_eq!(recorded.trim(), std::process::id().to_string());

        let error = match RunLock::acquire_in(&directory) {
            Ok(_) => panic!("the second lock must not be granted"),
            Err(error) => error,
        };

        match &error {
            LockError::Held { pid, path } => {
                assert_eq!(*pid, std::process::id());
                assert_eq!(path, &directory.join(LOCK_FILE_NAME));
                let message = error.to_string();
                assert!(message.contains(&format!("PID {pid}")), "{message}");
            }
            other => panic!("expected a held lock error, got {other:?}"),
        }

        drop(first);
        fs::remove_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to remove test directory: {error}"));
    }

    #[test]
    fn dropping_the_lock_lets_the_next_run_start() {
        let directory = temp_directory();
        let first = RunLock::acquire_in(&directory)
            .unwrap_or_else(|error| panic!("failed to take the first lock: {error}"));
        drop(first);

        let second = RunLock::acquire_in(&directory)
            .unwrap_or_else(|error| panic!("failed to take the lock after release: {error}"));
        assert_eq!(second.path(), directory.join(LOCK_FILE_NAME));

        drop(second);
        fs::remove_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to remove test directory: {error}"));
    }

    #[test]
    fn a_stale_lock_file_from_a_dead_run_does_not_block() {
        let directory = temp_directory();
        fs::create_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to create test directory: {error}"));
        let path = directory.join(LOCK_FILE_NAME);
        fs::write(&path, "999999\n")
            .unwrap_or_else(|error| panic!("failed to write stale lock: {error}"));
        fs::write(directory.join(PID_FILE_NAME), "999999\n")
            .unwrap_or_else(|error| panic!("failed to write stale PID sidecar: {error}"));

        let lock = RunLock::acquire_in(&directory)
            .unwrap_or_else(|error| panic!("stale lock must not block a run: {error}"));
        assert_eq!(lock.path(), path);
        // The lock file is locked for the whole run, so it is read through the
        // handle that holds the lock instead of through a second one.
        assert_eq!(
            lock.recorded_pid(),
            Some(std::process::id()),
            "the lock file must record the PID of the run that holds it"
        );
        let recorded = fs::read_to_string(lock.pid_path())
            .unwrap_or_else(|error| panic!("failed to read PID sidecar: {error}"));
        assert_eq!(recorded.trim(), std::process::id().to_string());

        drop(lock);
        fs::remove_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to remove test directory: {error}"));
    }
}
