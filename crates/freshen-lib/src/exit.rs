//! Stable process exit codes.
//!
//! These values are part of freshen's public interface: scripts and CI jobs
//! branch on them, so they must not change without a major release. The same
//! table is documented in the README and in `freshen --help`.
//!
//! | Code | Meaning |
//! |------|---------|
//! | 0 | Success |
//! | 1 | A step failed or timed out (also used for startup errors) |
//! | 2 | `--check` found pending updates |
//! | 3 | A preflight or package-manager check failed |
//! | 130 | The run was interrupted |

use std::process::ExitCode;

/// Everything that was asked for completed successfully.
pub const OK: u8 = 0;

/// A step failed or timed out. Startup errors (bad arguments, a held lock, an
/// unsupported platform) use this code too because they also mean the run did
/// not complete.
pub const STEP_FAILURE: u8 = 1;

/// `--check` found at least one pending update.
pub const UPDATES_PENDING: u8 = 2;

/// A preflight check or the other-package-manager check failed.
pub const PREFLIGHT_FAILED: u8 = 3;

/// The run was interrupted by SIGINT or SIGTERM.
pub const INTERRUPTED: u8 = 130;

/// Every documented exit code and its meaning, in ascending order.
pub const DOCUMENTED: &[(u8, &str)] = &[
    (OK, "success"),
    (STEP_FAILURE, "a step failed or timed out"),
    (UPDATES_PENDING, "updates are pending (--check)"),
    (
        PREFLIGHT_FAILED,
        "a preflight or package-manager check failed",
    ),
    (INTERRUPTED, "the run was interrupted"),
];

/// Builds a process exit code from one of the stable values above.
#[must_use]
pub fn code(value: u8) -> ExitCode {
    ExitCode::from(value)
}

#[cfg(test)]
mod tests {
    use super::{
        code, DOCUMENTED, INTERRUPTED, OK, PREFLIGHT_FAILED, STEP_FAILURE, UPDATES_PENDING,
    };
    use std::process::ExitCode;

    #[test]
    fn exit_code_values_are_stable() {
        assert_eq!(OK, 0);
        assert_eq!(STEP_FAILURE, 1);
        assert_eq!(UPDATES_PENDING, 2);
        assert_eq!(PREFLIGHT_FAILED, 3);
        assert_eq!(INTERRUPTED, 130);
    }

    #[test]
    fn every_code_is_documented_once_in_ascending_order() {
        let values: Vec<u8> = DOCUMENTED.iter().map(|(value, _)| *value).collect();
        assert_eq!(
            values,
            [
                OK,
                STEP_FAILURE,
                UPDATES_PENDING,
                PREFLIGHT_FAILED,
                INTERRUPTED
            ]
        );
        assert!(DOCUMENTED.iter().all(|(_, meaning)| !meaning.is_empty()));
    }

    #[test]
    fn code_wraps_the_numeric_value() {
        for &(value, _) in DOCUMENTED {
            let actual = code(value);
            let expected = ExitCode::from(value);
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        }
    }
}
