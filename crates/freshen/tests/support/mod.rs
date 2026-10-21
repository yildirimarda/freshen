//! A stub-based harness for running the `freshen` binary without touching the
//! real package managers.
//!
//! Each stub is a tiny executable that appends its name and arguments to a log
//! file. The binary is started with `PATH` pointing at the stub directory only,
//! so a step that shells out to `brew`, `uv`, `mise`, `docker` or `colima` can
//! never reach the real tool on the host. After the run, [`StubTools::calls`]
//! returns the recorded invocations in the order they happened.
//!
//! By default every stub succeeds silently. [`StubTools::set_responses`] queues
//! canned stdout, stderr and exit status for the next invocations of a stub, in
//! order, so a ported step can be replayed against a recorded transcript. A
//! stub invoked more times than it has responses keeps succeeding silently.
//! [`StubTools::set_script`] remains available for stubs that need to react to
//! their arguments or change state between calls.
//!
//! The binary must be built with the `test-harness` feature, which registers
//! the probe steps described by `FRESHEN_TEST_STEPS`. The project test command
//! (`cargo test --all-features`) enables it.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::Duration;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// The canned result of one stub invocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StubOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: i32,
}

#[allow(dead_code)]
impl StubOutput {
    /// A successful invocation with no output.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A successful invocation that writes `stdout`.
    #[must_use]
    pub fn stdout(stdout: impl Into<Vec<u8>>) -> Self {
        Self::new().with_stdout(stdout)
    }

    /// A successful invocation that writes `stderr`.
    #[must_use]
    pub fn stderr(stderr: impl Into<Vec<u8>>) -> Self {
        Self::new().with_stderr(stderr)
    }

    /// Sets the standard output of the invocation.
    #[must_use]
    pub fn with_stdout(mut self, stdout: impl Into<Vec<u8>>) -> Self {
        self.stdout = stdout.into();
        self
    }

    /// Sets the standard error of the invocation.
    #[must_use]
    pub fn with_stderr(mut self, stderr: impl Into<Vec<u8>>) -> Self {
        self.stderr = stderr.into();
        self
    }

    /// Sets the exit status of the invocation.
    #[must_use]
    pub fn with_status(mut self, status: i32) -> Self {
        self.status = status;
        self
    }
}

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
            .env("FRESHEN_STUB_STATE", &self.dir)
            .env("FRESHEN_SNAPSHOT_DIR", self.dir.join("snapshots"))
            .env("FRESHEN_LOCK_DIR", self.dir.join("lock"))
            .env("FRESHEN_SKIP_PREFLIGHT", "1")
            .env("HOME", &self.dir)
            .env("FRESHEN_APP_ROOTS", self.dir.join("Applications"))
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

    /// Runs a command built from [`StubTools::command`] with scripted `stdin`.
    ///
    /// The caller configures the command's environment and arguments; the
    /// harness pipes all three standard streams and writes `stdin` before
    /// waiting for the process to exit. This drives interactive prompts (for
    /// example the cask quit prompt) end to end.
    #[allow(dead_code)]
    pub fn run_with_stdin(&self, mut command: Command, stdin: &str) -> Output {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));
        let mut input = child
            .stdin
            .take()
            .unwrap_or_else(|| panic!("freshen standard input was not piped"));
        input
            .write_all(stdin.as_bytes())
            .unwrap_or_else(|error| panic!("failed to write scripted stdin: {error}"));
        drop(input);
        child
            .wait_with_output()
            .unwrap_or_else(|error| panic!("failed to wait for freshen: {error}"))
    }

    /// Recorded invocations, one `"tool arg arg"` line per call, in order.
    pub fn calls(&self) -> Vec<String> {
        match fs::read_to_string(&self.log) {
            Ok(text) => text.lines().map(str::to_owned).collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("failed to read stub log {}: {error}", self.log.display()),
        }
    }

    /// Recorded invocations excluding the read-only pre-upgrade snapshot
    /// commands.
    ///
    /// Parity tests assert the commands that change the machine. Snapshot
    /// capture has its own tests, so it does not shift those transcripts.
    #[allow(dead_code)]
    pub fn step_calls(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|call| !is_snapshot_capture(call))
            .collect()
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

    /// Queues canned results for the next invocations of `tool`, in order.
    ///
    /// The first invocation gets the first output, the second the second, and
    /// so on. An invocation without a matching response succeeds with no
    /// output. Calling this again replaces any previous responses for `tool`.
    #[allow(dead_code)]
    pub fn set_responses(&self, tool: &str, responses: &[StubOutput]) {
        #[cfg(unix)]
        {
            let _ = fs::remove_file(self.dir.join(format!("{tool}.count")));

            let mut script = format!(
                "#!/bin/sh\n\
                 {{\n\
                   printf '%s' '{tool}'\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 }} >> \"${{FRESHEN_STUB_LOG:?FRESHEN_STUB_LOG is not set}}\"\n\
                 state=\"${{FRESHEN_STUB_STATE:?FRESHEN_STUB_STATE is not set}}\"\n\
                 count_file=\"$state/{tool}.count\"\n\
                 count=0\n\
                 if [ -f \"$count_file\" ]; then read -r count < \"$count_file\"; fi\n\
                 count=$((count + 1))\n\
                 printf '%s\\n' \"$count\" > \"$count_file\"\n\
                 case \"$count\" in\n"
            );
            for (index, response) in responses.iter().enumerate() {
                script.push_str(&format!("  {})\n", index + 1));
                if !response.stdout.is_empty() {
                    script.push_str(&format!(
                        "    printf '%s' {}\n",
                        shell_quote(&String::from_utf8_lossy(&response.stdout))
                    ));
                }
                if !response.stderr.is_empty() {
                    script.push_str(&format!(
                        "    printf '%s' {} >&2\n",
                        shell_quote(&String::from_utf8_lossy(&response.stderr))
                    ));
                }
                script.push_str(&format!("    exit {}\n    ;;\n", response.status));
            }
            script.push_str("esac\nexit 0\n");

            use std::os::unix::fs::PermissionsExt;

            let path = self.dir.join(tool);
            fs::write(&path, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", path.display()));
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let _ = fs::remove_file(self.dir.join(format!("{tool}.count")));
            let prefix = format!("{tool}.");
            if let Ok(entries) = fs::read_dir(&self.dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    let is_response = name.starts_with(&prefix)
                        && (name.ends_with(".stdout")
                            || name.ends_with(".stderr")
                            || name.ends_with(".status"));
                    if is_response {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }

            for (index, response) in responses.iter().enumerate() {
                let index = index + 1;
                for (extension, contents) in [
                    ("stdout", response.stdout.as_slice()),
                    ("stderr", response.stderr.as_slice()),
                ] {
                    let path = self.dir.join(format!("{tool}.{index}.{extension}"));
                    fs::write(&path, contents).unwrap_or_else(|error| {
                        panic!("failed to write stub response {}: {error}", path.display())
                    });
                }
                let path = self.dir.join(format!("{tool}.{index}.status"));
                let status = response.status.to_string();
                fs::write(&path, status).unwrap_or_else(|error| {
                    panic!("failed to write stub response {}: {error}", path.display())
                });
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = (tool, responses);
            panic!("stub responses are not supported on this platform");
        }
    }

    /// A command that runs `tool`'s stub directly, with the harness environment.
    ///
    /// Useful for testing the harness itself; normal tests run the binary with
    /// [`StubTools::command`] instead.
    #[allow(dead_code)]
    pub fn stub_command(&self, tool: &str) -> Command {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(self.dir.join(format!("{tool}.cmd")));
            command
        };

        #[cfg(not(windows))]
        let mut command = Command::new(self.dir.join(tool));

        command
            .env("FRESHEN_STUB_LOG", &self.log)
            .env("FRESHEN_STUB_STATE", &self.dir);
        command
    }

    #[allow(clippy::needless_return)]
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
             setlocal EnableDelayedExpansion\r\n\
             >>\"%FRESHEN_STUB_LOG%\" echo {tool} %*\r\n\
             set \"__count_file=%FRESHEN_STUB_STATE%\\{tool}.count\"\r\n\
             set /a __count=0\r\n\
             if exist \"!__count_file!\" set /p __count=<\"!__count_file!\"\r\n\
             set /a __count+=1\r\n\
             > \"!__count_file!\" echo !__count!\r\n\
             set \"__base=%FRESHEN_STUB_STATE%\\{tool}.!__count!\"\r\n\
             if exist \"!__base!.stdout\" type \"!__base!.stdout\"\r\n\
             if exist \"!__base!.stderr\" type \"!__base!.stderr\" 1>&2\r\n\
             set /a __status=0\r\n\
             if exist \"!__base!.status\" set /p __status=<\"!__base!.status\"\r\n\
             exit /b !__status!\r\n"
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

/// How many times to retry an execution that fails with `ETXTBSY`.
#[cfg(unix)]
const TEXT_FILE_BUSY_RETRIES: u32 = 50;

/// Runs `attempt`, retrying briefly while it fails with `ETXTBSY`.
///
/// Test threads run in parallel, so one thread can still hold a freshly
/// written stub open for writing when another thread forks. The forked child
/// keeps that descriptor until it execs, which makes executing the stub in
/// that window fail with `Text file busy`. The window is tiny, so a short
/// retry is enough.
#[cfg(unix)]
#[allow(dead_code)]
pub fn retry_text_file_busy<T>(mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut retries = 0;
    loop {
        match attempt() {
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) => {
                if retries >= TEXT_FILE_BUSY_RETRIES {
                    return Err(error);
                }
                retries += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
}

/// Runs `command` to completion, retrying while its stub is temporarily busy.
#[allow(dead_code)]
pub fn output_with_retry(mut command: Command) -> io::Result<Output> {
    #[cfg(unix)]
    {
        retry_text_file_busy(|| command.output())
    }

    #[cfg(not(unix))]
    {
        command.output()
    }
}

/// Quotes `text` as a single shell word, including embedded newlines and quotes.
#[cfg(unix)]
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn is_snapshot_capture(call: &str) -> bool {
    call == "brew list --versions"
        || call == "brew list --cask --versions"
        || call.starts_with("brew bundle dump ")
        || call == "uv tool list"
        || call == "mise ls"
        || call.starts_with("docker image ls ")
}

impl Drop for StubTools {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
