//! Recording whether a step reports that a restart is needed, for structured
//! reports.

use std::cell::RefCell;

thread_local! {
    static RESTART_NEEDED: RefCell<bool> = const { RefCell::new(false) };
}

/// Records that the current step reports a restart is needed.
pub fn record() {
    RESTART_NEEDED.with(|needed| *needed.borrow_mut() = true);
}

/// Takes the restart-needed flag recorded on the current thread, resetting it
/// so it does not leak into the next step.
pub fn take() -> bool {
    RESTART_NEEDED.with(|needed| std::mem::replace(&mut *needed.borrow_mut(), false))
}
