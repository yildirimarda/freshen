//! Sudo credential validation for steps that need root.
//!
//! freshen validates `sudo` once up front: an elevated process already has
//! root, and a standard user has passwordless `sudo` when `sudo -n true`
//! succeeds. Steps that need root ([`crate::Step::needs_root`]) are skipped
//! with [`crate::step::SkipReason::NeedsSudo`] when the credentials cannot be
//! obtained.

use crate::system::Privilege;

#[cfg(unix)]
use std::process::{Command, Stdio};

/// Whether passwordless `sudo` is available (`sudo -n true` succeeds).
pub trait SudoProbe {
    /// Returns `true` when `sudo -n true` succeeds without a password.
    fn sudo_passwordless(&self) -> bool;
}

/// The real host: runs `sudo -n true` on Unix. On non-Unix platforms
/// passwordless `sudo` is never available.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostSudoProbe;

#[cfg(unix)]
impl SudoProbe for HostSudoProbe {
    fn sudo_passwordless(&self) -> bool {
        Command::new("sudo")
            .arg("-n")
            .arg("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

#[cfg(not(unix))]
impl SudoProbe for HostSudoProbe {
    fn sudo_passwordless(&self) -> bool {
        false
    }
}

/// The result of validating `sudo` credentials once up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SudoState {
    available: bool,
}

impl SudoState {
    /// Validates `sudo` once from the current privilege level and `probe`.
    ///
    /// An elevated process already has root. A standard user has `sudo`
    /// credentials when `probe` reports passwordless `sudo`. An unknown
    /// privilege level means the credentials cannot be obtained.
    #[must_use]
    pub fn from_privilege_and_probe(privilege: Privilege, probe: &dyn SudoProbe) -> Self {
        let available = match privilege {
            Privilege::Elevated => true,
            Privilege::StandardUser => probe.sudo_passwordless(),
            Privilege::Unknown => false,
        };
        Self { available }
    }

    /// Returns `true` when `sudo` credentials are available.
    #[must_use]
    pub fn available(&self) -> bool {
        self.available
    }
}

#[cfg(test)]
mod tests {
    use super::{HostSudoProbe, SudoProbe, SudoState};
    use crate::system::Privilege;

    struct FakeSudoProbe {
        passwordless: bool,
    }

    impl FakeSudoProbe {
        fn new(passwordless: bool) -> Self {
            Self { passwordless }
        }
    }

    impl SudoProbe for FakeSudoProbe {
        fn sudo_passwordless(&self) -> bool {
            self.passwordless
        }
    }

    #[test]
    fn elevated_process_has_sudo_without_probing() {
        let state =
            SudoState::from_privilege_and_probe(Privilege::Elevated, &FakeSudoProbe::new(false));

        assert!(state.available());
    }

    #[test]
    fn standard_user_with_passwordless_sudo_has_sudo() {
        let state =
            SudoState::from_privilege_and_probe(Privilege::StandardUser, &FakeSudoProbe::new(true));

        assert!(state.available());
    }

    #[test]
    fn standard_user_without_sudo_has_no_sudo() {
        let state = SudoState::from_privilege_and_probe(
            Privilege::StandardUser,
            &FakeSudoProbe::new(false),
        );

        assert!(!state.available());
    }

    #[test]
    fn unknown_privilege_has_no_sudo() {
        let state =
            SudoState::from_privilege_and_probe(Privilege::Unknown, &FakeSudoProbe::new(true));

        assert!(!state.available());
    }

    #[test]
    fn host_sudo_probe_is_the_default_probe() {
        let _probe = HostSudoProbe;
    }

    // Tests that need Unix-only fixtures (an executable shell-script stub,
    // `chmod` via `PermissionsExt`) live here and own every import they use,
    // never the shared `use` block at the top of `mod tests`.
    #[cfg(unix)]
    mod unix {
        use std::env;
        use std::fs;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Mutex;

        use super::super::{HostSudoProbe, SudoProbe};
        use std::os::unix::fs::PermissionsExt;

        static COUNTER: AtomicU64 = AtomicU64::new(0);

        // `PATH` is process-global and tests run in parallel threads, so the
        // stub-sudo setup must be serialized to avoid one test overwriting
        // another test's `PATH` while its probe is running.
        static PATH_LOCK: Mutex<()> = Mutex::new(());

        /// Runs `f` with a stub `sudo` on `PATH` that exits with the given
        /// code, then restores `PATH` and removes the temp directory.
        fn with_stub_sudo<F>(exit_code: i32, f: F) -> bool
        where
            F: FnOnce(&HostSudoProbe) -> bool,
        {
            let _guard = PATH_LOCK
                .lock()
                .unwrap_or_else(|_| panic!("sudo test lock poisoned"));
            let original = env::var("PATH").unwrap_or_default();
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = env::temp_dir().join(format!(
                "freshen-sudo-test-{pid}-{id}",
                pid = std::process::id(),
                id = id
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir)
                .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
            let sudo = dir.join("sudo");
            fs::write(&sudo, format!("#!/bin/sh\nexit {exit_code}\n"))
                .unwrap_or_else(|error| panic!("failed to write stub sudo: {error}"));
            fs::set_permissions(&sudo, fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to chmod stub sudo: {error}"));
            env::set_var("PATH", format!("{}:{}", dir.display(), original));
            let result = f(&HostSudoProbe);
            env::set_var("PATH", &original);
            let _ = fs::remove_dir_all(&dir);
            result
        }

        #[test]
        fn host_sudo_reports_passwordless_sudo_when_stub_succeeds() {
            assert!(with_stub_sudo(0, |probe| probe.sudo_passwordless()));
        }

        #[test]
        fn host_sudo_reports_no_sudo_when_stub_fails() {
            assert!(!with_stub_sudo(1, |probe| probe.sudo_passwordless()));
        }
    }
}
