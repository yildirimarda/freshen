//! Routing for human-readable progress, warnings and cleanup logs.
//!
//! The plain-text summary is freshen's default stdout output. When the CLI is
//! asked for a structured report (`--json` or `--markdown`) it calls
//! [`route_to_stderr`] first, so every human-readable line goes to stderr and
//! stdout contains nothing but the report.
//!
//! The `human!` and `human_print!` macros replace `println!` and `print!`
//! for library output. Errors written with `eprintln!` already go to stderr
//! and are not affected.

use std::cell::Cell;
use std::fmt;
use std::io::{self, Write};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Stdout,
    Stderr,
}

thread_local! {
    static HUMAN_STREAM: Cell<Stream> = const { Cell::new(Stream::Stdout) };
}

/// How much human-readable progress output freshen prints.
///
/// `--quiet` suppresses informational progress lines, `--verbose` adds
/// diagnostic detail, and the default (`Normal`) prints the ordinary progress
/// lines. Errors written with `eprintln!` always go to stderr and are not
/// affected by this level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputLevel {
    /// Print only errors, not progress or informational lines.
    Quiet,
    /// Print progress and informational lines (the default).
    #[default]
    Normal,
    /// Print progress plus verbose diagnostic detail.
    Verbose,
}

impl OutputLevel {
    #[must_use]
    fn rank(self) -> u8 {
        match self {
            Self::Quiet => 0,
            Self::Normal => 1,
            Self::Verbose => 2,
        }
    }
}

thread_local! {
    static OUTPUT_LEVEL: Cell<OutputLevel> = const { Cell::new(OutputLevel::Normal) };
}

/// Sets the human-readable output level for the current thread.
///
/// Called once from the binary's `main` before any step runs, so every
/// `human!`/`print_progress` and `print_verbose` call observes the level the
/// user asked for.
pub fn set_output_level(level: OutputLevel) {
    OUTPUT_LEVEL.with(|cell| cell.set(level));
}

/// Returns the current output level of this thread.
///
/// Exposed for tests; production code reads it through the `human!` and
/// `human_verbose!` macros instead.
pub(crate) fn current_output_level() -> OutputLevel {
    OUTPUT_LEVEL.with(Cell::get)
}

/// Writes `args` as a line when the current output level is at least
/// `min_level`. `Quiet` prints nothing from `human!`, `Normal` prints the
/// ordinary progress lines, and `Verbose` additionally prints the detail
/// lines emitted with `print_verbose`.
pub(crate) fn write_line_at(min_level: OutputLevel, args: fmt::Arguments<'_>) {
    if current_output_level().rank() >= min_level.rank() {
        write_line(args);
    }
}

macro_rules! human {
    ($($arg:tt)*) => {
        $crate::output::write_line_at(
            $crate::output::OutputLevel::Normal,
            format_args!($($arg)*),
        )
    };
}

#[allow(unused_macros)]
macro_rules! human_print {
    ($($arg:tt)*) => {
        $crate::output::write(format_args!($($arg)*))
    };
}

pub(crate) use human;
pub(crate) use human_print;

/// Prints `line` as an ordinary progress line.
///
/// Honors the output level: shown in `Normal` and `Verbose`, suppressed in
/// `Quiet`. Intended for callers outside this crate (for example the test
/// probe steps); within the crate the `human!` macro is preferred.
pub fn print_progress(line: &str) {
    human!("{line}");
}

/// Prints `line` as verbose diagnostic detail.
///
/// Shown only in `Verbose`; suppressed in `Normal` and `Quiet`. Intended for
/// callers outside this crate.
pub fn print_verbose(line: &str) {
    write_line_at(OutputLevel::Verbose, format_args!("{line}"));
}

/// Sends human-readable progress and log lines to stderr from now on.
///
/// Used when a structured report owns stdout. The setting is per thread, so
/// callers on other threads are unaffected.
pub fn route_to_stderr() {
    HUMAN_STREAM.with(|stream| stream.set(Stream::Stderr));
}

/// Sends human-readable progress and log lines back to stdout.
pub fn route_to_stdout() {
    HUMAN_STREAM.with(|stream| stream.set(Stream::Stdout));
}

pub(crate) fn write_line(args: fmt::Arguments<'_>) {
    let _ = match HUMAN_STREAM.with(Cell::get) {
        Stream::Stdout => writeln!(io::stdout().lock(), "{args}"),
        Stream::Stderr => writeln!(io::stderr().lock(), "{args}"),
    };
}

pub(crate) fn write(args: fmt::Arguments<'_>) {
    let _ = match HUMAN_STREAM.with(Cell::get) {
        Stream::Stdout => write!(io::stdout().lock(), "{args}"),
        Stream::Stderr => write!(io::stderr().lock(), "{args}"),
    };
}

pub(crate) fn flush() {
    let _ = match HUMAN_STREAM.with(Cell::get) {
        Stream::Stdout => io::stdout().lock().flush(),
        Stream::Stderr => io::stderr().lock().flush(),
    };
}
