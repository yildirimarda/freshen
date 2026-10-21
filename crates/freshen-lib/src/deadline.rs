//! Cooperative deadlines for the step currently running on this thread.
//!
//! The runner installs a deadline around every step. Subprocesses poll it the
//! same way they poll the interrupt flag, so a hung command is stopped when
//! its step or the whole run runs out of time. Keeping the deadline in a
//! thread-local means parallel runners (and parallel tests) never share one.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// The point in time at which the current step must stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Deadline {
    at: Instant,
    limit: Duration,
}

impl Deadline {
    /// Creates a deadline `limit` from `now`.
    pub(crate) fn after(now: Instant, limit: Duration) -> Self {
        Self {
            at: now + limit,
            limit,
        }
    }

    /// Creates a deadline at `at`, reporting `limit` as the configured bound.
    pub(crate) fn new(at: Instant, limit: Duration) -> Self {
        Self { at, limit }
    }

    /// Instant at which the deadline expires.
    pub(crate) fn at(self) -> Instant {
        self.at
    }

    /// Configured time limit this deadline represents.
    pub(crate) fn limit(self) -> Duration {
        self.limit
    }

    /// Returns `true` once the deadline has passed.
    pub(crate) fn expired(self) -> bool {
        Instant::now() >= self.at
    }
}

thread_local! {
    static CURRENT: Cell<Option<Deadline>> = const { Cell::new(None) };
}

/// Installs `deadline` for the current thread until the guard is dropped.
pub(crate) fn enter(deadline: Option<Deadline>) -> Guard {
    let previous = CURRENT.with(|current| current.replace(deadline));
    Guard(previous)
}

/// The deadline of the step running on this thread, if any.
pub(crate) fn current() -> Option<Deadline> {
    CURRENT.with(Cell::get)
}

/// Returns `true` when the current thread's deadline has passed.
pub(crate) fn expired() -> bool {
    current().is_some_and(Deadline::expired)
}

/// Restores the previous deadline when it is dropped.
pub(crate) struct Guard(Option<Deadline>);

impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::{enter, expired};
    use std::time::{Duration, Instant};

    #[test]
    fn a_deadline_expires_and_the_guard_restores_the_previous_one() {
        assert!(!expired());
        {
            let _guard = enter(Some(super::Deadline::after(
                Instant::now(),
                Duration::from_millis(1),
            )));
            assert!(!expired());
            std::thread::sleep(Duration::from_millis(10));
            assert!(expired());
        }
        assert!(!expired());
    }
}
