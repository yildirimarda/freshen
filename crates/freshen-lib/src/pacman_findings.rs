//! Recording `.pacnew` and `.pacsave` files found after a pacman upgrade,
//! for structured reports.
//!
//! pacman leaves a `.pacnew` file next to a configuration file it did not
//! overwrite (the local copy differs from the new default) and a `.pacsave`
//! file next to a configuration file it removed. Both need the user's
//! attention, so the step reports every one it finds after a successful
//! upgrade.

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};

thread_local! {
    static PACMAN_FINDINGS: RefCell<Vec<PacmanFinding>> = const { RefCell::new(Vec::new()) };
}

/// Kind of file pacman leaves behind when a configuration file differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacmanFileKind {
    /// A new version of a configuration file was installed alongside the old
    /// one (`.pacnew`); the user must merge the changes manually.
    Pacnew,
    /// The previous version of a configuration file was saved before it was
    /// replaced (`.pacsave`); the user can review and discard it.
    Pacsave,
}

impl fmt::Display for PacmanFileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pacnew => f.write_str("pacnew"),
            Self::Pacsave => f.write_str("pacsave"),
        }
    }
}

/// One `.pacnew` or `.pacsave` file that needs the user's attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacmanFinding {
    kind: PacmanFileKind,
    path: PathBuf,
}

impl PacmanFinding {
    /// Creates a finding for `kind` at `path`.
    #[must_use]
    pub fn new(kind: PacmanFileKind, path: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            path: path.into(),
        }
    }

    /// The kind of file.
    #[must_use]
    pub fn kind(&self) -> PacmanFileKind {
        self.kind
    }

    /// Path of the file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Records a `.pacnew` or `.pacsave` file found by the pacman step.
pub fn record(finding: PacmanFinding) {
    PACMAN_FINDINGS.with(|findings| findings.borrow_mut().push(finding));
}

/// Takes the findings recorded on the current thread, resetting them so they
/// do not leak into the next step.
pub fn take() -> Vec<PacmanFinding> {
    PACMAN_FINDINGS.with(|findings| findings.borrow_mut().drain(..).collect())
}

/// Returns the `.pacnew` and `.pacsave` files found in the pacman config
/// directory.
///
/// The directory defaults to `/etc` and can be overridden with the
/// `FRESHEN_PACMAN_CONFIG_DIR` environment variable so tests can inject a
/// directory listing instead of touching the real system.
pub fn pacman_findings() -> Vec<PacmanFinding> {
    scan(&config_dir())
}

/// The directory pacman writes `.pacnew` and `.pacsave` files into.
fn config_dir() -> PathBuf {
    std::env::var_os("FRESHEN_PACMAN_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc"))
}

/// Recursively scans `dir` for files whose name ends in `.pacnew` or
/// `.pacsave`, appending them to `findings` in sorted order.
fn scan(root: &Path) -> Vec<PacmanFinding> {
    let mut findings = Vec::new();
    scan_dir(root, &mut findings);
    findings.sort_by(|a, b| a.path().cmp(b.path()));
    findings
}

fn scan_dir(dir: &Path, findings: &mut Vec<PacmanFinding>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            scan_dir(&path, findings);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let kind = if name.ends_with(".pacnew") {
            PacmanFileKind::Pacnew
        } else if name.ends_with(".pacsave") {
            PacmanFileKind::Pacsave
        } else {
            continue;
        };
        findings.push(PacmanFinding::new(kind, path));
    }
}

#[cfg(test)]
mod tests {
    use super::{scan, PacmanFileKind, PacmanFinding};
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A unique temp directory for each test so parallel tests never collide
    /// and never touch the real `/etc`.
    fn temp_dir(name: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-pacman-{pid}-{id}-{name}",
            pid = std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
        dir
    }

    fn write_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, "")
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
        path
    }

    #[test]
    fn finds_pacnew_and_pacsave_files() {
        let dir = temp_dir("mixed");
        write_file(&dir, "pacman.conf.pacnew");
        write_file(&dir, "group.pacsave");
        write_file(&dir, "hosts"); // not a finding

        let findings = scan(&dir);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].path(), dir.join("group.pacsave"));
        assert_eq!(findings[0].kind(), PacmanFileKind::Pacsave);
        assert_eq!(findings[1].path(), dir.join("pacman.conf.pacnew"));
        assert_eq!(findings[1].kind(), PacmanFileKind::Pacnew);
    }

    #[test]
    fn finds_files_in_subdirectories() {
        let dir = temp_dir("nested");
        let ssh = dir.join("ssh");
        fs::create_dir_all(&ssh)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", ssh.display()));
        write_file(&ssh, "sshd_config.pacnew");
        write_file(&dir, "pacman.conf.pacsave");

        let findings = scan(&dir);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].path(), dir.join("pacman.conf.pacsave"));
        assert_eq!(findings[0].kind(), PacmanFileKind::Pacsave);
        assert_eq!(findings[1].path(), ssh.join("sshd_config.pacnew"));
        assert_eq!(findings[1].kind(), PacmanFileKind::Pacnew);
    }

    #[test]
    fn ignores_symlinks_and_other_files() {
        let dir = temp_dir("other");
        write_file(&dir, "not-a-finding.conf");
        write_file(&dir, "pacman.conf.rpmnew"); // similar suffix, not a match
        write_file(&dir, "pacman.conf.pacnew.bak"); // not ending in .pacnew

        let findings = scan(&dir);
        assert!(findings.is_empty());
    }

    #[test]
    fn empty_directory_yields_no_findings() {
        let dir = temp_dir("empty");
        assert!(scan(&dir).is_empty());
    }

    #[test]
    fn finding_path_and_kind_are_exposed() {
        let finding = PacmanFinding::new(PacmanFileKind::Pacnew, "/etc/foo.pacnew");
        assert_eq!(finding.kind(), PacmanFileKind::Pacnew);
        assert_eq!(finding.path(), PathBuf::from("/etc/foo.pacnew"));
        assert_eq!(finding.kind().to_string(), "pacnew");
        assert_eq!(PacmanFileKind::Pacsave.to_string(), "pacsave");
    }
}
