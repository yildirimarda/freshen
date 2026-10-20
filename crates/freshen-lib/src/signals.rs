//! Process-wide interrupt handling.
//!
//! The signal handler does as little as possible: it records that the user
//! asked the run to stop. The subprocess runner polls the flag and stops the
//! command it started, and the runner stops scheduling new steps. That keeps
//! the handler safe and lets the normal code path restore whatever was
//! stopped before printing the partial summary.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Records that an interrupt was requested.
pub fn request() {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Returns `true` once an interrupt has been requested.
#[must_use]
pub fn requested() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Clears the interrupt flag.
///
/// Restoration must run after the user interrupted the run, so the runner
/// clears the flag before it starts the restore commands. Tests use it to
/// isolate cases that request an interrupt.
#[doc(hidden)]
pub fn reset() {
    INTERRUPTED.store(false, Ordering::SeqCst);
}

/// Installs the handler for SIGINT and SIGTERM on Unix and Ctrl-C on Windows.
///
/// Calling this more than once is harmless: the first installation wins and
/// later calls report the same result.
///
/// # Errors
///
/// Returns [`SignalError`] when the operating system refuses to install the
/// handler.
pub fn install() -> Result<(), SignalError> {
    static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();

    match INSTALLED.get_or_init(|| ctrlc::set_handler(request).map_err(|error| error.to_string())) {
        Ok(()) => Ok(()),
        Err(message) => Err(SignalError(message.clone())),
    }
}

/// The interrupt handler could not be installed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not install the interrupt handler: {0}")]
pub struct SignalError(String);
