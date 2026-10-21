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

macro_rules! human {
    ($($arg:tt)*) => {
        $crate::output::write_line(format_args!($($arg)*))
    };
}

macro_rules! human_print {
    ($($arg:tt)*) => {
        $crate::output::write(format_args!($($arg)*))
    };
}

pub(crate) use human;
pub(crate) use human_print;
