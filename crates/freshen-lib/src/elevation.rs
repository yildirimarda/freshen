//! Elevation (UAC) requests for steps that need administrator rights.
//!
//! On Windows, a step that needs administrator rights ([`crate::Step::needs_administrator`])
//! is re-launched through a single UAC prompt instead of being skipped.
//! Elevation is requested only for a step that is selected to run, and only
//! once per run, so the user sees at most one UAC prompt.
//!
//! The decision of *when* to request elevation is platform-neutral and tested
//! on every operating system. Only the probe that performs the actual UAC
//! request is Windows-only, behind [`mod@self::windows`], together with its
//! tests and imports. Parsers and decision logic stay platform-neutral so
//! Linux and macOS CI do not fail with `unused_imports`/`dead_code`.

/// The result of asking the elevation helper to request elevation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationOutcome {
    /// The elevation prompt was shown (or would be shown by a real helper).
    Requested,
    /// The helper could not show the prompt, so elevation is not available.
    Unavailable,
}

/// Why elevation was or was not requested for a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationDecision {
    /// Elevation should be requested: one UAC prompt.
    Request,
    /// The step does not need administrator rights; run it normally.
    NotNeeded,
    /// The process is already elevated; run the step without prompting.
    AlreadyElevated,
    /// The step is not selected to run; do not elevate.
    NotSelected,
    /// Elevation was already requested earlier in this run; one prompt only.
    AlreadyRequested,
}

/// Tracks whether elevation has already been requested in this run, so the
/// user sees at most one UAC prompt.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ElevationState {
    requested: bool,
}

impl ElevationState {
    /// Creates a state with no elevation requested yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when elevation has already been requested.
    #[must_use]
    pub fn is_requested(&self) -> bool {
        self.requested
    }

    /// Records that elevation was requested.
    pub fn mark_requested(&mut self) {
        self.requested = true;
    }
}

/// A helper that asks the operating system to elevate the current process,
/// showing a single UAC prompt on Windows.
///
/// `step_id` names the single step that should run elevated; the real
/// implementation passes it along (for example through `--only`) so only that
/// step is elevated rather than the whole run.
///
/// The real implementation is [`HostElevationProbe`] on Windows. Tests inject
/// a fake so no actual UAC prompt appears.
pub trait ElevationProbe: Sync {
    /// Requests elevation for `step_id`, showing one UAC prompt.
    ///
    /// Returns [`ElevationOutcome::Requested`] when the prompt was shown, or
    /// [`ElevationOutcome::Unavailable`] when elevation could not be requested.
    fn request_elevation(&self, step_id: &str) -> ElevationOutcome;
}

/// A probe that never elevates. Used by default so steps that need
/// administrator rights are skipped with [`crate::step::SkipReason::NeedsAdministrator`]
/// unless a real elevation probe is supplied.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoElevationProbe;

impl ElevationProbe for NoElevationProbe {
    fn request_elevation(&self, _step_id: &str) -> ElevationOutcome {
        ElevationOutcome::Unavailable
    }
}

/// Decides whether a step that needs administrator rights should trigger a UAC
/// prompt.
///
/// Elevation is requested only when the step needs administrator rights, is
/// selected to run, the process is not already elevated, and elevation has not
/// already been requested in this run (one prompt only). The decision is
/// platform-neutral.
#[must_use]
pub fn decide(
    needs_administrator: bool,
    is_selected: bool,
    is_elevated: bool,
    state: &ElevationState,
) -> ElevationDecision {
    if !needs_administrator {
        return ElevationDecision::NotNeeded;
    }
    if !is_selected {
        return ElevationDecision::NotSelected;
    }
    if is_elevated {
        return ElevationDecision::AlreadyElevated;
    }
    if state.is_requested() {
        return ElevationDecision::AlreadyRequested;
    }
    ElevationDecision::Request
}

#[cfg(windows)]
pub mod windows {
    use super::{ElevationOutcome, ElevationProbe};
    use std::process::{Command, Stdio};

    /// Requests elevation by re-launching the current process through
    /// PowerShell's `Start-Process -Verb RunAs`, which shows a single UAC
    /// prompt. `step_id` is passed to the re-launched process through `--only`
    /// so only that step runs elevated.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct HostElevationProbe;

    impl ElevationProbe for HostElevationProbe {
        fn request_elevation(&self, step_id: &str) -> ElevationOutcome {
            request_elevation_with(|| real_request(step_id))
        }
    }

    /// Performs the real elevation request through PowerShell's
    /// `Start-Process -Verb RunAs`.
    fn real_request(step_id: &str) -> ElevationOutcome {
        let Ok(exe) = std::env::current_exe() else {
            return ElevationOutcome::Unavailable;
        };
        let Some(powershell) = crate::shell::windows::powershell() else {
            return ElevationOutcome::Unavailable;
        };
        let script = format!(
            "Start-Process -FilePath '{exe}' -Verb RunAs -ArgumentList '--only {step_id}'",
            exe = exe.display(),
        );
        Command::new(powershell.command())
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ElevationOutcome::Requested)
            .unwrap_or(ElevationOutcome::Unavailable)
    }

    /// Requests elevation through `request`, which is injected so tests can
    /// verify the call without showing a UAC prompt.
    #[must_use]
    pub fn request_elevation_with(request: impl FnOnce() -> ElevationOutcome) -> ElevationOutcome {
        request()
    }

    #[cfg(test)]
    mod tests {
        use super::request_elevation_with;
        use super::ElevationOutcome;

        #[test]
        fn request_elevation_forwards_to_the_injected_helper() {
            let mut called = false;
            let outcome = request_elevation_with(|| {
                called = true;
                ElevationOutcome::Requested
            });

            assert!(called);
            assert_eq!(outcome, ElevationOutcome::Requested);
        }

        #[test]
        fn request_elevation_propagates_the_helper_outcome() {
            let outcome = request_elevation_with(|| ElevationOutcome::Unavailable);
            assert_eq!(outcome, ElevationOutcome::Unavailable);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{decide, ElevationDecision, ElevationOutcome, ElevationState};

    #[test]
    fn decide_requests_elevation_for_selected_non_elevated_step() {
        let state = ElevationState::new();
        assert_eq!(
            decide(true, true, false, &state),
            ElevationDecision::Request
        );
    }

    #[test]
    fn decide_requests_elevation_for_the_single_selected_step() {
        let state = ElevationState::new();
        assert_eq!(
            decide(true, true, false, &state),
            ElevationDecision::Request
        );
        assert_eq!(
            decide(true, false, false, &state),
            ElevationDecision::NotSelected
        );
    }

    #[test]
    fn decide_does_not_request_when_the_step_is_not_selected() {
        let state = ElevationState::new();
        assert_eq!(
            decide(true, false, false, &state),
            ElevationDecision::NotSelected
        );
    }

    #[test]
    fn decide_does_not_request_when_already_elevated() {
        let state = ElevationState::new();
        assert_eq!(
            decide(true, true, true, &state),
            ElevationDecision::AlreadyElevated
        );
    }

    #[test]
    fn decide_does_not_request_when_the_step_does_not_need_administrator() {
        let state = ElevationState::new();
        assert_eq!(
            decide(false, true, false, &state),
            ElevationDecision::NotNeeded
        );
    }

    #[test]
    fn decide_requests_elevation_only_once_per_run() {
        let mut state = ElevationState::new();
        assert_eq!(
            decide(true, true, false, &state),
            ElevationDecision::Request
        );
        state.mark_requested();
        assert_eq!(
            decide(true, true, false, &state),
            ElevationDecision::AlreadyRequested
        );
    }

    #[test]
    fn elevation_state_starts_with_no_elevation_requested() {
        let state = ElevationState::new();
        assert!(!state.is_requested());
    }

    #[test]
    fn elevation_outcomes_are_distinct() {
        assert_ne!(ElevationOutcome::Requested, ElevationOutcome::Unavailable);
    }
}
