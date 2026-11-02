//! Restarting services that need a restart after a Linux upgrade.
//!
//! The apt, dnf and zypper steps record the services that need a restart
//! (reported by `needrestart`, `needs-restarting` or `zypper ps -s`) during
//! their upgrade. With `--restart-services`, freshen restarts those services
//! after the run, capturing each service's previous enabled/running state
//! first so it can be restored on error or interrupt.

use crate::process::{CommandSpec, ProcessError};

/// The captured enabled/running state of one service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ServiceState {
    /// Whether `systemctl is-enabled` reported the service as enabled.
    pub(crate) enabled: bool,
    /// Whether `systemctl is-active` reported the service as active.
    pub(crate) running: bool,
}

#[cfg(unix)]
impl ServiceState {
    /// Captures the current enabled/running state of `name`.
    pub(crate) fn capture(name: &str) -> Self {
        let enabled = CommandSpec::new("systemctl")
            .args(["is-enabled", name])
            .run()
            .map(|output| output.stdout().trim() == "enabled")
            .unwrap_or(false);
        let running = CommandSpec::new("systemctl")
            .args(["is-active", name])
            .run()
            .map(|output| output.stdout().trim() == "active")
            .unwrap_or(false);
        Self { enabled, running }
    }

    /// Restarts `name` with `systemctl restart`.
    pub(crate) fn restart(name: &str) -> Result<(), ProcessError> {
        CommandSpec::new("systemctl")
            .args(["restart", name])
            .run()
            .map(|_| ())
    }

    /// Restores `name` to `state` (enabled/disabled, then start/stop).
    pub(crate) fn restore(name: &str, state: ServiceState) -> Result<(), ProcessError> {
        if state.enabled {
            CommandSpec::new("systemctl").args(["enable", name]).run()?;
        } else {
            CommandSpec::new("systemctl")
                .args(["disable", name])
                .run()?;
        }
        if state.running {
            CommandSpec::new("systemctl").args(["start", name]).run()?;
        } else {
            CommandSpec::new("systemctl").args(["stop", name]).run()?;
        }
        Ok(())
    }
}

#[cfg(not(unix))]
impl ServiceState {
    /// Non-Unix platforms have no systemd services to restart.
    pub(crate) fn capture(_name: &str) -> Self {
        Self {
            enabled: false,
            running: false,
        }
    }

    /// Non-Unix platforms have no systemd services to restart.
    pub(crate) fn restart(_name: &str) -> Result<(), ProcessError> {
        Ok(())
    }

    /// Non-Unix platforms have no systemd services to restore.
    pub(crate) fn restore(_name: &str, _state: ServiceState) -> Result<(), ProcessError> {
        Ok(())
    }
}

/// The outcome of restarting the detected services.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RestartOutcome {
    /// The services that were restarted, in restart order.
    pub(crate) restarted: Vec<String>,
    /// Failures encountered while restarting or restoring services.
    pub(crate) failures: Vec<String>,
    /// Whether an interrupt was requested during the restart phase.
    pub(crate) interrupted: bool,
}

/// Restarts every service in `services`, capturing its previous enabled/running
/// state first. On interrupt, every service that was restarted is restored to
/// its previous state. A restart failure restores that service to its previous
/// state and is recorded as a failure without stopping the remaining services.
pub(crate) fn restart_detected_services(
    services: &[String],
    interrupt: fn() -> bool,
) -> RestartOutcome {
    let mut restarted: Vec<(String, ServiceState)> = Vec::new();
    let mut failures = Vec::new();
    let mut interrupted = false;

    for service in services {
        if interrupt() {
            interrupted = true;
            break;
        }
        let state = ServiceState::capture(service);
        match ServiceState::restart(service) {
            Ok(()) => restarted.push((service.clone(), state)),
            Err(error) => {
                // An interrupt leaves the signal flag set, which would make
                // every subsequent command return immediately. Clear it so the
                // restore commands can actually run, mirroring the runner's
                // restore phase. Remember the interrupt so the run still exits
                // with the interrupt status.
                crate::signals::reset();
                let _ = ServiceState::restore(service, state);
                failures.push(format!("{service}: {error}"));
                if matches!(error, ProcessError::Interrupted { .. }) {
                    interrupted = true;
                }
            }
        }
    }

    let interrupted = interrupted || interrupt();
    if interrupted {
        crate::signals::reset();
        for (service, state) in restarted.iter().rev() {
            if let Err(error) = ServiceState::restore(service, *state) {
                failures.push(format!("{service}: {error}"));
            }
        }
    }

    let restarted: Vec<String> = restarted.into_iter().map(|(name, _)| name).collect();
    RestartOutcome {
        restarted,
        failures,
        interrupted,
    }
}
