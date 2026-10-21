//! Finding the executable a shell would run for a program name.
//!
//! Steps name the tools they need (`brew`, `uv`, `mise`, ...) instead of
//! hard-coding paths. Detection and the process runner both have to agree on
//! what that name means: on Windows the file on disk is usually `uv.exe`, and
//! the test harness deliberately installs `.cmd` stubs so a test never reaches
//! a real package manager. Resolving through `PATHEXT` keeps both cases
//! working while Unix keeps its executable-bit check.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Returns `true` when `program` names an executable on `PATH`.
#[must_use]
pub fn command_exists(program: &str) -> bool {
    resolve(program).is_some()
}

/// Resolves `program` to the file a shell would execute.
///
/// A name containing a path separator is checked as given. A bare name is
/// looked up in every `PATH` entry, trying the platform's executable
/// extensions when the name does not already have one.
#[must_use]
pub fn resolve(program: impl AsRef<OsStr>) -> Option<PathBuf> {
    let program = program.as_ref();
    if program.is_empty() {
        return None;
    }

    let path = Path::new(program);
    if path.components().count() > 1 {
        return is_executable(path).then(|| path.to_path_buf());
    }

    let search_path = env::var_os("PATH")?;
    env::split_paths(&search_path).find_map(|directory| candidate(&directory, program))
}

fn candidate(directory: &Path, program: &OsStr) -> Option<PathBuf> {
    let direct = directory.join(program);
    let has_extension = Path::new(program).extension().is_some();

    if !has_extension {
        for extension in executable_extensions() {
            let mut name = program.to_os_string();
            name.push(extension);
            let path = directory.join(name);
            if is_executable(&path) {
                return Some(path);
            }
        }
    }

    is_executable(&direct).then_some(direct)
}

#[cfg(windows)]
fn executable_extensions() -> Vec<String> {
    let extensions = env::var_os("PATHEXT")
        .map(|value| {
            value
                .to_string_lossy()
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if extensions.is_empty() {
        [".COM", ".EXE", ".BAT", ".CMD"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    } else {
        extensions
    }
}

#[cfg(not(windows))]
fn executable_extensions() -> Vec<String> {
    Vec::new()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::candidate;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("freshen-executable-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
        dir
    }

    #[cfg(unix)]
    #[test]
    fn a_unix_candidate_must_be_executable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("unix");
        let path = dir.join("tool");
        fs::write(&path, "#!/bin/sh\n").unwrap_or_else(|error| panic!("{error}"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(candidate(&dir, OsStr::new("tool")), None);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(candidate(&dir, OsStr::new("tool")), Some(path));

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_candidate_is_found_through_its_extension() {
        let dir = temp_dir("windows");

        // The file is written in lower case with the extension `PATHEXT` names
        // first, so the test does not depend on the extensions the machine
        // happens to have.
        let extensions = super::executable_extensions();
        let extension = extensions
            .iter()
            .find(|extension| extension.eq_ignore_ascii_case(".cmd"))
            .or_else(|| extensions.first())
            .map(|extension| extension.to_lowercase())
            .unwrap_or_else(|| ".cmd".to_owned());
        let path = dir.join(format!("tool{extension}"));
        fs::write(&path, "@echo off\r\n").unwrap_or_else(|error| panic!("{error}"));

        let resolved = candidate(&dir, OsStr::new("tool"))
            .unwrap_or_else(|| panic!("the candidate was not found through {extension}"));

        // `PATHEXT` spells the extension in upper case, so the resolved path
        // can differ from the file on disk in case only.
        assert_eq!(resolved.parent(), Some(dir.as_path()));
        assert_eq!(
            resolved
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase()),
            Some(
                path.file_name()
                    .map(|name| name.to_string_lossy().to_lowercase())
                    .unwrap_or_default()
            )
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
