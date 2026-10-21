//! Running external commands: live output, timeouts, environment and retries.
//!
//! Every step shells out to a package manager. This module gives those calls
//! one implementation so each step does not have to reinvent process handling:
//!
//! * stdout and stderr are read on separate threads, so a chatty command can
//!   never deadlock by filling one pipe while the other is blocked;
//! * each line is handed to a callback as soon as it arrives, which lets the
//!   CLI stream progress while the command is still running;
//! * the same bytes are captured and returned for the run log;
//! * a per-command timeout kills a hung child instead of hanging the whole run;
//! * the runner's per-step and global deadlines stop the same child even when
//!   the command itself has no timeout;
//! * the environment is inherited by default and can be extended, reduced or
//!   cleared;
//! * a command can be retried with exponential backoff.
//!
//! The runner never inherits stdin: a package manager must not block waiting
//! for input from the terminal that freshen itself was started from.

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{deadline, signals};

/// How often the parent checks whether the child exited or timed out.
///
/// This bounds how quickly a timeout is noticed and how quickly buffered
/// output is delivered when the child produces no new lines.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// How long to wait for reader threads after the child has been reaped.
///
/// A child may exit while a grandchild it started still holds the output
/// pipes open (a daemon or a killed shell's remaining child). Waiting forever
/// would turn a per-command timeout into a hang, so readers that have not
/// finished by this deadline are detached.
const READER_GRACE: Duration = Duration::from_millis(100);

/// Which of a command's output pipes a chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

impl OutputStream {
    /// Lowercase name used in logs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// Retry delays for a command that fails or times out.
///
/// The delay before attempt `n` (for `n >= 2`) is the initial delay doubled
/// for every earlier retry, capped at [`Backoff::max`]. The default is one
/// second growing to thirty seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    initial: Duration,
    max: Duration,
}

impl Backoff {
    /// No delay between attempts.
    pub const NONE: Self = Self {
        initial: Duration::ZERO,
        max: Duration::ZERO,
    };

    /// Creates a backoff that starts at `initial` and never exceeds `max`.
    ///
    /// A `max` smaller than `initial` is raised to `initial`, so the first
    /// retry is never shortened.
    #[must_use]
    pub fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max: max.max(initial),
        }
    }

    /// The delay before the first retry.
    #[must_use]
    pub fn initial(&self) -> Duration {
        self.initial
    }

    /// The upper bound on any single delay.
    #[must_use]
    pub fn max(&self) -> Duration {
        self.max
    }

    /// Delay to wait before `attempt`, where attempts are numbered from 1.
    ///
    /// The first attempt never waits. Attempts past the point where doubling
    /// would exceed `max` all wait `max`.
    #[must_use]
    pub fn delay_before(&self, attempt: u32) -> Duration {
        if attempt <= 1 {
            return Duration::ZERO;
        }
        let doublings = attempt.saturating_sub(2).min(31);
        let factor = 1_u32 << doublings;
        self.initial.saturating_mul(factor).min(self.max)
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(Duration::from_secs(1), Duration::from_secs(30))
    }
}

/// Description of one external command.
///
/// The builder methods consume and return the spec, so a command reads as a
/// chain:
///
/// ```
/// use freshen_lib::process::CommandSpec;
/// use std::time::Duration;
///
/// let spec = CommandSpec::new("brew")
///     .arg("upgrade")
///     .env("HOMEBREW_NO_AUTO_UPDATE", "1")
///     .timeout(Duration::from_secs(600))
///     .retries(2);
/// assert_eq!(spec.program(), "brew");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    program: OsString,
    args: Vec<OsString>,
    env_clear: bool,
    env: Vec<(OsString, OsString)>,
    env_remove: Vec<OsString>,
    timeout: Option<Duration>,
    retries: u32,
    backoff: Backoff,
}

impl CommandSpec {
    /// Creates a spec for `program`, inheriting the current environment.
    #[must_use]
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env_clear: false,
            env: Vec::new(),
            env_remove: Vec::new(),
            timeout: None,
            retries: 0,
            backoff: Backoff::default(),
        }
    }

    /// Adds one argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds several arguments in order.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets an environment variable for the child.
    #[must_use]
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Sets several environment variables for the child.
    #[must_use]
    pub fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env.extend(
            vars.into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }

    /// Removes an inherited environment variable from the child.
    #[must_use]
    pub fn env_remove(mut self, key: impl Into<OsString>) -> Self {
        self.env_remove.push(key.into());
        self
    }

    /// Starts the child with an empty environment instead of inheriting one.
    ///
    /// Variables added with [`CommandSpec::env`] are still set.
    #[must_use]
    pub fn clear_env(mut self) -> Self {
        self.env_clear = true;
        self
    }

    /// Sets a timeout that applies to each attempt.
    ///
    /// A command that is still running when the timeout elapses is killed and
    /// reported as [`ProcessError::TimedOut`], which is distinct from a
    /// command that ran and exited unsuccessfully.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets how many times a failed or timed-out command is retried.
    ///
    /// `retries(2)` allows up to three attempts in total. Spawn and I/O errors
    /// are never retried because they are not expected to go away by waiting.
    #[must_use]
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Sets the delay policy used between retries.
    #[must_use]
    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// Program name or path, as given to [`CommandSpec::new`].
    #[must_use]
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// Arguments in the order they were added.
    #[must_use]
    pub fn arguments(&self) -> &[OsString] {
        &self.args
    }

    /// Timeout per attempt, if one is set.
    #[must_use]
    pub fn timeout_limit(&self) -> Option<Duration> {
        self.timeout
    }

    /// Number of retries after the first attempt.
    #[must_use]
    pub fn retry_count(&self) -> u32 {
        self.retries
    }

    /// Runs the command, discarding live output.
    ///
    /// # Errors
    ///
    /// Returns a [`ProcessError`] when the command cannot be started, its
    /// output cannot be captured, it exits unsuccessfully, or it times out
    /// after all retries are exhausted.
    pub fn run(&self) -> Result<ProcessOutput, ProcessError> {
        self.run_streaming(|_, _| {})
    }

    /// Runs the command, calling `on_line` for every chunk of output.
    ///
    /// Chunks are normally complete lines, without the trailing newline; the
    /// final line of a stream may be a partial line. The callback runs on the
    /// calling thread, so it may borrow local state.
    ///
    /// Output from all attempts is captured. The returned [`ProcessOutput`]
    /// describes the attempt that finally succeeded.
    ///
    /// # Errors
    ///
    /// Returns a [`ProcessError`] when the command cannot be started, its
    /// output cannot be captured, it exits unsuccessfully, or it times out
    /// after all retries are exhausted.
    pub fn run_streaming<F>(&self, mut on_line: F) -> Result<ProcessOutput, ProcessError>
    where
        F: FnMut(OutputStream, &str),
    {
        let max_attempts = self.retries.saturating_add(1);
        let program = self.program.to_string_lossy().into_owned();
        let started = Instant::now();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut attempt: u32 = 1;

        loop {
            if signals::requested() {
                return Err(ProcessError::Interrupted { program });
            }

            if deadline::expired() {
                let timeout = deadline::current().map_or_else(
                    || self.timeout.unwrap_or_default(),
                    deadline::Deadline::limit,
                );
                return Err(ProcessError::TimedOut {
                    program,
                    timeout,
                    output: Box::new(ProcessOutput {
                        stdout: String::from_utf8_lossy(&stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&stderr).into_owned(),
                        status: None,
                        attempts: attempt.saturating_sub(1),
                        duration: started.elapsed(),
                    }),
                });
            }

            if attempt > 1 {
                let delay = self.backoff.delay_before(attempt);
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
                if signals::requested() {
                    return Err(ProcessError::Interrupted { program });
                }
            }

            let result = self.run_attempt(&mut on_line)?;
            stdout.extend_from_slice(&result.captured.stdout);
            stderr.extend_from_slice(&result.captured.stderr);

            let outcome = result.outcome;
            if matches!(outcome, AttemptOutcome::Interrupted) {
                return Err(ProcessError::Interrupted { program });
            }
            if let AttemptOutcome::Exited(status) = outcome {
                if status.success() {
                    return Ok(ProcessOutput {
                        stdout: String::from_utf8_lossy(&stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&stderr).into_owned(),
                        status: Some(status),
                        attempts: attempt,
                        duration: started.elapsed(),
                    });
                }
            }

            if attempt >= max_attempts {
                let output = Box::new(ProcessOutput {
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    status: match outcome {
                        AttemptOutcome::Exited(status) => Some(status),
                        AttemptOutcome::TimedOut | AttemptOutcome::Interrupted => None,
                    },
                    attempts: attempt,
                    duration: started.elapsed(),
                });
                return Err(match outcome {
                    AttemptOutcome::Exited(status) => ProcessError::Failed {
                        program,
                        status,
                        output,
                    },
                    AttemptOutcome::TimedOut => ProcessError::TimedOut {
                        program,
                        timeout: deadline::current()
                            .filter(|deadline| deadline.expired())
                            .map_or_else(
                                || self.timeout.unwrap_or_default(),
                                deadline::Deadline::limit,
                            ),
                        output,
                    },
                    AttemptOutcome::Interrupted => ProcessError::Interrupted { program },
                });
            }

            attempt += 1;
        }
    }

    /// Runs one attempt, streaming and capturing its output.
    fn run_attempt(
        &self,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> Result<AttemptResult, ProcessError> {
        let program = self.program.to_string_lossy().into_owned();
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if self.env_clear {
            command.env_clear();
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        for key in &self.env_remove {
            command.env_remove(key);
        }

        let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
            program: program.clone(),
            source,
        })?;

        let (sender, receiver) = mpsc::channel();
        let mut readers = Vec::with_capacity(2);
        if let Some(pipe) = child.stdout.take() {
            readers.push(spawn_reader(pipe, OutputStream::Stdout, sender.clone()));
        }
        if let Some(pipe) = child.stderr.take() {
            readers.push(spawn_reader(pipe, OutputStream::Stderr, sender.clone()));
        }
        drop(sender);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut pending = readers.len();
        let mut reader_error = None;
        let attempt_started = Instant::now();
        let outcome;

        loop {
            drain(
                &receiver,
                &mut pending,
                &mut reader_error,
                &mut stdout,
                &mut stderr,
                on_line,
            );

            match child.try_wait() {
                Ok(Some(status)) => {
                    outcome = AttemptOutcome::Exited(status);
                    break;
                }
                Ok(None) => {}
                Err(source) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = join_readers(readers, &program);
                    return Err(ProcessError::Wait { program, source });
                }
            }

            let timed_out = self
                .timeout
                .is_some_and(|limit| attempt_started.elapsed() >= limit)
                || deadline::expired();
            if timed_out {
                let _ = child.kill();
                let _ = child.wait();
                outcome = AttemptOutcome::TimedOut;
                break;
            }

            if signals::requested() {
                let _ = child.kill();
                let _ = child.wait();
                outcome = AttemptOutcome::Interrupted;
                break;
            }

            thread::sleep(POLL_INTERVAL);
        }

        let deadline = Instant::now() + READER_GRACE;
        while pending > 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match receiver.recv_timeout(remaining) {
                Ok(event) => handle_event(
                    event,
                    &mut pending,
                    &mut reader_error,
                    &mut stdout,
                    &mut stderr,
                    on_line,
                ),
                Err(_) => break,
            }
        }
        drain(
            &receiver,
            &mut pending,
            &mut reader_error,
            &mut stdout,
            &mut stderr,
            on_line,
        );

        if pending == 0 {
            join_readers(readers, &program)?;
        } else {
            drop(readers);
        }
        if let Some(source) = reader_error {
            return Err(ProcessError::Capture { program, source });
        }

        Ok(AttemptResult {
            outcome,
            captured: Captured { stdout, stderr },
        })
    }
}

/// A chunk of output sent from a reader thread to the parent.
struct Chunk {
    stream: OutputStream,
    bytes: Vec<u8>,
}

/// A message from a reader thread.
enum ReaderEvent {
    /// A line of output, normally including its newline.
    Chunk(Chunk),
    /// The reader reached end of file or failed.
    Done(Option<std::io::Error>),
}

/// Output captured from one attempt.
struct Captured {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// How one attempt ended.
#[derive(Debug, Clone, Copy)]
enum AttemptOutcome {
    Exited(ExitStatus),
    TimedOut,
    Interrupted,
}

/// The result of one attempt: how it ended and what it printed.
struct AttemptResult {
    outcome: AttemptOutcome,
    captured: Captured,
}

/// Reads `pipe` line by line and forwards each chunk to the parent.
fn spawn_reader<R>(pipe: R, stream: OutputStream, sender: Sender<ReaderEvent>) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) => {
                    let _ = sender.send(ReaderEvent::Done(None));
                    return;
                }
                Ok(_) => {
                    let chunk = Chunk {
                        stream,
                        bytes: buffer.clone(),
                    };
                    if sender.send(ReaderEvent::Chunk(chunk)).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = sender.send(ReaderEvent::Done(Some(error)));
                    return;
                }
            }
        }
    })
}

/// Delivers everything currently queued by the reader threads.
fn drain(
    receiver: &Receiver<ReaderEvent>,
    pending: &mut usize,
    reader_error: &mut Option<std::io::Error>,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    on_line: &mut dyn FnMut(OutputStream, &str),
) {
    while let Ok(event) = receiver.try_recv() {
        handle_event(event, pending, reader_error, stdout, stderr, on_line);
    }
}

/// Captures a chunk or records that a reader finished.
fn handle_event(
    event: ReaderEvent,
    pending: &mut usize,
    reader_error: &mut Option<std::io::Error>,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    on_line: &mut dyn FnMut(OutputStream, &str),
) {
    match event {
        ReaderEvent::Chunk(chunk) => {
            let text = String::from_utf8_lossy(&chunk.bytes);
            let line = text.trim_end_matches(['\n', '\r']);
            on_line(chunk.stream, line);
            match chunk.stream {
                OutputStream::Stdout => stdout.extend_from_slice(&chunk.bytes),
                OutputStream::Stderr => stderr.extend_from_slice(&chunk.bytes),
            }
        }
        ReaderEvent::Done(error) => {
            *pending = pending.saturating_sub(1);
            if let Some(error) = error {
                reader_error.get_or_insert(error);
            }
        }
    }
}

/// Joins the reader threads and reports panics.
fn join_readers(readers: Vec<JoinHandle<()>>, program: &str) -> Result<(), ProcessError> {
    for reader in readers {
        if reader.join().is_err() {
            return Err(ProcessError::Capture {
                program: program.to_owned(),
                source: std::io::Error::other("output reader thread panicked"),
            });
        }
    }
    Ok(())
}

/// Captured output of a command run by [`CommandSpec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    stdout: String,
    stderr: String,
    status: Option<ExitStatus>,
    attempts: u32,
    duration: Duration,
}

impl ProcessOutput {
    /// Everything the command wrote to stdout, across all attempts.
    #[must_use]
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// Everything the command wrote to stderr, across all attempts.
    #[must_use]
    pub fn stderr(&self) -> &str {
        &self.stderr
    }

    /// Exit status of the final attempt, or `None` when it timed out.
    #[must_use]
    pub fn status(&self) -> Option<ExitStatus> {
        self.status
    }

    /// Returns `true` when the final attempt exited successfully.
    #[must_use]
    pub fn success(&self) -> bool {
        self.status.is_some_and(|status| status.success())
    }

    /// Number of attempts that were made, including the successful one.
    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Wall-clock time spent on the command, including retry delays.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.duration
    }
}

/// A command that could not be run to a successful completion.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// The child process could not be started.
    #[error("could not start `{program}`: {source}")]
    Spawn {
        /// Program that could not be started.
        program: String,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// The child exited unsuccessfully after every attempt.
    #[error("`{program}` failed with {status} after {attempts} attempt(s)", attempts = .output.attempts())]
    Failed {
        /// Program that failed.
        program: String,
        /// Exit status of the final attempt.
        status: ExitStatus,
        /// Captured output of all attempts.
        output: Box<ProcessOutput>,
    },

    /// The child was still running when the timeout elapsed, on every attempt.
    #[error("`{program}` timed out after {timeout:?} on attempt {attempts}", attempts = .output.attempts())]
    TimedOut {
        /// Program that timed out.
        program: String,
        /// Timeout that was configured per attempt.
        timeout: Duration,
        /// Captured output of all attempts.
        output: Box<ProcessOutput>,
    },

    /// The run was interrupted and the child was stopped.
    #[error("`{program}` was interrupted")]
    Interrupted {
        /// Program that was stopped.
        program: String,
    },

    /// The parent could not wait for the child.
    #[error("could not wait for `{program}`: {source}")]
    Wait {
        /// Program whose status could not be read.
        program: String,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// Output could not be read from the child's pipes.
    #[error("could not capture output of `{program}`: {source}")]
    Capture {
        /// Program whose output could not be read.
        program: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

impl ProcessError {
    /// Captured output attached to the error, when there is any.
    #[must_use]
    pub fn output(&self) -> Option<&ProcessOutput> {
        match self {
            Self::Failed { output, .. } | Self::TimedOut { output, .. } => Some(output),
            Self::Spawn { .. }
            | Self::Interrupted { .. }
            | Self::Wait { .. }
            | Self::Capture { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Backoff, CommandSpec, OutputStream, ProcessError};
    use std::time::{Duration, Instant};

    fn backoff() -> Backoff {
        Backoff::new(Duration::from_millis(10), Duration::from_millis(100))
    }

    fn run_or_panic(spec: &CommandSpec) -> super::ProcessOutput {
        match spec.run() {
            Ok(output) => output,
            Err(error) => panic!("command failed: {error}"),
        }
    }

    #[test]
    fn backoff_starts_after_the_first_attempt_and_doubles_up_to_the_cap() {
        let backoff = backoff();

        assert_eq!(backoff.delay_before(0), Duration::ZERO);
        assert_eq!(backoff.delay_before(1), Duration::ZERO);
        assert_eq!(backoff.delay_before(2), Duration::from_millis(10));
        assert_eq!(backoff.delay_before(3), Duration::from_millis(20));
        assert_eq!(backoff.delay_before(4), Duration::from_millis(40));
        assert_eq!(backoff.delay_before(5), Duration::from_millis(80));
        assert_eq!(backoff.delay_before(6), Duration::from_millis(100));
        assert_eq!(backoff.delay_before(100), Duration::from_millis(100));
    }

    #[test]
    fn backoff_none_never_waits() {
        for attempt in 0..10 {
            assert_eq!(Backoff::NONE.delay_before(attempt), Duration::ZERO);
        }
    }

    #[test]
    fn backoff_raises_a_max_below_the_initial_delay() {
        let backoff = Backoff::new(Duration::from_millis(50), Duration::from_millis(10));

        assert_eq!(backoff.max(), Duration::from_millis(50));
        assert_eq!(backoff.delay_before(2), Duration::from_millis(50));
    }

    #[test]
    fn builder_records_the_command() {
        let spec = CommandSpec::new("brew")
            .arg("upgrade")
            .args(["--formula", "--quiet"])
            .timeout(Duration::from_secs(60))
            .retries(3)
            .env("A", "1")
            .envs([("B", "2"), ("C", "3")]);

        assert_eq!(spec.program(), "brew");
        assert_eq!(
            spec.arguments(),
            ["upgrade", "--formula", "--quiet"]
                .map(std::ffi::OsString::from)
                .to_vec()
        );
        assert_eq!(spec.timeout_limit(), Some(Duration::from_secs(60)));
        assert_eq!(spec.retry_count(), 3);
    }

    #[cfg(unix)]
    fn shell(script: &str) -> CommandSpec {
        CommandSpec::new("/bin/sh").arg("-c").arg(script)
    }

    #[cfg(unix)]
    #[test]
    fn captures_stdout_and_stderr() {
        let output = run_or_panic(&shell("printf 'hello\\n'; printf 'oops\\n' >&2"));

        assert!(output.success());
        assert_eq!(output.stdout(), "hello\n");
        assert_eq!(output.stderr(), "oops\n");
        assert_eq!(output.attempts(), 1);
        assert_eq!(output.status().and_then(|status| status.code()), Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn streams_lines_while_the_command_is_still_running() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            shell("printf 'first\\n'; sleep 1; printf 'second\\n'").run_streaming(
                move |stream, line| {
                    let _ = sender.send((stream, line.to_owned()));
                },
            )
        });

        let first = match receiver.recv_timeout(Duration::from_millis(800)) {
            Ok(chunk) => chunk,
            Err(error) => panic!("no live output before the process exited: {error}"),
        };
        assert_eq!(first, (OutputStream::Stdout, "first".to_owned()));

        match handle.join() {
            Ok(Ok(output)) => {
                assert!(output.success());
                assert_eq!(output.stdout(), "first\nsecond\n");
            }
            Ok(Err(error)) => panic!("command failed: {error}"),
            Err(_) => panic!("runner thread panicked"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_timeout_kills_the_child_and_is_reported_separately_from_failure() {
        let spec = shell("sleep 5").timeout(Duration::from_millis(50));

        match spec.run() {
            Err(ProcessError::TimedOut {
                timeout, output, ..
            }) => {
                assert_eq!(timeout, Duration::from_millis(50));
                assert_eq!(output.attempts(), 1);
                assert!(!output.success());
                assert_eq!(output.status(), None);
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_step_deadline_stops_a_command_even_without_a_command_timeout() {
        let _guard = crate::deadline::enter(Some(crate::deadline::Deadline::after(
            Instant::now(),
            Duration::from_millis(50),
        )));
        let spec = shell("sleep 5");

        match spec.run() {
            Err(ProcessError::TimedOut {
                timeout, output, ..
            }) => {
                assert_eq!(timeout, Duration::from_millis(50));
                assert!(!output.success());
                assert_eq!(output.status(), None);
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_nonzero_exit_is_a_failure_with_captured_output() {
        let spec = shell("printf 'boom\\n' >&2; exit 7");

        match spec.run() {
            Err(ProcessError::Failed { status, output, .. }) => {
                assert_eq!(status.code(), Some(7));
                assert_eq!(output.attempts(), 1);
                assert_eq!(output.stderr(), "boom\n");
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn retries_a_failing_command_until_it_succeeds() {
        let marker =
            std::env::temp_dir().join(format!("freshen-process-retry-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!(
            "if [ -f '{marker}' ]; then printf 'ok\\n'; else : > '{marker}'; printf 'retry\\n' >&2; exit 1; fi",
            marker = marker.display()
        );
        let spec = shell(&script).retries(2).backoff(Backoff::new(
            Duration::from_millis(1),
            Duration::from_millis(5),
        ));

        let result = spec.run();
        let _ = std::fs::remove_file(&marker);

        let output = match result {
            Ok(output) => output,
            Err(error) => panic!("command failed: {error}"),
        };
        assert!(output.success());
        assert_eq!(output.attempts(), 2);
        assert_eq!(output.stdout(), "ok\n");
        assert_eq!(output.stderr(), "retry\n");
    }

    #[cfg(unix)]
    #[test]
    fn retries_exhausted_returns_the_last_failure_with_all_output() {
        let spec = shell("printf 'boom\\n' >&2; exit 7")
            .retries(2)
            .backoff(Backoff::new(
                Duration::from_millis(1),
                Duration::from_millis(2),
            ));

        match spec.run() {
            Err(ProcessError::Failed { status, output, .. }) => {
                assert_eq!(status.code(), Some(7));
                assert_eq!(output.attempts(), 3);
                assert_eq!(output.stderr().matches("boom").count(), 3);
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_timed_out_attempt_is_retried() {
        let spec = shell("sleep 5")
            .timeout(Duration::from_millis(30))
            .retries(1)
            .backoff(Backoff::new(
                Duration::from_millis(1),
                Duration::from_millis(1),
            ));

        match spec.run() {
            Err(ProcessError::TimedOut { output, .. }) => {
                assert_eq!(output.attempts(), 2);
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_program_is_a_spawn_error_and_is_not_retried() {
        let spec = CommandSpec::new("freshen-definitely-not-a-real-binary")
            .retries(3)
            .backoff(Backoff::NONE);

        match spec.run() {
            Err(ProcessError::Spawn { program, .. }) => {
                assert_eq!(program, "freshen-definitely-not-a-real-binary");
            }
            other => panic!("expected a spawn error, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn environment_is_inherited_extended_and_can_be_cleared() {
        let inherited = run_or_panic(&shell("printf '%s' \"$CARGO_PKG_NAME\""));
        assert_eq!(inherited.stdout(), env!("CARGO_PKG_NAME"));

        let overridden = run_or_panic(&shell("printf '%s' \"$FOO\"").env("FOO", "bar"));
        assert_eq!(overridden.stdout(), "bar");

        let cleared = run_or_panic(
            &shell("printf '%s|%s' \"${HOME-unset}\" \"$FOO\"")
                .clear_env()
                .env("FOO", "bar"),
        );
        assert_eq!(cleared.stdout(), "unset|bar");
    }

    #[cfg(unix)]
    #[test]
    fn an_inherited_variable_can_be_removed() {
        let output = run_or_panic(
            &shell("printf '%s' \"${CARGO_PKG_NAME-unset}\"").env_remove("CARGO_PKG_NAME"),
        );

        assert_eq!(output.stdout(), "unset");
    }

    #[cfg(unix)]
    #[test]
    fn stdin_is_not_inherited_from_the_terminal() {
        let output = run_or_panic(&shell("read value; printf 'done'"));

        assert!(output.success());
        assert_eq!(output.stdout(), "done");
    }
}
