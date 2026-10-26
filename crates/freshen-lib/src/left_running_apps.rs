//! Recording apps left running because they were updating themselves,
//! produced by a step for structured reports.

use std::cell::RefCell;

thread_local! {
    static LEFT_RUNNING_APPS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Records an app that was left running because it was updating itself.
pub fn record(name: impl Into<String>) {
    LEFT_RUNNING_APPS.with(|apps| apps.borrow_mut().push(name.into()));
}

/// Takes the apps recorded on the current thread.
pub fn take() -> Vec<String> {
    LEFT_RUNNING_APPS.with(|apps| apps.borrow_mut().drain(..).collect())
}
