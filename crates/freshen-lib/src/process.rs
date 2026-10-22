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
//! * a timeout or interrupt stops the whole process tree (a process group on
//!   Unix, a job object on Windows), so a grandchild cannot keep the output
//!   pipes open after the step is gone;
//! * on Windows the child is created suspended and only resumed after it has
//!   joined the job, so even a grandchild started immediately is covered;
//! * the environment is inherited by default and can be extended, reduced or
//!   cleared;
//! * a command can be retried with exponential backoff.
//!
//! The runner never inherits stdin: a package manager must not block waiting
//! for input from the terminal that freshen itself was started from.

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
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

/// Why a retry backoff wait ended.
enum RetryWait {
    Completed,
    Interrupted,
    TimedOut,
}

/// Waits `delay` before a retry without ignoring an interrupt or a deadline.
///
/// The wait is split into short slices so an interrupt stops the run promptly,
/// and it is capped by the step's deadline so a long backoff can never run
/// past the time the step is allowed to take.
fn wait_before_retry(delay: Duration) -> RetryWait {
    let started = Instant::now();

    loop {
        if signals::requested() {
            return RetryWait::Interrupted;
        }

        let deadline = deadline::current();
        if deadline.is_some_and(deadline::Deadline::expired) {
            return RetryWait::TimedOut;
        }

        let remaining = delay.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return RetryWait::Completed;
        }

        let until_deadline = deadline.map_or(remaining, |deadline| {
            deadline.at().saturating_duration_since(Instant::now())
        });
        let slice = remaining.min(until_deadline).min(POLL_INTERVAL);
        if !slice.is_zero() {
            thread::sleep(slice);
        }
    }
}

/// How many times a network command is retried after its first attempt.
pub const NETWORK_RETRIES: u32 = 2;

/// How often a command is started again while its executable is busy.
///
/// A file that is open for writing cannot be executed, so a tool that is
/// being replaced at that very moment answers "Text file busy". That is a
/// moment in time and not a broken installation, so the command is started
/// again after a short pause instead of failing the step.
const BUSY_RETRIES: u32 = 40;

/// How long to wait before a command that was busy is started again.
const BUSY_RETRY_DELAY: Duration = Duration::from_millis(50);

/// The kind of network failure found in a command's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkFailure {
    /// A temporary problem that may disappear when the request is repeated,
    /// such as a reset connection, a timeout or an HTTP 5xx response.
    Transient,
    /// The registry cannot be reached at all, such as a DNS failure, a
    /// refused connection or an unreachable network.
    Unreachable,
}

/// Classifies network failures from command output.
///
/// Package managers and the download tools they call print very different
/// messages. This recognises the common curl, HTTP and package-manager
/// phrases so a transient error can be retried and a permanently unreachable
/// registry can skip the step instead of failing the whole run.
#[must_use]
pub fn network_failure(text: &str) -> Option<NetworkFailure> {
    let text = text.to_ascii_lowercase();

    const UNREACHABLE: &[&str] = &[
        "could not resolve host",
        "couldn't resolve host",
        "could not resolve proxy",
        "name or service not known",
        "nodename nor servname provided",
        "network is unreachable",
        "no route to host",
        "connection refused",
        "failed to connect",
        "could not connect",
        "can't connect",
        "unable to connect",
        "host not found",
        "getaddrinfo",
        "proxy connect aborted",
        "offline",
    ];
    if UNREACHABLE.iter().any(|marker| text.contains(marker)) {
        return Some(NetworkFailure::Unreachable);
    }

    const TRANSIENT: &[&str] = &[
        "temporary failure in name resolution",
        "connection reset",
        "connection was reset",
        "connection aborted",
        "connection closed",
        "broken pipe",
        "unexpected eof",
        "unexpected end of file",
        "failure when receiving data",
        "ssl connect error",
        "ssl_error_syscall",
        "tls handshake timeout",
        "operation timed out",
        "connection timed out",
        "request timed out",
        "temporarily unavailable",
        "try again",
        "rate limit",
        "too many requests",
        "http 429",
        " 429 ",
        "http 500",
        "http 502",
        "http 503",
        "http 504",
        "status 500",
        "status 502",
        "status 503",
        "status 504",
        "internal server error",
        "bad gateway",
        "service unavailable",
        "gateway timeout",
        "returned error: 500",
        "returned error: 502",
        "returned error: 503",
        "returned error: 504",
        "error downloading",
        "failed to download",
        "download failed",
        "could not download",
        "unable to download",
        "error fetching",
        "failed to fetch",
        "network error",
        "connection error",
    ];
    if TRANSIENT.iter().any(|marker| text.contains(marker)) {
        return Some(NetworkFailure::Transient);
    }

    None
}

fn captured_network_failure(stdout: &[u8], stderr: &[u8]) -> Option<NetworkFailure> {
    let mut text = sanitize_output(&String::from_utf8_lossy(stdout));
    text.push('\n');
    text.push_str(&sanitize_output(&String::from_utf8_lossy(stderr)));
    network_failure(&text)
}

/// Turns captured bytes into the text the parsers read.
fn captured_text(bytes: &[u8]) -> String {
    sanitize_output(&String::from_utf8_lossy(bytes))
}

/// Removes the escape sequences and the progress redraws a tool wrote.
///
/// A tool that draws a progress bar rewrites the same line over and over and
/// ends it with a carriage return, and it colours its output with escape
/// sequences. The parsers read the captured text, so both would end up
/// inside the values they extract. The live output keeps the original bytes,
/// so the user still sees the progress.
#[must_use]
pub fn sanitize_output(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut line_start = 0;

    for (index, character) in text.char_indices() {
        match character {
            '\n' => {
                result.push_str(&strip_escape_sequences(&text[line_start..index]));
                result.push('\n');
                line_start = index + 1;
            }
            '\r' => {
                // A carriage return rewrites the line: what came before it is
                // replaced by what follows.
                line_start = index + 1;
            }
            _ => {}
        }
    }
    result.push_str(&strip_escape_sequences(&text[line_start..]));

    result
}

/// Removes the ANSI escape sequences from `text`.
fn strip_escape_sequences(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            result.push(character);
            continue;
        }

        match characters.peek() {
            // A control sequence: `ESC [`, its parameters and a final byte.
            Some(&'[') => {
                characters.next();
                for character in characters.by_ref() {
                    if matches!(character, '\u{40}'..='\u{7e}') {
                        break;
                    }
                }
            }
            // Any other escape sequence: `ESC` and one more byte.
            Some(_) => {
                characters.next();
            }
            None => {}
        }
    }

    result
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
    retry_transient_network_only: bool,
    backoff: Backoff,
    neutral_locale: bool,
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
            retry_transient_network_only: false,
            backoff: Backoff::default(),
            neutral_locale: false,
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
        self.retry_transient_network_only = false;
        self
    }

    /// Retries only transient network failures, up to `retries` times.
    ///
    /// A registry that is permanently unreachable (DNS failure, refused
    /// connection) is not retried: the caller can report the step as skipped
    /// instead of spending the whole backoff waiting for an answer that will
    /// not come. Timeouts are treated as transient because a stalled download
    /// often succeeds when it is started again.
    #[must_use]
    pub fn retry_transient_network(mut self, retries: u32) -> Self {
        self.retries = retries;
        self.retry_transient_network_only = true;
        self
    }

    /// Sets the delay policy used between retries.
    #[must_use]
    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// Runs the command in a neutral locale, so its output can be parsed.
    ///
    /// Package managers and system tools translate their messages and the
    /// labels of their tables, which would break every parser that reads
    /// them. The command therefore runs with `LC_ALL=C` and `LANG=C`, and on
    /// Windows with a UTF-8 console and without interactivity.
    ///
    /// The variables are applied last, so they win over the ones the user
    /// set in the environment freshen was started from.
    #[must_use]
    pub fn neutral_locale(mut self) -> Self {
        self.neutral_locale = true;
        self
    }

    /// Whether the command runs in a neutral locale.
    #[must_use]
    pub fn uses_neutral_locale(&self) -> bool {
        self.neutral_locale
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
                        stdout: captured_text(&stdout),
                        stderr: captured_text(&stderr),
                        status: None,
                        attempts: attempt.saturating_sub(1),
                        duration: started.elapsed(),
                        network_failure: None,
                    }),
                });
            }

            if attempt > 1 {
                let delay = self.backoff.delay_before(attempt);
                if matches!(
                    wait_before_retry(delay),
                    RetryWait::Interrupted | RetryWait::TimedOut
                ) {
                    continue;
                }
            }

            let result = self.run_attempt(&mut on_line)?;
            let attempt_network_failure =
                captured_network_failure(&result.captured.stdout, &result.captured.stderr);
            let retryable = self.should_retry(result.outcome, &result.captured);
            stdout.extend_from_slice(&result.captured.stdout);
            stderr.extend_from_slice(&result.captured.stderr);

            let outcome = result.outcome;
            if matches!(outcome, AttemptOutcome::Interrupted) {
                return Err(ProcessError::Interrupted { program });
            }
            if let AttemptOutcome::Exited(status) = outcome {
                if status.success() {
                    return Ok(ProcessOutput {
                        stdout: captured_text(&stdout),
                        stderr: captured_text(&stderr),
                        status: Some(status),
                        attempts: attempt,
                        duration: started.elapsed(),
                        network_failure: attempt_network_failure,
                    });
                }
            }

            if attempt >= max_attempts || !retryable {
                let output = Box::new(ProcessOutput {
                    stdout: captured_text(&stdout),
                    stderr: captured_text(&stderr),
                    status: match outcome {
                        AttemptOutcome::Exited(status) => Some(status),
                        AttemptOutcome::TimedOut | AttemptOutcome::Interrupted => None,
                    },
                    attempts: attempt,
                    duration: started.elapsed(),
                    network_failure: attempt_network_failure,
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

    fn should_retry(&self, outcome: AttemptOutcome, captured: &Captured) -> bool {
        if !self.retry_transient_network_only {
            return true;
        }

        match outcome {
            AttemptOutcome::Interrupted => false,
            AttemptOutcome::TimedOut => true,
            AttemptOutcome::Exited(_) => {
                captured_network_failure(&captured.stdout, &captured.stderr)
                    == Some(NetworkFailure::Transient)
            }
        }
    }

    /// Runs one attempt, streaming and capturing its output.
    fn run_attempt(
        &self,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> Result<AttemptResult, ProcessError> {
        let program = self.program.to_string_lossy().into_owned();
        #[cfg(windows)]
        let mut command = match crate::executable::resolve(&self.program) {
            Some(path) => Command::new(path),
            None => Command::new(&self.program),
        };
        #[cfg(not(windows))]
        let mut command = Command::new(&self.program);
        if self.neutral_locale {
            // The flag comes before the subcommand, because a tool that
            // knows it expects it as a global option.
            if let Some(flag) = crate::locale::non_interactive_flag(&self.program) {
                command.arg(flag);
            }
        }
        command
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;

            // Put the child in its own process group so a timeout or interrupt
            // can stop its descendants as well, without touching freshen.
            command.process_group(0);
        }

        #[cfg(windows)]
        {
            // Start the child suspended so it cannot run any code, and thus
            // cannot start a grandchild, before it joins the job object.
            win32::suspend_on_spawn(&mut command);
        }

        if self.env_clear {
            command.env_clear();
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        for key in &self.env_remove {
            command.env_remove(key);
        }
        if self.neutral_locale {
            crate::locale::with_neutral_locale(&mut command);
        }

        let mut child = spawn_child(&mut command, &program)?;
        let mut tree = match ProcessTree::new(&child) {
            Ok(tree) => tree,
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessError::Spawn { program, source });
            }
        };

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
                    tree.terminate(&mut child);
                    let _ = join_readers(readers, &program);
                    return Err(ProcessError::Wait { program, source });
                }
            }

            let timed_out = self
                .timeout
                .is_some_and(|limit| attempt_started.elapsed() >= limit)
                || deadline::expired();
            if timed_out {
                tree.terminate(&mut child);
                outcome = AttemptOutcome::TimedOut;
                break;
            }

            if signals::requested() {
                tree.terminate(&mut child);
                outcome = AttemptOutcome::Interrupted;
                break;
            }

            thread::sleep(POLL_INTERVAL);
        }
        tree.release();

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

/// Starts the command, retrying while its executable is busy.
///
/// A file that is open for writing cannot be executed, so a tool that is
/// being replaced at that moment answers "Text file busy". The window is
/// tiny, so the command is started again after a short pause instead of
/// failing the step. Every other error is reported right away, and the retry
/// stops as soon as the step runs out of time or the run is interrupted.
fn spawn_child(command: &mut Command, program: &str) -> Result<Child, ProcessError> {
    let mut attempt = 0;
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(source)
                if is_busy(&source)
                    && attempt < BUSY_RETRIES
                    && !deadline::expired()
                    && !signals::requested() =>
            {
                attempt += 1;
                thread::sleep(BUSY_RETRY_DELAY);
            }
            Err(source) => {
                return Err(ProcessError::Spawn {
                    program: program.to_owned(),
                    source,
                });
            }
        }
    }
}

/// Whether the error says that the executable was busy.
#[cfg(unix)]
fn is_busy(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(nix::errno::Errno::ETXTBSY as i32)
}

/// Whether the error says that the executable was busy.
///
/// Windows reports a locked executable as a sharing violation.
#[cfg(windows)]
fn is_busy(error: &std::io::Error) -> bool {
    let sharing_violation = windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;
    error.raw_os_error() == Some(sharing_violation as i32)
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

/// The Win32 calls needed to start a child suspended and resume it after it
/// has joined a job object.
///
/// Everything unsafe is confined to this module; the rest of the process
/// handling uses these safe wrappers.
#[cfg(windows)]
mod win32 {
    #![allow(unsafe_code)]

    use std::io;
    use std::process::Command;

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{
        OpenThread, ResumeThread, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
    };

    /// Creates the child suspended so it cannot run before it joins a job.
    pub(super) fn suspend_on_spawn(command: &mut Command) {
        use std::os::windows::process::CommandExt;

        command.creation_flags(CREATE_SUSPENDED);
    }

    /// Resumes every thread of a process that was created suspended.
    pub(super) fn resume(pid: u32) -> io::Result<()> {
        // SAFETY: the snapshot handle is closed before returning and `entry`
        // is a correctly sized buffer owned for the duration of the loop.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut resumed = false;
        let mut result = Err(io::Error::other(format!(
            "no thread found for suspended process {pid}"
        )));

        // SAFETY: `snapshot` is a valid snapshot handle and `entry` is a
        // valid THREADENTRY32 for the duration of the iteration.
        let mut found = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while found {
            if entry.th32OwnerProcessID == pid {
                // SAFETY: `entry` describes a real thread and the returned
                // handle is closed below.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    result = Err(io::Error::last_os_error());
                } else {
                    // SAFETY: `thread` is a valid handle opened with
                    // THREAD_SUSPEND_RESUME. u32::MAX signals failure; any
                    // other value is the previous suspend count.
                    let previous = unsafe { ResumeThread(thread) };
                    result = if previous == u32::MAX {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(())
                    };
                    // SAFETY: `thread` is a handle we own and no longer use.
                    unsafe { CloseHandle(thread) };
                }
                resumed = true;
                break;
            }
            // SAFETY: `snapshot` and `entry` remain valid between calls.
            found = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }

        // SAFETY: `snapshot` is a handle we own and no longer use.
        unsafe { CloseHandle(snapshot) };

        if resumed {
            result
        } else {
            Err(io::Error::other(format!(
                "no thread found for suspended process {pid}"
            )))
        }
    }

    /// Returns the direct children of `parent` currently in the process table.
    #[cfg(test)]
    pub(super) fn child_process_ids(parent: u32) -> Vec<u32> {
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
        };

        // SAFETY: the snapshot handle is closed before returning and `entry`
        // is a correctly sized buffer owned for the duration of the loop.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }

        let mut entry = PROCESSENTRY32 {
            dwSize: std::mem::size_of::<PROCESSENTRY32>() as u32,
            ..Default::default()
        };
        let mut children = Vec::new();
        // SAFETY: `snapshot` is valid and `entry` is a valid PROCESSENTRY32.
        let mut found = unsafe { Process32First(snapshot, &mut entry) } != 0;
        while found {
            if entry.th32ParentProcessID == parent {
                children.push(entry.th32ProcessID);
            }
            // SAFETY: `snapshot` and `entry` remain valid between calls.
            found = unsafe { Process32Next(snapshot, &mut entry) } != 0;
        }

        // SAFETY: `snapshot` is a handle we own and no longer use.
        unsafe { CloseHandle(snapshot) };
        children
    }

    /// Returns whether `pid` is still running.
    #[cfg(test)]
    pub(super) fn is_alive(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::STILL_ACTIVE;
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        // SAFETY: `OpenProcess` does not dereference any pointer; the handle
        // is closed below when valid.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return false;
        }
        let mut code = 0;
        // SAFETY: `process` is a valid handle and `code` is a valid out-pointer.
        let alive =
            unsafe { GetExitCodeProcess(process, &mut code) } != 0 && code == STILL_ACTIVE as u32;
        // SAFETY: `process` is a handle we own and no longer use.
        unsafe { CloseHandle(process) };
        alive
    }
}

/// Controls the lifetime of a child and every process it starts.
///
/// Killing only the direct child is not enough: a shell can leave a grandchild
/// running with the output pipes still open, which keeps the reader threads
/// alive and outlives the step. A timeout or interrupt therefore stops the
/// whole tree, using a process group on Unix and a job object on Windows.
struct ProcessTree {
    #[cfg(unix)]
    pgid: i32,
    #[cfg(windows)]
    job: Option<win32job::Job>,
}

impl ProcessTree {
    #[cfg(unix)]
    fn new(child: &Child) -> std::io::Result<Self> {
        Ok(Self {
            pgid: i32::try_from(child.id()).unwrap_or(i32::MAX),
        })
    }

    #[cfg(windows)]
    fn new(child: &Child) -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle;

        let mut limits = win32job::ExtendedLimitInfo::new();
        limits.limit_kill_on_job_close();
        let job = win32job::Job::create_with_limit_info(&limits)
            .ok()
            .and_then(|job| {
                job.assign_process(child.as_raw_handle() as isize).ok()?;
                Some(job)
            });

        // The child was created suspended, so it is in the job (when one could
        // be created) before its first instruction runs. Resume it even
        // without a job so the taskkill fallback in `terminate` still works.
        if let Err(error) = win32::resume(child.id()) {
            // Dropping the job kills the still-suspended child because the
            // job was created with kill-on-close.
            drop(job);
            return Err(error);
        }

        Ok(Self { job })
    }

    #[cfg(not(any(unix, windows)))]
    fn new(_child: &Child) -> std::io::Result<Self> {
        Ok(Self {})
    }

    /// Kills the child and all of its descendants, then reaps the child.
    fn terminate(&mut self, child: &mut Child) {
        #[cfg(unix)]
        {
            use nix::sys::signal::{killpg, Signal};
            use nix::unistd::Pid;

            let _ = killpg(Pid::from_raw(self.pgid), Signal::SIGKILL);
        }

        #[cfg(windows)]
        {
            if let Some(job) = self.job.take() {
                // The job was created with kill-on-close, so dropping the last
                // handle terminates every process that belongs to it.
                drop(job);
            } else {
                let _ = Command::new("taskkill")
                    .args(["/T", "/F", "/PID", &child.id().to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }

        let _ = child.wait();
    }

    /// Releases the tree after the child exited on its own.
    ///
    /// On Windows this clears kill-on-close first, so a process the command
    /// deliberately left running is not terminated after a successful step.
    fn release(&mut self) {
        #[cfg(unix)]
        {
            let _ = self.pgid;
        }

        #[cfg(windows)]
        {
            if let Some(job) = self.job.take() {
                if let Ok(mut limits) = job.query_extended_limit_info() {
                    limits.clear_limits();
                    let _ = job.set_extended_limit_info(&limits);
                }
                drop(job);
            }
        }
    }
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
    network_failure: Option<NetworkFailure>,
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

    /// Network failure found in the final attempt's output, when there was one.
    #[must_use]
    pub fn network_failure(&self) -> Option<NetworkFailure> {
        self.network_failure
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

    /// Network failure found in the captured output, when there is one.
    #[must_use]
    pub fn network_failure(&self) -> Option<NetworkFailure> {
        self.output()?.network_failure()
    }

    /// Returns `true` when the captured output describes a network failure.
    #[must_use]
    pub fn is_network_failure(&self) -> bool {
        self.network_failure().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{network_failure, Backoff, CommandSpec, NetworkFailure};
    #[cfg(unix)]
    use super::{OutputStream, ProcessError};
    use std::time::{Duration, Instant};

    fn backoff() -> Backoff {
        Backoff::new(Duration::from_millis(10), Duration::from_millis(100))
    }

    #[cfg(unix)]
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
    fn classifies_transient_and_unreachable_network_errors() {
        for message in [
            "curl: (56) Failure when receiving data from the peer",
            "HTTP 503 Service Unavailable",
            "curl: (28) Operation timed out",
            "Temporary failure in name resolution",
            "connection reset by peer",
            "Too Many Requests (429)",
        ] {
            assert_eq!(
                network_failure(message),
                Some(NetworkFailure::Transient),
                "{message}"
            );
        }

        for message in [
            "curl: (6) Could not resolve host: formulae.brew.sh",
            "curl: (7) Failed to connect to github.com port 443",
            "Network is unreachable",
            "connect: Connection refused",
        ] {
            assert_eq!(
                network_failure(message),
                Some(NetworkFailure::Unreachable),
                "{message}"
            );
        }

        assert_eq!(network_failure("Error: formula not found"), None);
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

    #[test]
    fn a_command_is_neutral_locale_only_when_it_is_asked_for() {
        assert!(!CommandSpec::new("brew").uses_neutral_locale());
        assert!(CommandSpec::new("brew")
            .neutral_locale()
            .uses_neutral_locale());
    }

    #[cfg(unix)]
    #[test]
    fn a_neutral_locale_command_runs_with_the_c_locale() {
        let spec = shell("printf '%s|%s|%s' \"$LC_ALL\" \"$LANG\" \"$LANGUAGE\"").neutral_locale();

        let output = run_or_panic(&spec);

        assert_eq!(output.stdout(), "C|C|C");
    }

    #[cfg(unix)]
    #[test]
    fn the_neutral_locale_overrides_the_one_of_the_shell() {
        let spec = shell("printf '%s' \"$LC_ALL\"")
            .env("LC_ALL", "tr_TR.UTF-8")
            .neutral_locale();

        let output = run_or_panic(&spec);

        assert_eq!(output.stdout(), "C");
    }

    #[cfg(unix)]
    #[test]
    fn the_captured_output_has_no_escape_sequences() {
        let spec = shell(r"printf '\033[32mgreen\033[0m\n'");

        let output = run_or_panic(&spec);

        assert_eq!(output.stdout(), "green\n");
    }

    #[test]
    fn removes_escape_sequences_and_progress_redraws() {
        assert_eq!(super::sanitize_output("\u{1b}[32mgreen\u{1b}[0m"), "green");
        assert_eq!(super::sanitize_output("50%\r75%\r100%"), "100%");
        assert_eq!(super::sanitize_output("first\nsecond\n"), "first\nsecond\n");
        assert_eq!(super::sanitize_output("plain"), "plain");
    }

    #[cfg(unix)]
    fn shell(script: &str) -> CommandSpec {
        CommandSpec::new("/bin/sh").arg("-c").arg(script)
    }

    #[cfg(unix)]
    fn process_is_alive(pid: i32) -> bool {
        #[cfg(target_os = "linux")]
        {
            match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                Ok(stat) => stat
                    .rsplit_once(')')
                    .and_then(|(_, rest)| rest.split_whitespace().next())
                    .is_some_and(|state| state != "Z"),
                Err(_) => false,
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            use nix::errno::Errno;
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            match kill(Pid::from_raw(pid), None::<Signal>) {
                Ok(()) | Err(Errno::EPERM) => true,
                Err(_) => false,
            }
        }
    }

    #[cfg(unix)]
    fn wait_until_gone(pid: i32, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if !process_is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        !process_is_alive(pid)
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
    fn a_timeout_kills_grandchildren_that_keep_the_output_pipes_open() {
        let marker =
            std::env::temp_dir().join(format!("freshen-process-grandchild-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!(
            "sleep 30 & child=$!; printf '%s\\n' \"$child\" > '{marker}'; wait",
            marker = marker.display()
        );
        let started = Instant::now();

        let result = shell(&script).timeout(Duration::from_millis(500)).run();
        let elapsed = started.elapsed();

        let pid = std::fs::read_to_string(&marker)
            .unwrap_or_else(|error| panic!("grandchild did not record its pid: {error}"))
            .trim()
            .parse::<i32>()
            .unwrap_or_else(|error| panic!("invalid grandchild pid: {error}"));
        let _ = std::fs::remove_file(&marker);

        assert!(
            wait_until_gone(pid, Duration::from_secs(2)),
            "grandchild {pid} outlived the timed-out step"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "the timed-out command took {elapsed:?}"
        );
        match result {
            Err(ProcessError::TimedOut { .. }) => {}
            other => panic!("expected a timeout, got {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_child_joins_its_job_before_it_can_start_a_grandchild() {
        use super::{win32, ProcessTree};

        let directory = std::env::temp_dir();
        let marker_name = format!("freshen-windows-job-{}.txt", std::process::id());
        let marker = directory.join(&marker_name);
        let _ = std::fs::remove_file(&marker);

        // The script is handed to `cmd.exe` without a single quote character:
        // the C runtime escapes a quote inside an argument with a backslash,
        // which `cmd.exe` does not understand, so the redirection would write
        // to a file with a mangled name. The marker is therefore written
        // relative to the working directory of the child.
        let script = format!(
            "echo started>{marker_name} & start /B ping -n 30 127.0.0.1 >nul & ping -n 30 127.0.0.1 >nul"
        );

        let mut command = std::process::Command::new("cmd");
        command
            .args(["/C", &script])
            .current_dir(&directory)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        win32::suspend_on_spawn(&mut command);

        let mut child = command
            .spawn()
            .unwrap_or_else(|error| panic!("failed to spawn suspended child: {error}"));

        // A suspended process cannot execute its first instruction, so the
        // marker must not exist until the job assignment has happened.
        std::thread::sleep(Duration::from_millis(250));
        assert!(
            !marker.exists(),
            "the child ran before it was assigned to the job"
        );

        let mut tree = ProcessTree::new(&child)
            .unwrap_or_else(|error| panic!("failed to assign and resume the child: {error}"));
        assert!(
            tree.job.is_some(),
            "the child was not assigned to a job object"
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() {
            assert!(
                Instant::now() < deadline,
                "the resumed child never wrote its marker"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        // `start /B` runs immediately after the marker, so this grandchild is
        // created as soon as the child gets to run.
        let grandchild = wait_for_child_of(child.id(), Duration::from_secs(10));

        tree.terminate(&mut child);
        let _ = std::fs::remove_file(&marker);

        assert!(
            wait_until_gone(grandchild, Duration::from_secs(5)),
            "grandchild {grandchild} outlived the job"
        );
    }

    #[cfg(windows)]
    fn wait_for_child_of(parent: u32, limit: Duration) -> u32 {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(&pid) = super::win32::child_process_ids(parent).first() {
                return pid;
            }
            assert!(
                Instant::now() < deadline,
                "the child never started a grandchild"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(windows)]
    fn wait_until_gone(pid: u32, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if !super::win32::is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        !super::win32::is_alive(pid)
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
    fn retry_backoff_stops_at_the_step_deadline_instead_of_overshooting_it() {
        let _guard = crate::deadline::enter(Some(crate::deadline::Deadline::after(
            Instant::now(),
            Duration::from_millis(100),
        )));
        let spec = shell("exit 1").retries(5).backoff(Backoff::new(
            Duration::from_secs(30),
            Duration::from_secs(30),
        ));
        let started = Instant::now();

        let result = spec.run();
        let elapsed = started.elapsed();

        match result {
            Err(ProcessError::TimedOut {
                timeout, output, ..
            }) => {
                assert_eq!(timeout, Duration::from_millis(100));
                assert_eq!(output.attempts(), 1);
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert!(
            elapsed < Duration::from_secs(5),
            "the retry backoff overshot the deadline: {elapsed:?}"
        );
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
    fn a_command_whose_executable_is_busy_is_started_again() {
        use std::fs::OpenOptions;
        use std::os::unix::fs::PermissionsExt;

        let directory =
            std::env::temp_dir().join(format!("freshen-process-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory)
            .unwrap_or_else(|error| panic!("failed to create {directory:?}: {error}"));
        let tool = directory.join("busy-tool");
        std::fs::write(&tool, "#!/bin/sh\nprintf 'ran\\n'\n")
            .unwrap_or_else(|error| panic!("failed to write the tool: {error}"));
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("failed to make the tool executable: {error}"));

        // Hold the tool open for writing, the way an installer that replaces
        // it does, and let go of it again a moment later. An executable that
        // is open for writing cannot be started, so the first attempt fails
        // with "Text file busy".
        let held = OpenOptions::new()
            .write(true)
            .open(&tool)
            .unwrap_or_else(|error| panic!("failed to hold the tool open: {error}"));
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(held);
        });

        let result = CommandSpec::new(&tool).run();
        release
            .join()
            .unwrap_or_else(|error| panic!("the holder panicked: {error:?}"));

        let output = match result {
            Ok(output) => output,
            Err(error) => panic!("command failed: {error}"),
        };
        assert!(output.success());
        assert_eq!(output.stdout(), "ran\n");

        let _ = std::fs::remove_dir_all(&directory);
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
    fn retries_transient_network_failures_until_they_succeed() {
        let marker = std::env::temp_dir().join(format!(
            "freshen-process-network-retry-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let script = format!(
            "if [ -f '{marker}' ]; then printf 'ok\\n'; else : > '{marker}'; printf 'HTTP 503 Service Unavailable\\n' >&2; exit 1; fi",
            marker = marker.display()
        );
        let spec = shell(&script)
            .retry_transient_network(2)
            .backoff(Backoff::NONE);

        let result = spec.run();
        let _ = std::fs::remove_file(&marker);

        let output = match result {
            Ok(output) => output,
            Err(error) => panic!("command failed: {error}"),
        };
        assert_eq!(output.attempts(), 2);
        assert_eq!(output.stdout(), "ok\n");
    }

    #[cfg(unix)]
    #[test]
    fn an_unreachable_registry_is_not_retried() {
        let spec = shell("printf 'Could not resolve host: formulae.brew.sh\\n' >&2; exit 1")
            .retry_transient_network(3)
            .backoff(Backoff::NONE);

        match spec.run() {
            Err(error @ ProcessError::Failed { .. }) => {
                assert_eq!(error.network_failure(), Some(NetworkFailure::Unreachable));
                assert_eq!(error.output().map(super::ProcessOutput::attempts), Some(1));
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_non_network_failure_is_not_retried_by_the_network_policy() {
        let spec = shell("printf 'Error: formula not found\\n' >&2; exit 1")
            .retry_transient_network(3)
            .backoff(Backoff::NONE);

        match spec.run() {
            Err(error @ ProcessError::Failed { .. }) => {
                assert_eq!(error.network_failure(), None);
                assert_eq!(error.output().map(super::ProcessOutput::attempts), Some(1));
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_network_failure_is_cleared_when_the_final_attempt_fails_for_another_reason() {
        let marker = std::env::temp_dir().join(format!(
            "freshen-process-network-then-error-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let script = format!(
            "if [ -f '{marker}' ]; then printf 'Error: formula not found\\n' >&2; else : > '{marker}'; printf 'HTTP 503 Service Unavailable\\n' >&2; fi; exit 1",
            marker = marker.display()
        );
        let spec = shell(&script)
            .retry_transient_network(2)
            .backoff(Backoff::NONE);

        let result = spec.run();
        let _ = std::fs::remove_file(&marker);

        match result {
            Err(error @ ProcessError::Failed { .. }) => {
                assert_eq!(error.network_failure(), None);
                assert_eq!(error.output().map(super::ProcessOutput::attempts), Some(2));
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
