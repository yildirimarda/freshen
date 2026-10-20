//! A stub-based harness for running the `freshen` binary without touching the
//! real package managers.
//!
//! Each stub is a tiny executable that appends its name and arguments to a log
//! file. The binary is started with `PATH` pointing at the stub directory only,
//! so a step that shells out to `brew`, `uv`, `mise`, `docker` or `colima` can
//! never reach the real tool on the host. After the run, [`StubTools::calls`]
//! returns the recorded invocations in the order they happened.
//!
//! The binary must be built with the `test-harness` feature, which registers
//! the probe steps described by `FRESHEN_TEST_STEPS`. The project test command
//! (`cargo test --all-features`) enables it.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// A temporary directory of stub executables plus the log they write to.
pub struct StubTools {
    dir: PathBuf,
    log: PathBuf,
}

impl StubTools {
    /// Creates a stub for every named tool in a fresh temporary directory.
    pub fn new(tools: &[&str]) -> Self {
        let unique = format!(
            "freshen-stubs-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });

        let harness = Self {
            log: dir.join("calls.log"),
            dir,
        };
        for tool in tools {
            harness.write_stub(tool);
        }
        harness
    }

    /// A command that runs the built binary with only the stubs on `PATH`.
    ///
    /// The caller adds arguments and any `FRESHEN_TEST_STEPS` value.
    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_freshen"));
        command
            .env("PATH", self.path())
            .env("FRESHEN_STUB_LOG", &self.log)
            .env("HOME", &self.dir)
            .env("USERPROFILE", &self.dir);
        command
    }

    /// Runs the binary with `FRESHEN_TEST_STEPS` set to `steps`.
    #[allow(dead_code)]
    pub fn run(&self, steps: &str, args: &[&str]) -> Output {
        self.command()
            .env("FRESHEN_TEST_STEPS", steps)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
    }

    /// Recorded invocations, one `"tool arg arg"` line per call, in order.
    pub fn calls(&self) -> Vec<String> {
        match fs::read_to_string(&self.log) {
            Ok(text) => text.lines().map(str::to_owned).collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("failed to read stub log {}: {error}", self.log.display()),
        }
    }

    /// The temporary home directory the binary runs with.
    #[allow(dead_code)]
    pub fn home(&self) -> &std::path::Path {
        &self.dir
    }

    /// Replaces the stub for `tool` with a shell script body.
    ///
    /// The standard invocation logging is kept, and `body` runs afterwards, so
    /// a test can return recorded command output without reaching the real
    /// tool. Only available on Unix, where the Homebrew steps run.
    #[cfg(unix)]
    #[allow(dead_code)]
    pub fn set_script(&self, tool: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        let script = format!(
            "#!/bin/sh\n\
             {{\n\
               printf '%s' '{tool}'\n\
               for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
               printf '\\n'\n\
             }} >> \"${{FRESHEN_STUB_LOG:?FRESHEN_STUB_LOG is not set}}\"\n\
             {body}\n"
        );
        let path = self.dir.join(tool);
        fs::write(&path, script)
            .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", path.display()));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
    }

    fn path(&self) -> std::ffi::OsString {
        #[cfg(windows)]
        {
            // `cmd.exe` and the Windows command resolution machinery live in
            // System32; they are not package managers, so including them keeps
            // the stubs runnable without exposing a real `brew` or `uv`.
            let system_root =
                std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let system32 = PathBuf::from(system_root).join("System32");
            return std::env::join_paths([self.dir.clone(), system32])
                .unwrap_or_else(|error| panic!("failed to build PATH: {error}"));
        }

        #[cfg(not(windows))]
        {
            self.dir.clone().into_os_string()
        }
    }

    #[cfg(unix)]
    fn write_stub(&self, tool: &str) {
        use std::os::unix::fs::PermissionsExt;

        let script = format!(
            "#!/bin/sh\n\
             {{\n\
               printf '%s' '{tool}'\n\
               for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
               printf '\\n'\n\
             }} >> \"${{FRESHEN_STUB_LOG:?FRESHEN_STUB_LOG is not set}}\"\n"
        );
        let path = self.dir.join(tool);
        fs::write(&path, script)
            .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", path.display()));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
    }

    #[cfg(windows)]
    fn write_stub(&self, tool: &str) {
        let script = format!(
            "@echo off\r\n\
             >>\"%FRESHEN_STUB_LOG%\" echo {tool} %*\r\n"
        );
        let path = self.dir.join(format!("{tool}.cmd"));
        fs::write(&path, script)
            .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", path.display()));
    }

    #[cfg(not(any(unix, windows)))]
    fn write_stub(&self, _tool: &str) {
        panic!("stub executables are not supported on this platform");
    }
}

impl Drop for StubTools {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
