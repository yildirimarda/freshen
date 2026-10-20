//! Pre-upgrade snapshots of installed versions.
//!
//! Before a step changes anything, the runner asks it for a [`SnapshotSection`]
//! and writes the captured command output to a timestamped file. The file is a
//! plain-text record of what was installed, so a later rollback can be derived
//! from it without freshen.
//!
//! Snapshots are best effort: a command that fails is recorded with its error
//! instead of failing the step, and a file that cannot be written only produces
//! a warning. Upgrading the machine is never blocked by the record of how to
//! undo it.

use std::collections::HashSet;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::process::{CommandSpec, ProcessError};
use crate::step::StepId;

/// Environment variable that overrides the directory snapshots are written to.
pub const SNAPSHOT_DIR_ENV: &str = "FRESHEN_SNAPSHOT_DIR";

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// One read-only command whose output records installed versions.
///
/// Commands are declarations, not actions: the recorder runs them only when
/// the section's group has not been captured yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotCommand {
    program: String,
    args: Vec<String>,
    file_flag: Option<String>,
}

impl SnapshotCommand {
    /// Creates a command whose stdout is recorded.
    #[must_use]
    pub fn new<I, S>(program: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            file_flag: None,
        }
    }

    /// Marks a command that writes its output to a file instead of stdout.
    ///
    /// `flag` is prepended to a temporary path, so `"--file="` produces
    /// `--file=/tmp/...`. This is how `brew bundle dump` is captured without
    /// leaving a `Brewfile` in the user's working directory.
    #[must_use]
    pub fn writing_to_file(mut self, flag: impl Into<String>) -> Self {
        self.file_flag = Some(flag.into());
        self
    }

    /// Program that is run.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Arguments passed to the program, without any temporary file path.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.args
    }

    fn label(&self) -> String {
        let mut label = self.program.clone();
        for arg in &self.args {
            label.push(' ');
            label.push_str(arg);
        }
        label
    }

    fn run(&self) -> SnapshotEntry {
        let label = self.label();
        let Some(flag) = &self.file_flag else {
            return match CommandSpec::new(&self.program)
                .args(self.args.iter().map(String::as_str))
                .run()
            {
                Ok(output) => SnapshotEntry::new(label, output.stdout().trim_end()),
                Err(error) => SnapshotEntry::new(label, failure_text(&error)),
            };
        };

        let path = temp_file_path();
        let file_argument = format!("{flag}{}", path.display());
        let result = CommandSpec::new(&self.program)
            .args(self.args.iter().map(String::as_str))
            .arg(file_argument)
            .run();
        let output = match result {
            Ok(_) => fs::read_to_string(&path).unwrap_or_default(),
            Err(error) => failure_text(&error),
        };
        let _ = fs::remove_file(&path);

        SnapshotEntry::new(label, output.trim_end())
    }
}

/// One captured command and its output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotEntry {
    command: String,
    output: String,
}

impl SnapshotEntry {
    fn new(command: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            output: output.into(),
        }
    }
}

/// The installed versions one step can change, captured before it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSection {
    group: StepId,
    commands: Vec<SnapshotCommand>,
}

impl SnapshotSection {
    /// Creates a section for `group`, which steps sharing a package manager
    /// also use so the runner captures it only once per run.
    #[must_use]
    pub fn new(group: StepId, commands: Vec<SnapshotCommand>) -> Self {
        Self { group, commands }
    }

    /// Id shared by every step that manages the same set of installed tools.
    #[must_use]
    pub fn group(&self) -> StepId {
        self.group
    }

    /// Commands to run, in report order.
    #[must_use]
    pub fn commands(&self) -> &[SnapshotCommand] {
        &self.commands
    }
}

/// The Homebrew formulae and casks shared by every Homebrew step.
#[must_use]
pub fn homebrew_section() -> SnapshotSection {
    SnapshotSection::new(
        "brew",
        vec![
            SnapshotCommand::new("brew", ["list", "--versions"]),
            SnapshotCommand::new("brew", ["bundle", "dump", "--force"]).writing_to_file("--file="),
        ],
    )
}

/// Writes snapshot sections to one timestamped file per run.
///
/// The file is created lazily when the first section is recorded, so a run
/// that changes nothing (or a `--dry-run`) leaves no file behind. Sections are
/// deduplicated by group, so selecting several Homebrew steps still records
/// the installed formulae only once.
#[derive(Debug)]
pub struct SnapshotRecorder {
    directory: PathBuf,
    path: Option<PathBuf>,
    file: Option<File>,
    captured: HashSet<StepId>,
}

impl Default for SnapshotRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotRecorder {
    /// Creates a recorder for the platform's default snapshot directory.
    #[must_use]
    pub fn new() -> Self {
        Self::with_directory(snapshot_directory())
    }

    /// Creates a recorder that writes into `directory`.
    #[must_use]
    pub fn with_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            path: None,
            file: None,
            captured: HashSet::new(),
        }
    }

    /// Path of the snapshot file, once the first section has been written.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Returns `true` when no section has been written yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.path.is_none()
    }

    /// Runs `section`'s commands and writes them to the snapshot file unless
    /// its group was already recorded.
    ///
    /// Returns `true` when a new section was written.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error when the directory or file cannot be
    /// created or written.
    pub fn record(&mut self, section: SnapshotSection) -> io::Result<bool> {
        if !self.captured.insert(section.group) {
            return Ok(false);
        }

        self.ensure_file()?;
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("snapshot file is not open"))?;

        writeln!(file, "## {}", section.group)?;
        writeln!(file)?;
        for command in &section.commands {
            let entry = command.run();
            writeln!(file, "$ {}", entry.command)?;
            if !entry.output.is_empty() {
                writeln!(file, "{}", entry.output)?;
            }
            writeln!(file)?;
        }
        file.flush()?;

        Ok(true)
    }

    fn ensure_file(&mut self) -> io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }

        fs::create_dir_all(&self.directory)?;
        let (path, file) = create_snapshot_file(&self.directory)?;
        self.path = Some(path);
        self.file = Some(file);

        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("snapshot file is not open"))?;
        writeln!(file, "# freshen pre-upgrade snapshot")?;
        writeln!(file, "# created: {}", format_timestamp(SystemTime::now()))?;
        writeln!(file, "# freshen: {}", crate::version())?;
        writeln!(file)?;
        file.flush()?;

        Ok(())
    }
}

fn failure_text(error: &ProcessError) -> String {
    let mut text = format!("# command failed: {error}");
    if let Some(output) = error.output() {
        let stdout = output.stdout().trim_end();
        let stderr = output.stderr().trim_end();
        if !stdout.is_empty() {
            text.push('\n');
            text.push_str(stdout);
        }
        if !stderr.is_empty() {
            text.push('\n');
            text.push_str(stderr);
        }
    }
    text
}

fn temp_file_path() -> PathBuf {
    let id = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    env::temp_dir().join(format!("freshen-snapshot-{}-{id}", process::id()))
}

fn create_snapshot_file(directory: &Path) -> io::Result<(PathBuf, File)> {
    let base = format!(
        "snapshot-{}-{}",
        filename_timestamp(SystemTime::now()),
        process::id()
    );

    for attempt in 0..100 {
        let name = if attempt == 0 {
            format!("{base}.txt")
        } else {
            format!("{base}-{attempt}.txt")
        };
        let path = directory.join(name);
        match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a unique snapshot file",
    ))
}

fn snapshot_directory() -> PathBuf {
    if let Some(directory) = env::var_os(SNAPSHOT_DIR_ENV).filter(|directory| !directory.is_empty())
    {
        return PathBuf::from(directory);
    }

    if cfg!(target_os = "macos") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/freshen/snapshots");
        }
    } else if cfg!(windows) {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("freshen").join("snapshots");
        }
        if let Some(profile) = env::var_os("USERPROFILE") {
            return PathBuf::from(profile).join("AppData/Local/freshen/snapshots");
        }
    } else {
        if let Some(state) = env::var_os("XDG_STATE_HOME") {
            return PathBuf::from(state).join("freshen").join("snapshots");
        }
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(".local/state/freshen/snapshots");
        }
    }

    env::temp_dir().join("freshen").join("snapshots")
}

fn format_timestamp(now: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = timestamp_parts(now);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn filename_timestamp(now: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = timestamp_parts(now);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}-{minute:02}-{second:02}Z")
}

fn timestamp_parts(now: SystemTime) -> (i64, u32, u32, u64, u64, u64) {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let time = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);

    (
        year,
        month,
        day,
        time / 3_600,
        (time % 3_600) / 60,
        time % 60,
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::{
        civil_from_days, filename_timestamp, format_timestamp, SnapshotCommand, SnapshotRecorder,
        SnapshotSection,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, UNIX_EPOCH};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn temp_directory() -> PathBuf {
        let unique = format!(
            "freshen-snapshot-test-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        );
        std::env::temp_dir().join(unique)
    }

    fn section(group: crate::step::StepId) -> SnapshotSection {
        let program = std::env::current_exe()
            .unwrap_or_else(|error| panic!("failed to find the test binary: {error}"))
            .to_string_lossy()
            .into_owned();
        SnapshotSection::new(group, vec![SnapshotCommand::new(program, ["--help"])])
    }

    #[test]
    fn formats_a_human_readable_utc_timestamp() {
        assert_eq!(
            format_timestamp(UNIX_EPOCH),
            "1970-01-01T00:00:00Z",
            "unix epoch"
        );
        assert_eq!(
            format_timestamp(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(
            filename_timestamp(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            "2023-11-14T22-13-20Z"
        );
    }

    #[test]
    fn converts_days_to_civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_654), (2026, 7, 20));
    }

    #[test]
    fn writes_sections_to_a_timestamped_file_and_deduplicates_groups() {
        let directory = temp_directory();
        let mut recorder = SnapshotRecorder::with_directory(&directory);

        assert!(recorder.is_empty());
        assert!(recorder
            .record(section("brew"))
            .unwrap_or_else(|error| panic!("failed to record section: {error}")));
        assert!(!recorder
            .record(section("brew"))
            .unwrap_or_else(|error| panic!("failed to record section: {error}")));
        assert!(!recorder.is_empty());

        let path = recorder
            .path()
            .unwrap_or_else(|| panic!("recorder should have a path"));
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("snapshot path should be valid UTF-8"));
        assert!(name.starts_with("snapshot-"), "name: {name}");
        assert!(name.ends_with(".txt"), "name: {name}");

        let text = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read snapshot: {error}"));
        assert!(text.contains("# freshen pre-upgrade snapshot"), "{text}");
        assert!(text.contains("## brew"), "{text}");
        assert!(text.contains("--help"), "{text}");
        assert!(text.contains("Usage"), "{text}");

        fs::remove_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to remove test directory: {error}"));
    }

    #[test]
    fn writes_a_new_file_for_each_recorder() {
        let directory = temp_directory();

        let mut first = SnapshotRecorder::with_directory(&directory);
        let mut second = SnapshotRecorder::with_directory(&directory);
        assert!(first
            .record(section("uv"))
            .unwrap_or_else(|error| panic!("failed to record section: {error}")));
        assert!(second
            .record(section("uv"))
            .unwrap_or_else(|error| panic!("failed to record section: {error}")));
        assert_ne!(first.path(), second.path());

        fs::remove_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to remove test directory: {error}"));
    }

    #[test]
    fn sections_expose_their_group_and_commands() {
        let section = SnapshotSection::new(
            "mise",
            vec![SnapshotCommand::new("mise", ["ls"]).writing_to_file("--file=")],
        );

        assert_eq!(section.group(), "mise");
        assert_eq!(section.commands().len(), 1);
        assert_eq!(section.commands()[0].program(), "mise");
        assert_eq!(section.commands()[0].arguments(), ["ls"]);
    }
}
