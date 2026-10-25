//! Recording restarted Homebrew services produced by a step for structured
//! reports.

use std::cell::RefCell;

thread_local! {
    static RESTARTED_SERVICES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Records a restarted Homebrew service produced by the current step.
pub fn record(name: impl Into<String>) {
    RESTARTED_SERVICES.with(|services| services.borrow_mut().push(name.into()));
}

/// Takes the restarted services recorded on the current thread.
pub fn take() -> Vec<String> {
    RESTARTED_SERVICES.with(|services| services.borrow_mut().drain(..).collect())
}
