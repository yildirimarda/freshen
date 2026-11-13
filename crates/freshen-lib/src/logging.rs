//! Log files with rotation, written to the OS-appropriate location.
//!
//! freshen appends one line per logged event to `freshen.log`. When the file
//! grows beyond a size limit it is rotated: the current file becomes
//! `freshen.log.1`, an existing `.1` becomes `.2`, and so on, keeping only the
//! most recent files so the logs never grow without bound.
//!
//! | Platform | Log directory |
//! |---|---|
//! | macOS, Linux | `$XDG_CACHE_HOME/freshen/logs`, else `~/.cache/freshen/logs` |
//! | Windows | `%LOCALAPPDATA%\freshen\logs`, else `%APPDATA%\freshen\logs` |
//!
//! The directory rules are testable on every platform because they are driven
//! by an explicit environment struct rather than by `cfg!`-gated calls to the
//! real process environment.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::platform::Platform;

/// Name of the directory that holds the log files.
pub const LOG_DIR_NAME: &str = "freshen";

/// Name of the current log file inside that directory.
pub const LOG_FILE_NAME: &str = "freshen.log";

/// Default maximum size of the current log file before it is rotated (10 MiB).
pub const DEFAULT_MAX_SIZE: u64 = 10 * 1024 * 1024;

/// Default number of rotated log files to keep alongside the current one.
pub const DEFAULT_MAX_FILES: usize = 5;

/// The environment values that decide where the log files live.
///
/// Reading them from a struct instead of from the process environment keeps
/// the location rules testable on every platform.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoggingEnvironment {
    /// `XDG_CACHE_HOME`: the XDG cache directory (macOS, Linux).
    pub xdg_cache_home: Option<PathBuf>,

    /// The home directory of the user.
    pub home: Option<PathBuf>,

    /// `%LOCALAPPDATA%`: the local application data directory on Windows.
    pub local_app_data: Option<PathBuf>,

    /// `%APPDATA%`: the roaming application data directory on Windows.
    pub appdata: Option<PathBuf>,
}

impl LoggingEnvironment {
    /// Reads the values from the environment of the running process.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            xdg_cache_home: env_path("XDG_CACHE_HOME"),
            home: env_path("HOME").or_else(|| env_path("USERPROFILE")),
            local_app_data: env_path("LOCALAPPDATA"),
            appdata: env_path("APPDATA"),
        }
    }

    /// The directory that holds the log files on `platform`.
    #[must_use]
    pub fn log_dir(&self, platform: Platform) -> PathBuf {
        let base = match platform {
            Platform::Windows => self
                .local_app_data
                .clone()
                .or_else(|| self.appdata.clone())
                .or_else(|| {
                    self.home
                        .as_ref()
                        .map(|home| home.join("AppData").join("Local"))
                })
                .or_else(|| {
                    self.home
                        .as_ref()
                        .map(|home| home.join("AppData").join("Roaming"))
                })
                .unwrap_or_else(env::temp_dir),
            Platform::Macos | Platform::Linux => self
                .xdg_cache_home
                .clone()
                .or_else(|| self.home.as_ref().map(|home| home.join(".cache")))
                .unwrap_or_else(env::temp_dir),
        };

        base.join(LOG_DIR_NAME).join("logs")
    }

    /// The current log file on `platform`.
    #[must_use]
    pub fn log_path(&self, platform: Platform) -> PathBuf {
        self.log_dir(platform).join(LOG_FILE_NAME)
    }
}

/// The log file of the running platform.
///
/// Returns `None` when freshen does not support the operating system it was
/// built for.
#[must_use]
pub fn log_path() -> Option<PathBuf> {
    Platform::current().map(|platform| LoggingEnvironment::from_env().log_path(platform))
}

/// Returns the log lines of the most recent run from the log file at `path`.
///
/// A run begins at a line that announces the start of a freshen invocation
/// ("freshen <version> started"). Everything from the last such line to the
/// end of the file is the last run. When the file contains no such line, the
/// whole file is the last run.
///
/// Returns `None` when the file does not exist or is empty.
///
/// # Errors
///
/// Returns the underlying I/O error when the file cannot be read.
pub fn last_run(path: &Path) -> io::Result<Option<String>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    let lines: Vec<&str> = contents.lines().collect();
    let start = lines
        .iter()
        .rposition(|line| is_run_start(line))
        .unwrap_or(0);
    let last_run = lines[start..].join("\n");
    if last_run.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!("{last_run}\n")))
}

/// Returns the log lines of the most recent run on the running platform, or
/// `None` when there is no log file or the platform is unsupported.
pub fn last_run_log() -> Option<String> {
    let path = log_path()?;
    last_run(&path).ok().flatten()
}

/// Returns `true` when `line` announces the start of a freshen run.
fn is_run_start(line: &str) -> bool {
    line.starts_with("freshen ") && line.contains(" started")
}

/// A problem that prevented a log file from being used.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    /// The log directory could not be created.
    #[error("could not create the log directory {path}: {source}")]
    CreateDirectory {
        /// Directory that could not be created.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// The log file could not be opened.
    #[error("could not open the log file {path}: {source}")]
    Open {
        /// Path of the log file.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },
}

/// One rotating log file.
///
/// Lines are appended to the current file. When appending a line would make
/// the file exceed [`LogFile::max_size`], the file is rotated first so only
/// [`LogFile::max_files`] files are kept at any time. A line that is larger
/// than the limit is still written: rotation only makes room, it never drops
/// a line.
#[derive(Debug)]
pub struct LogFile {
    /// Path of the current log file.
    path: PathBuf,
    /// The file handle for the current log file, opened in append mode.
    file: File,
    /// Current size of the current log file in bytes.
    size: u64,
    /// Rotate when the file would exceed this many bytes.
    max_size: u64,
    /// Keep at most this many files (current plus rotated). `1` disables
    /// rotation.
    max_files: usize,
}

impl LogFile {
    /// Opens (or creates) `path` as the current log file with the default
    /// size and retention limits.
    ///
    /// The parent directory is created when it does not exist. Opening never
    /// truncates an existing log file.
    ///
    /// # Errors
    ///
    /// Returns [`LogError`] when the directory or the file cannot be created.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, LogError> {
        Self::open_with(path, DEFAULT_MAX_SIZE, DEFAULT_MAX_FILES)
    }

    /// Opens (or creates) `path` as the current log file with explicit limits.
    ///
    /// `max_files` of `1` keeps a single, ever-growing file and disables
    /// rotation. Values below `1` are treated as `1`.
    ///
    /// # Errors
    ///
    /// Returns [`LogError`] when the directory or the file cannot be created.
    pub fn open_with(
        path: impl Into<PathBuf>,
        max_size: u64,
        max_files: usize,
    ) -> Result<Self, LogError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| LogError::CreateDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| LogError::Open {
                path: path.clone(),
                source,
            })?;

        let size = file
            .metadata()
            .map_err(|source| LogError::Open {
                path: path.clone(),
                source,
            })?
            .len();

        Ok(Self {
            path,
            file,
            size,
            max_size,
            max_files: max_files.max(1),
        })
    }

    /// Writes `line` followed by a newline to the current log file, rotating
    /// first when the file would exceed its size limit.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error when the line cannot be written.
    pub fn write(&mut self, line: &str) -> io::Result<()> {
        let line_len = line.len() as u64;
        let needed = self.size.saturating_add(line_len).saturating_add(1); // + newline

        if self.max_files > 1 && needed > self.max_size {
            self.rotate()?;
        }

        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.size = self.size.saturating_add(line_len).saturating_add(1);
        self.file.flush()?;
        Ok(())
    }

    /// Returns the path of the current log file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the current size of the current log file in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Forces a rotation now: the current file becomes `.1` and a fresh empty
    /// current file is created, keeping at most `max_files` files.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error when a file cannot be renamed or
    /// created.
    pub fn rotate(&mut self) -> io::Result<()> {
        // Remove the oldest rotated file so the total stays within max_files.
        let oldest = self.rotated_path(self.max_files - 1);
        let _ = fs::remove_file(&oldest);

        // Shift every file down by one: current -> .1, .1 -> .2, ...
        for index in (1..self.max_files).rev() {
            let from = if index == 1 {
                self.path.clone()
            } else {
                self.rotated_path(index - 1)
            };
            let to = self.rotated_path(index);
            if from.exists() {
                fs::rename(from, to)?;
            }
        }

        // Start a fresh current file.
        self.file = File::create(&self.path)?;
        self.size = 0;
        Ok(())
    }

    /// Path of the rotated file with the given index (`.1`, `.2`, ...).
    ///
    /// `with_extension` replaces the text after the last dot, so the current
    /// file `freshen.log` becomes `freshen.log.1`.
    fn rotated_path(&self, index: usize) -> PathBuf {
        self.path.with_extension(format!("log.{index}"))
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = env::var_os(key)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use super::{last_run, LogFile, LoggingEnvironment, LOG_DIR_NAME, LOG_FILE_NAME};
    use crate::platform::Platform;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    fn environment() -> LoggingEnvironment {
        LoggingEnvironment {
            home: Some(PathBuf::from("/home/user")),
            ..LoggingEnvironment::default()
        }
    }

    #[test]
    fn the_log_lives_in_the_cache_directory_on_unix() {
        let environment = environment();

        for platform in [Platform::Macos, Platform::Linux] {
            assert_eq!(
                environment.log_path(platform),
                PathBuf::from("/home/user/.cache")
                    .join(LOG_DIR_NAME)
                    .join("logs")
                    .join(LOG_FILE_NAME),
                "{platform}"
            );
        }
    }

    #[test]
    fn xdg_cache_home_wins_over_the_home_directory() {
        let environment = LoggingEnvironment {
            xdg_cache_home: Some(PathBuf::from("/home/user/cache")),
            home: Some(PathBuf::from("/home/user")),
            ..LoggingEnvironment::default()
        };

        assert_eq!(
            environment.log_path(Platform::Linux),
            PathBuf::from("/home/user/cache")
                .join(LOG_DIR_NAME)
                .join("logs")
                .join(LOG_FILE_NAME)
        );
    }

    #[test]
    fn the_log_lives_in_local_appdata_on_windows() {
        let environment = LoggingEnvironment {
            local_app_data: Some(PathBuf::from(r"C:\Users\user\AppData\Local")),
            home: Some(PathBuf::from(r"C:\Users\user")),
            ..LoggingEnvironment::default()
        };

        assert_eq!(
            environment.log_path(Platform::Windows),
            PathBuf::from(r"C:\Users\user\AppData\Local")
                .join(LOG_DIR_NAME)
                .join("logs")
                .join(LOG_FILE_NAME)
        );
    }

    #[test]
    fn appdata_is_used_when_local_appdata_is_missing() {
        let environment = LoggingEnvironment {
            appdata: Some(PathBuf::from(r"C:\Users\user\AppData\Roaming")),
            home: Some(PathBuf::from(r"C:\Users\user")),
            ..LoggingEnvironment::default()
        };

        assert_eq!(
            environment.log_path(Platform::Windows),
            PathBuf::from(r"C:\Users\user\AppData\Roaming")
                .join(LOG_DIR_NAME)
                .join("logs")
                .join(LOG_FILE_NAME)
        );
    }

    #[test]
    fn windows_falls_back_to_the_local_directory_of_the_home_directory() {
        let environment = LoggingEnvironment {
            home: Some(PathBuf::from(r"C:\Users\user")),
            ..LoggingEnvironment::default()
        };

        assert_eq!(
            environment.log_path(Platform::Windows),
            PathBuf::from(r"C:\Users\user")
                .join("AppData")
                .join("Local")
                .join(LOG_DIR_NAME)
                .join("logs")
                .join(LOG_FILE_NAME)
        );
    }

    #[test]
    fn every_missing_variable_falls_back_to_the_temp_directory() {
        let environment = LoggingEnvironment::default();

        let expected = env::temp_dir()
            .join(LOG_DIR_NAME)
            .join("logs")
            .join(LOG_FILE_NAME);

        for platform in Platform::ALL {
            assert_eq!(environment.log_path(platform), expected, "{platform}");
        }
    }

    #[test]
    fn the_log_file_of_the_running_platform_is_found() {
        let expected =
            Platform::current().map(|platform| LoggingEnvironment::from_env().log_path(platform));

        assert_eq!(super::log_path(), expected);
    }

    #[test]
    fn writing_appends_lines_in_order() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        let mut log = LogFile::open(&path).unwrap_or_else(|error| panic!("{error:?}"));

        log.write("first")
            .unwrap_or_else(|error| panic!("{error:?}"));
        log.write("second")
            .unwrap_or_else(|error| panic!("{error:?}"));

        let contents = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(contents, "first\nsecond\n");
    }

    #[test]
    fn the_directory_is_created_when_it_is_missing() {
        let directory = temp_directory().join("nested").join("logs");
        let path = directory.join(LOG_FILE_NAME);

        LogFile::open(&path).unwrap_or_else(|error| panic!("{error:?}"));

        assert!(directory.is_dir());
        assert!(path.is_file());
    }

    #[test]
    fn an_existing_log_file_is_not_truncated() {
        let directory = temp_directory();
        fs::create_dir_all(&directory).unwrap_or_else(|error| panic!("{error:?}"));
        let path = directory.join(LOG_FILE_NAME);
        fs::write(&path, "existing\n").unwrap_or_else(|error| panic!("{error:?}"));

        let mut log = LogFile::open(&path).unwrap_or_else(|error| panic!("{error:?}"));
        log.write("added")
            .unwrap_or_else(|error| panic!("{error:?}"));

        let contents = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(contents, "existing\nadded\n");
    }

    #[test]
    fn rotation_moves_the_current_file_aside_and_starts_a_fresh_one() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        let mut log = LogFile::open_with(&path, 40, 3).unwrap_or_else(|error| panic!("{error:?}"));

        log.write("aaaaaaaaaaaaaaaaaaaa")
            .unwrap_or_else(|error| panic!("{error:?}")); // 20 bytes + newline
        log.write("bbbbbbbbbbbbbbbbbbbb")
            .unwrap_or_else(|error| panic!("{error:?}")); // 20 bytes + newline, rotates the first aside

        let rotated = directory.join(format!("{LOG_FILE_NAME}.1"));
        assert!(rotated.exists());
        assert_eq!(
            fs::read_to_string(&rotated).unwrap_or_else(|error| panic!("{error:?}")),
            "aaaaaaaaaaaaaaaaaaaa\n"
        );

        // A write that exceeds the limit rotates first.
        log.write("cccccccccccccccccccccccccccccccc")
            .unwrap_or_else(|error| panic!("{error:?}")); // 32 bytes + newline

        assert!(directory.join(format!("{LOG_FILE_NAME}.2")).exists());
        assert_eq!(
            fs::read_to_string(directory.join(format!("{LOG_FILE_NAME}.2")))
                .unwrap_or_else(|error| panic!("{error:?}")),
            "aaaaaaaaaaaaaaaaaaaa\n"
        );
        assert_eq!(
            fs::read_to_string(directory.join(format!("{LOG_FILE_NAME}.1")))
                .unwrap_or_else(|error| panic!("{error:?}")),
            "bbbbbbbbbbbbbbbbbbbb\n"
        );

        let current = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(current, "cccccccccccccccccccccccccccccccc\n");
    }

    #[test]
    fn rotation_shifts_previous_rotated_files_down() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        let mut log = LogFile::open_with(&path, 4, 3).unwrap_or_else(|error| panic!("{error:?}"));

        log.write("one").unwrap_or_else(|error| panic!("{error:?}")); // rotates straight into the fresh file
        log.write("two").unwrap_or_else(|error| panic!("{error:?}")); // rotates: current -> .1
        log.write("three")
            .unwrap_or_else(|error| panic!("{error:?}")); // rotates: .1 -> .2, current -> .1

        let level1 = directory.join(format!("{LOG_FILE_NAME}.1"));
        let level2 = directory.join(format!("{LOG_FILE_NAME}.2"));
        assert!(level1.exists());
        assert!(level2.exists());

        let current = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(current, "three\n");

        assert_eq!(
            fs::read_to_string(&level1).unwrap_or_else(|error| panic!("{error:?}")),
            "two\n"
        );
        assert_eq!(
            fs::read_to_string(&level2).unwrap_or_else(|error| panic!("{error:?}")),
            "one\n"
        );
    }

    #[test]
    fn rotation_keeps_at_most_max_files() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        let mut log = LogFile::open_with(&path, 5, 3).unwrap_or_else(|error| panic!("{error:?}")); // current + .1 + .2

        for message in ["a", "b", "c", "d", "e"] {
            log.write(message)
                .unwrap_or_else(|error| panic!("{error:?}")); // each write rotates (2 bytes > ... no)
        }

        // Keep: current plus at most max_files - 1 rotated files.
        let mut rotated: Vec<PathBuf> = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("{error:?}"))
            .map(|entry| entry.unwrap_or_else(|error| panic!("{error:?}")).path())
            .filter(|path| {
                path.file_name()
                    .unwrap_or_else(|| panic!("missing file name"))
                    .to_str()
                    .unwrap_or_else(|| panic!("non-utf8 file name"))
                    .starts_with(LOG_FILE_NAME)
            })
            .collect();
        rotated.sort();

        assert!(
            rotated.len() <= 3,
            "expected at most 3 log files, found {}: {rotated:?}",
            rotated.len()
        );
        assert!(rotated
            .iter()
            .any(|path| *path == directory.join(LOG_FILE_NAME)));
    }

    #[test]
    fn a_single_file_is_never_rotated() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        let mut log = LogFile::open_with(&path, 1, 1).unwrap_or_else(|error| panic!("{error:?}"));

        log.write("short")
            .unwrap_or_else(|error| panic!("{error:?}"));
        log.write("this-is-a-very-long-line-that-exceeds-the-limit")
            .unwrap_or_else(|error| panic!("{error:?}"));

        assert!(!directory.join(format!("{LOG_FILE_NAME}.1")).exists());
        let contents = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(
            contents,
            "short\nthis-is-a-very-long-line-that-exceeds-the-limit\n"
        );
    }

    #[test]
    fn a_line_larger_than_the_limit_is_still_written() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        let mut log = LogFile::open_with(&path, 5, 3).unwrap_or_else(|error| panic!("{error:?}"));

        log.write("this-line-is-much-longer-than-the-limit")
            .unwrap_or_else(|error| panic!("{error:?}"));

        let contents = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(contents, "this-line-is-much-longer-than-the-limit\n");
    }

    #[test]
    fn last_run_returns_the_lines_of_the_most_recent_run() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        fs::create_dir_all(&directory).unwrap_or_else(|error| panic!("{error:?}"));
        fs::write(
            &path,
            "freshen 0.1.0 started\nfirst run event\nrun completed with exit code 0\n\
         freshen 0.1.0 started\nsecond run event\nrun completed with exit code 1\n",
        )
        .unwrap_or_else(|error| panic!("failed to write {path:?}: {error}"));

        let last_run = last_run(&path)
            .unwrap_or_else(|error| panic!("failed to read {path:?}: {error}"))
            .unwrap_or_else(|| panic!("expected a last run"));
        assert_eq!(
            last_run, "freshen 0.1.0 started\nsecond run event\nrun completed with exit code 1\n",
            "last run: {last_run:?}"
        );
    }

    #[test]
    fn last_run_returns_the_whole_file_when_there_is_no_run_start() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        fs::create_dir_all(&directory).unwrap_or_else(|error| panic!("{error:?}"));
        fs::write(&path, "some orphaned line\n")
            .unwrap_or_else(|error| panic!("failed to write {path:?}: {error}"));

        let last_run =
            last_run(&path).unwrap_or_else(|error| panic!("failed to read {path:?}: {error}"));
        assert_eq!(last_run, Some("some orphaned line\n".to_owned()));
    }

    #[test]
    fn last_run_is_none_when_the_log_file_is_missing() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        assert_eq!(
            last_run(&path).unwrap_or_else(|error| panic!("failed to read {path:?}: {error}")),
            None
        );
    }

    #[test]
    fn last_run_is_none_when_the_log_file_is_empty() {
        let directory = temp_directory();
        let path = directory.join(LOG_FILE_NAME);
        fs::create_dir_all(&directory).unwrap_or_else(|error| panic!("{error:?}"));
        fs::write(&path, "").unwrap_or_else(|error| panic!("failed to write {path:?}: {error}"));
        assert_eq!(
            last_run(&path).unwrap_or_else(|error| panic!("failed to read {path:?}: {error}")),
            None
        );
    }

    fn temp_directory() -> PathBuf {
        let unique = format!(
            "freshen-log-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }
}
