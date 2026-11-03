//! `cmd.exe` shims as launching shells.
//!
//! Some Windows package managers install `.cmd`/`.bat` shim wrappers around
//! real executables. To run one as a shell, freshen launches `cmd.exe /c
//! <shim> <args>`. cmd.exe parses its command line with its own quoting rules,
//! so arguments that contain spaces or shell metacharacters must be quoted for
//! cmd.exe; Rust's default C-runtime quoting (a backslash before a quote) is
//! not understood by cmd.exe.
//!
//! [`quote`] and the shim-detection decision are platform-neutral so they are
//! tested on every operating system. Only the probe that confirms `cmd.exe`
//! exists on this machine is Windows-only, behind [`mod@self::windows`],
//! together with its tests and imports.

/// A `cmd.exe` launching shell used to run `.cmd`/`.bat` shims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CmdShell;

impl CmdShell {
    /// The executable name used to launch this shell.
    #[must_use]
    pub fn command(self) -> &'static str {
        "cmd"
    }
}

/// Characters that cmd.exe treats specially outside a double-quoted region and
/// therefore must be quoted when they appear in an argument.
const CMD_METACHARACTERS: &[char] = &[' ', '\t', '&', '|', '<', '>', '^', '"', '(', ')', '%', '!'];

/// Quotes `arg` so it survives `cmd.exe`'s own command-line parsing.
///
/// An argument that contains none of [`CMD_METACHARACTERS`] is returned
/// unchanged. Otherwise it is wrapped in double quotes and every internal
/// double quote is doubled (`"` -> `""`), which is how cmd.exe represents a
/// literal double quote inside a quoted argument. An empty argument becomes
/// `""`.
///
/// This is cmd.exe-specific quoting: the C runtime escapes a quote inside an
/// argument with a backslash, which cmd.exe does not understand, so this must
/// be used instead of Rust's default command-line quoting when building a
/// `cmd.exe /c <command>` line.
#[must_use]
pub fn quote(arg: &str) -> String {
    if arg.is_empty() {
        return String::from("\"\"");
    }
    if !arg.contains(CMD_METACHARACTERS) {
        return arg.to_string();
    }
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    for ch in arg.chars() {
        if ch == '"' {
            quoted.push_str("\"\"");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('"');
    quoted
}

/// Returns `true` when `command` is a `.cmd`/`.bat` shim that must be launched
/// through `cmd.exe /c` instead of being executed directly.
///
/// The comparison is case-insensitive because Windows file names are
/// case-insensitive.
#[must_use]
pub fn is_shim(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    command.ends_with(".cmd") || command.ends_with(".bat")
}

#[cfg(windows)]
pub mod windows {
    use super::{quote, CmdShell};

    /// Probes whether `program` exists on `PATH`/`PATHEXT`.
    ///
    /// Separate from the real probe so tests can substitute a fake instead of
    /// reading the machine's `PATH`.
    fn exists(program: &str) -> bool {
        crate::executable::command_exists(program)
    }

    /// The `cmd.exe` launching shell, probing `PATH`/`PATHEXT` through `probe`.
    #[must_use]
    pub fn cmd_with(probe: impl Fn(&str) -> bool) -> Option<CmdShell> {
        if probe("cmd") {
            return Some(CmdShell);
        }
        None
    }

    /// The `cmd.exe` launching shell on this machine, probing the real `PATH`.
    #[must_use]
    pub fn cmd() -> Option<CmdShell> {
        cmd_with(exists)
    }

    #[cfg(test)]
    mod tests {
        use super::{cmd_with, CmdShell};

        #[test]
        fn finds_cmd_exe_when_it_exists() {
            let probe = |program: &str| program == "cmd";
            assert_eq!(cmd_with(probe), Some(CmdShell));
        }

        #[test]
        fn returns_none_when_cmd_exe_is_absent() {
            let probe = |_: &str| false;
            assert_eq!(cmd_with(probe), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_name() {
        assert_eq!(CmdShell.command(), "cmd");
    }

    #[test]
    fn quote_leaves_a_plain_word_unquoted() {
        assert_eq!(quote("tool"), "tool");
    }

    #[test]
    fn quote_quotes_an_empty_string() {
        assert_eq!(quote(""), "\"\"");
    }

    #[test]
    fn quote_quotes_spaces() {
        assert_eq!(quote("my tool.cmd"), "\"my tool.cmd\"");
    }

    #[test]
    fn quote_quotes_each_metacharacter() {
        assert_eq!(quote("a&b"), "\"a&b\"");
        assert_eq!(quote("a|b"), "\"a|b\"");
        assert_eq!(quote("a<b"), "\"a<b\"");
        assert_eq!(quote("a>b"), "\"a>b\"");
        assert_eq!(quote("a^b"), "\"a^b\"");
        assert_eq!(quote("a(b)c"), "\"a(b)c\"");
        assert_eq!(quote("a%b"), "\"a%b\"");
        assert_eq!(quote("a!b"), "\"a!b\"");
        assert_eq!(quote("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote("a\tb"), "\"a\tb\"");
    }

    #[test]
    fn quote_does_not_double_a_word_without_metacharacters() {
        assert_eq!(quote("uv"), "uv");
        assert_eq!(quote("mise.exe"), "mise.exe");
    }

    #[test]
    fn is_shim_recognizes_cmd_and_bat_case_insensitively() {
        assert!(is_shim("tool.cmd"));
        assert!(is_shim("tool.bat"));
        assert!(is_shim("tool.CMD"));
        assert!(is_shim("tool.Bat"));
        assert!(!is_shim("tool"));
        assert!(!is_shim("tool.exe"));
        assert!(!is_shim("tool.ps1"));
        assert!(!is_shim("shim.cmd.bak"));
    }
}
