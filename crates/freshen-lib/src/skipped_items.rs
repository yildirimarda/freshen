//! Recording skipped items produced by a step for structured reports.

use std::cell::RefCell;

thread_local! {
    static SKIPPED_ITEMS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Records a skipped item produced by the current step.
pub fn record(name: impl Into<String>) {
    SKIPPED_ITEMS.with(|items| items.borrow_mut().push(name.into()));
}

/// Takes the skipped items recorded on the current thread.
pub fn take() -> Vec<String> {
    SKIPPED_ITEMS.with(|items| items.borrow_mut().drain(..).collect())
}
