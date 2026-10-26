//! Recording apps whose window was put back, produced by a step for
//! structured reports.

use std::cell::RefCell;

thread_local! {
    static RESTORED_WINDOWS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Records an app whose window was put back where it was.
pub fn record(name: impl Into<String>) {
    RESTORED_WINDOWS.with(|apps| apps.borrow_mut().push(name.into()));
}

/// Takes the apps recorded on the current thread.
pub fn take() -> Vec<String> {
    RESTORED_WINDOWS.with(|apps| apps.borrow_mut().drain(..).collect())
}
