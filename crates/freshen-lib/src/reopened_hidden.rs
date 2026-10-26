//! Recording apps that were reopened hidden, produced by a step for
//! structured reports.

use std::cell::RefCell;

thread_local! {
    static REOPENED_HIDDEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Records an app that was reopened hidden.
pub fn record(name: impl Into<String>) {
    REOPENED_HIDDEN.with(|apps| apps.borrow_mut().push(name.into()));
}

/// Takes the apps recorded on the current thread.
pub fn take() -> Vec<String> {
    REOPENED_HIDDEN.with(|apps| apps.borrow_mut().drain(..).collect())
}
