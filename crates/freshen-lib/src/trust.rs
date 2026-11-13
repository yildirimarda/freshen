//! Trust decisions for commands that come from the configuration file.
//!
//! Hooks (`[hooks.<step>]`) and custom steps (`[[custom_steps]]`) run arbitrary
//! commands, so freshen must not run a command it has not seen before without
//! an explicit opt-in. The first time a hook or custom step is seen, or the
//! first time its command changes, freshen requires either `--trust-config` or
//! an interactive confirmation that records the command's hash. Afterwards the
//! recorded hash makes the command trusted without another prompt.
//!
//! Trusted commands are recorded in `trusted.toml` next to the configuration
//! file, keyed by an identifier that names the hook or custom step.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::output::{flush, human_print};

/// Encodes `bytes` as lowercase hexadecimal.
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Name of the file that records the commands freshen is trusted to run.
///
/// It lives in the same directory as the configuration file.
pub const TRUSTED_FILE_NAME: &str = "trusted.toml";

/// The version of the hash format, prefixed to every stored hash so the
/// format can change without confusing entries written by an older freshen.
const HASH_PREFIX: &str = "sha256:v1";

/// A problem while reading or writing the trust store.
#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    /// The trust store could not be read.
    #[error("could not read the trust store {path}: {source}")]
    Read {
        /// Path of the trust store.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// The trust store could not be written.
    #[error("could not write the trust store {path}: {source}")]
    Write {
        /// Path of the trust store.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// The trust store is not valid freshen trust data.
    #[error("the trust store {path} is not valid freshen trust data: {message}")]
    Invalid {
        /// Path of the trust store.
        path: PathBuf,
        /// Line of the offending value.
        line: usize,
        /// Column of the offending value.
        column: usize,
        /// What is wrong with the value.
        message: String,
    },
}

/// The commands freshen is trusted to run, keyed by an identifier that names
/// the hook or custom step.
///
/// ```toml
/// [trusted]
/// "hook:brew:pre" = "sha256:v1:..."
/// "custom_step:my-linter" = "sha256:v1:..."
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    /// The trusted commands, keyed by their identifier.
    trusted: BTreeMap<String, String>,
    /// The file the trusted commands are recorded in.
    path: PathBuf,
}

/// The serialisable shape of the trust store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TrustStoreData {
    /// The trusted commands, keyed by their identifier.
    #[serde(default)]
    trusted: BTreeMap<String, String>,
}

impl TrustStore {
    /// Loads the trust store from `path`.
    ///
    /// A missing file is not an error: freshen starts with no trusted
    /// commands and records them as the user confirms them.
    ///
    /// # Errors
    ///
    /// Returns [`TrustError`] when the file cannot be read or is not valid
    /// trust data.
    pub fn load(path: &Path) -> Result<Self, TrustError> {
        let trusted = match fs::read_to_string(path) {
            Ok(text) => {
                let data: TrustStoreData =
                    toml::from_str(&text).map_err(|error| invalid(path, &text, &error))?;
                data.trusted
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => {
                return Err(TrustError::Read {
                    path: path.to_owned(),
                    source: error,
                })
            }
        };

        Ok(Self {
            trusted,
            path: path.to_owned(),
        })
    }

    /// Returns `true` when `command`, identified by `key`, is trusted.
    #[must_use]
    pub fn is_trusted(&self, key: &str, hash: &str) -> bool {
        self.trusted.get(key).is_some_and(|stored| stored == hash)
    }

    /// Records `command`, identified by `key`, as trusted in the trust store.
    ///
    /// Nothing happens when the command is already recorded.
    ///
    /// # Errors
    ///
    /// Returns [`TrustError::Write`] when the trust store cannot be written.
    pub fn record(&self, key: &str, hash: &str) -> Result<(), TrustError> {
        if self.is_trusted(key, hash) {
            return Ok(());
        }

        let mut trusted = self.trusted.clone();
        trusted.insert(key.to_owned(), hash.to_owned());
        let data = TrustStoreData { trusted };
        let text = toml::to_string(&data).map_err(|error| TrustError::Write {
            path: self.path.clone(),
            source: io::Error::other(error),
        })?;
        fs::write(&self.path, text).map_err(|source| TrustError::Write {
            path: self.path.clone(),
            source,
        })?;
        Ok(())
    }
}

/// Hashes `command` into a `sha256:v1:<hex>` identifier that changes whenever
/// the program or any of its arguments change.
#[must_use]
pub fn hash_command(command: &[String]) -> String {
    let mut hasher = Sha256::new();
    for part in command {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    format!("{HASH_PREFIX}:{}", to_hex(&digest))
}

/// Decides whether `command`, identified by `key`, may run.
///
/// Returns `Ok(true)` when the command may run. Returns `Ok(false)` when it
/// must be refused because it is not trusted and cannot be confirmed.
///
/// # Errors
///
/// Returns [`TrustError`] when the trust store cannot be read or written.
pub fn ensure_trusted(
    store: &TrustStore,
    key: &str,
    command: &[String],
    assume_yes: bool,
    trust_config: bool,
) -> Result<bool, TrustError> {
    let hash = hash_command(command);
    if store.is_trusted(key, &hash) {
        return Ok(true);
    }
    if trust_config {
        store.record(key, &hash)?;
        return Ok(true);
    }
    if assume_yes {
        return Ok(false);
    }
    if confirm_command(key, command) {
        store.record(key, &hash)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Prints the confirmation prompt and returns `true` when the user answers yes.
fn confirm_command(key: &str, command: &[String]) -> bool {
    let command = command.join(" ");
    human_print!("{key} runs `{command}`, which has not been trusted yet. Run it? [y/N]: ");
    flush();
    let mut answer = String::new();
    let _ = io::stdin().read_line(&mut answer);
    is_confirmation_yes(&answer)
}

/// Returns `true` when the confirmation answer means yes.
fn is_confirmation_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Turns a toml deserialization error into a [`TrustError::Invalid`] that
/// points at the offending line.
fn invalid(path: &Path, text: &str, error: &toml::de::Error) -> TrustError {
    let message = error
        .message()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let (line, column) = error
        .span()
        .map_or((1, 1), |span| line_and_column(text, span.start));

    TrustError::Invalid {
        path: path.to_owned(),
        line,
        column,
        message,
    }
}

fn line_and_column(text: &str, offset: usize) -> (usize, usize) {
    let mut start = offset.min(text.len());
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }

    let before = &text[..start];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(1, |line| line.chars().count() + 1);

    (line, column)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{ensure_trusted, hash_command, TrustStore, TRUSTED_FILE_NAME};
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "freshen-trust-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_trust_store_trusts_nothing() {
        let dir = temp_dir();
        let path = dir.join(TRUSTED_FILE_NAME);
        let store = TrustStore::load(&path).unwrap();

        assert!(!store.is_trusted("hook:brew:pre", "sha256:v1:abc"));
    }

    #[test]
    fn a_changed_command_is_not_trusted_even_when_the_key_is() {
        let dir = temp_dir();
        let path = dir.join(TRUSTED_FILE_NAME);
        let store = TrustStore::load(&path).unwrap();
        store
            .record("custom_step:my-linter", &hash_command(&["old".to_owned()]))
            .unwrap();

        assert!(!store.is_trusted("custom_step:my-linter", &hash_command(&["new".to_owned()])));
    }

    #[test]
    fn record_writes_and_reloads_the_hash() {
        let dir = temp_dir();
        let path = dir.join(TRUSTED_FILE_NAME);
        let store = TrustStore::load(&path).unwrap();
        let hash = hash_command(&["echo".to_owned(), "hi".to_owned()]);

        store.record("hook:brew:pre", &hash).unwrap();

        let reloaded = TrustStore::load(&path).unwrap();
        assert!(reloaded.is_trusted("hook:brew:pre", &hash));
    }

    #[test]
    fn recording_an_already_trusted_command_is_idempotent() {
        let dir = temp_dir();
        let path = dir.join(TRUSTED_FILE_NAME);
        let store = TrustStore::load(&path).unwrap();
        let hash = hash_command(&["echo".to_owned()]);

        store.record("custom_step:a", &hash).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        store.record("custom_step:a", &hash).unwrap();
        let second = fs::read_to_string(&path).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn ensure_trusted_records_with_trust_config_without_prompting() {
        let dir = temp_dir();
        let path = dir.join(TRUSTED_FILE_NAME);
        let store = TrustStore::load(&path).unwrap();
        let command = vec!["custom-a".to_owned()];

        let trusted =
            ensure_trusted(&store, "custom_step:custom-a", &command, false, true).unwrap();

        assert!(trusted);

        let reloaded = TrustStore::load(&path).unwrap();
        assert!(reloaded.is_trusted("custom_step:custom-a", &hash_command(&command)));
    }

    #[test]
    fn ensure_trusted_refuses_without_trust_config_or_confirmation() {
        let dir = temp_dir();
        let path = dir.join(TRUSTED_FILE_NAME);
        let store = TrustStore::load(&path).unwrap();
        let command = vec!["custom-a".to_owned()];

        let trusted =
            ensure_trusted(&store, "custom_step:custom-a", &command, false, false).unwrap();

        assert!(!trusted);
        assert!(!store.is_trusted("custom_step:custom-a", &hash_command(&command)));
    }

    #[test]
    fn ensure_trusted_refuses_when_assume_yes_is_set_without_trust_config() {
        let dir = temp_dir();
        let path = dir.join(TRUSTED_FILE_NAME);
        let store = TrustStore::load(&path).unwrap();
        let command = vec!["custom-a".to_owned()];

        let trusted =
            ensure_trusted(&store, "custom_step:custom-a", &command, true, false).unwrap();

        assert!(!trusted);
    }

    #[test]
    fn an_invalid_trust_store_is_refused() {
        let dir = temp_dir();
        let path = dir.join(TRUSTED_FILE_NAME);
        fs::write(&path, "not valid toml [[[").unwrap();

        let error = TrustStore::load(&path).unwrap_err();

        assert!(
            matches!(error, super::TrustError::Invalid { .. }),
            "expected an invalid trust store error, got {error}"
        );
    }
}
