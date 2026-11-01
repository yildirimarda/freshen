//! Recording the services that need a restart after a step, for structured
//! reports.

use std::cell::RefCell;

thread_local! {
    static SERVICES_NEEDING_RESTART: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Records a service that needs a restart after the current step.
pub fn record(name: impl Into<String>) {
    SERVICES_NEEDING_RESTART.with(|services| services.borrow_mut().push(name.into()));
}

/// Takes the services needing restart recorded on the current thread, resetting
/// it so it does not leak into the next step.
pub fn take() -> Vec<String> {
    SERVICES_NEEDING_RESTART.with(|services| services.borrow_mut().drain(..).collect())
}
