//! Recording `brew bundle cleanup` packages produced by a step for structured reports.

use std::cell::RefCell;

use crate::homebrew::BundlePackage;

thread_local! {
    static BUNDLE_CLEANUP_PACKAGES: RefCell<Vec<BundlePackage>> = const { RefCell::new(Vec::new()) };
}

/// Records a `brew bundle cleanup` package produced by the current step.
pub fn record(package: BundlePackage) {
    BUNDLE_CLEANUP_PACKAGES.with(|packages| packages.borrow_mut().push(package));
}

/// Takes the bundle cleanup packages recorded on the current thread.
pub fn take() -> Vec<BundlePackage> {
    BUNDLE_CLEANUP_PACKAGES.with(|packages| packages.borrow_mut().drain(..).collect())
}
