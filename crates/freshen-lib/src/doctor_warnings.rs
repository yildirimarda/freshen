//! Recording `brew doctor` warnings produced by a step for structured reports.

use std::cell::RefCell;

use crate::homebrew::DoctorFinding;

thread_local! {
    static DOCTOR_WARNINGS: RefCell<Vec<DoctorFinding>> = const { RefCell::new(Vec::new()) };
}

/// Records a `brew doctor` warning produced by the current step.
pub fn record(finding: DoctorFinding) {
    DOCTOR_WARNINGS.with(|warnings| warnings.borrow_mut().push(finding));
}

/// Takes the doctor warnings recorded on the current thread.
pub fn take() -> Vec<DoctorFinding> {
    DOCTOR_WARNINGS.with(|warnings| warnings.borrow_mut().drain(..).collect())
}
