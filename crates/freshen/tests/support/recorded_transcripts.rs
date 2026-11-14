//! Recorded command transcripts stored per tool and version.
//!
//! Fixtures live under `crates/freshen/tests/fixtures/<tool>/<version>/` and
//! are named after the command that produced them, e.g.
//! `fixtures/zypper/1.14/zypper-ps-s.txt`. Parser tests load a fixture with
//! [`RecordedTranscripts::stdout`] and feed it to a stub so the parser is
//! exercised against real recorded output instead of an inline string.

#![allow(unused)]

use std::path::{Path, PathBuf};

/// A collection of recorded transcripts keyed by tool, version and command.
#[derive(Debug)]
pub struct RecordedTranscripts {
    root: PathBuf,
}

impl RecordedTranscripts {
    /// Opens the fixture tree rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The path to the fixture for `tool` at `version` produced by `command`.
    #[must_use]
    pub fn path(&self, tool: &str, version: &str, command: &str) -> PathBuf {
        self.root
            .join(tool)
            .join(version)
            .join(format!("{command}.txt"))
    }

    /// Returns the recorded stdout of `command` for `tool` at `version`.
    ///
    /// Panics with a clear message when the fixture is missing so a test that
    /// references an unrecorded transcript fails loudly instead of parsing an
    /// empty string.
    pub fn stdout(&self, tool: &str, version: &str, command: &str) -> String {
        let path = self.path(tool, version, command);
        std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "missing recorded transcript for {tool} {version} `{command}` at {}: {error}",
                path.display()
            )
        })
    }
}

impl Default for RecordedTranscripts {
    fn default() -> Self {
        Self::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"))
    }
}

impl AsRef<Path> for RecordedTranscripts {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}
