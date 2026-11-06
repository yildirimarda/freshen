//! PowerShell support.
//!
//! On Windows freshen runs PowerShell for some probes. PowerShell 7
//! (`pwsh`) is preferred when it is installed; otherwise Windows PowerShell
//! 5.1 (`powershell`) is used.
//!
//! The choice itself is platform-neutral so it is tested on every operating
//! system. Only the probe that asks the machine which PowerShells exist is
//! Windows-only, behind [`mod@self::windows`], together with its tests and
//! imports.

/// A PowerShell installation freshen can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerShell {
    /// PowerShell 7, invoked as `pwsh`.
    Pwsh,
    /// Windows PowerShell 5.1, invoked as `powershell`.
    Legacy,
}

impl PowerShell {
    /// The executable name a shell would run for this PowerShell.
    #[must_use]
    pub fn command(self) -> &'static str {
        match self {
            Self::Pwsh => "pwsh",
            Self::Legacy => "powershell",
        }
    }
}

/// Chooses the PowerShell to use.
///
/// PowerShell 7 (`pwsh`) is preferred when it exists; otherwise Windows
/// PowerShell 5.1 (`powershell`) is used. Returns `None` when neither is
/// available.
///
/// `pwsh_exists` and `powershell_exists` are injected so this decision is
/// platform-neutral and can be tested on every operating system.
#[must_use]
pub fn choose(pwsh_exists: bool, powershell_exists: bool) -> Option<PowerShell> {
    if pwsh_exists {
        return Some(PowerShell::Pwsh);
    }
    if powershell_exists {
        return Some(PowerShell::Legacy);
    }
    None
}

#[cfg(windows)]
pub mod windows {
    use super::{choose, PowerShell};
    use std::ffi::OsString;

    /// Probes whether `program` exists on `PATH`/`PATHEXT`.
    ///
    /// Separate from the real probe so tests can substitute a fake instead of
    /// reading the machine's `PATH`.
    fn exists(program: &str) -> bool {
        crate::executable::command_exists(program)
    }

    /// The PowerShell to use, probing `PATH`/`PATHEXT` through `probe`.
    #[must_use]
    pub fn powershell_with(probe: impl Fn(&str) -> bool) -> Option<PowerShell> {
        choose(probe("pwsh"), probe("powershell"))
    }

    /// The PowerShell to use on this machine, probing the real `PATH`.
    #[must_use]
    pub fn powershell() -> Option<PowerShell> {
        powershell_with(exists)
    }

    /// Arguments for invoking PowerShell for freshen's own probes and steps:
    /// `-NoProfile -ExecutionPolicy Bypass [-NonInteractive] -Command <script>`.
    ///
    /// `-ExecutionPolicy Bypass` is process-scoped: it applies only to this
    /// PowerShell invocation and never changes the machine execution policy.
    /// `-NoProfile` skips the user's profile so freshen's behaviour does not
    /// depend on it. When `non_interactive` is set, `-NonInteractive` is added
    /// so a prompt can never block freshen on its closed standard input.
    #[must_use]
    pub fn invocation_args(script: &str, non_interactive: bool) -> Vec<OsString> {
        let mut args = Vec::with_capacity(6);
        args.push(OsString::from("-NoProfile"));
        args.push(OsString::from("-ExecutionPolicy"));
        args.push(OsString::from("Bypass"));
        if non_interactive {
            args.push(OsString::from("-NonInteractive"));
        }
        args.push(OsString::from("-Command"));
        args.push(OsString::from(script));
        args
    }

    #[cfg(test)]
    mod tests {
        use super::{invocation_args, powershell_with, PowerShell};
        use std::ffi::OsStr;

        #[test]
        fn prefers_powershell_7_when_it_exists() {
            let probe = |program: &str| program == "pwsh";
            assert_eq!(powershell_with(probe), Some(PowerShell::Pwsh));
        }

        #[test]
        fn falls_back_to_windows_powershell_51_when_pwsh_is_absent() {
            let probe = |program: &str| program == "powershell";
            assert_eq!(powershell_with(probe), Some(PowerShell::Legacy));
        }

        #[test]
        fn returns_none_when_neither_power_shell_exists() {
            let probe = |_: &str| false;
            assert_eq!(powershell_with(probe), None);
        }

        #[test]
        fn chooses_powershell_7_when_both_exist() {
            let probe = |_: &str| true;
            assert_eq!(powershell_with(probe), Some(PowerShell::Pwsh));
        }

        #[test]
        fn invocation_args_uses_a_process_scoped_execution_policy() {
            let args = invocation_args("Get-Date", false);
            assert_eq!(
                args.as_slice(),
                [
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    "Get-Date",
                ]
                .map(OsStr::new)
                .as_slice()
            );
        }

        #[test]
        fn invocation_args_adds_non_interactive_before_the_command() {
            let args = invocation_args("Get-Date", true);
            assert_eq!(
                args.as_slice(),
                [
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-NonInteractive",
                    "-Command",
                    "Get-Date",
                ]
                .map(OsStr::new)
                .as_slice()
            );
        }

        #[test]
        fn invocation_args_never_changes_the_machine_policy() {
            let args = invocation_args("anything", false);
            // The generated invocation never carries a `Set-ExecutionPolicy`
            // command, and the only `-ExecutionPolicy` value is `Bypass`, which
            // applies to this process alone and never to the machine.
            assert!(!args
                .iter()
                .any(|arg| arg == OsStr::new("Set-ExecutionPolicy")));
            // Exactly one `-ExecutionPolicy` flag, set to `Bypass` for this
            // process alone and never to the machine.
            let policies: Vec<&OsStr> = args
                .iter()
                .zip(args.iter().skip(1))
                .filter_map(|(key, value)| {
                    if key == OsStr::new("-ExecutionPolicy") {
                        Some(value.as_os_str())
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(policies, vec![OsStr::new("Bypass")]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_powershell_7_when_both_exist() {
        assert_eq!(choose(true, true), Some(PowerShell::Pwsh));
    }

    #[test]
    fn chooses_powershell_7_alone() {
        assert_eq!(choose(true, false), Some(PowerShell::Pwsh));
    }

    #[test]
    fn falls_back_to_windows_powershell_51() {
        assert_eq!(choose(false, true), Some(PowerShell::Legacy));
    }

    #[test]
    fn returns_none_when_neither_exists() {
        assert_eq!(choose(false, false), None);
    }

    #[test]
    fn command_names() {
        assert_eq!(PowerShell::Pwsh.command(), "pwsh");
        assert_eq!(PowerShell::Legacy.command(), "powershell");
    }
}
