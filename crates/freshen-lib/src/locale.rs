//! A neutral locale for the commands whose output freshen parses.
//!
//! Package managers and system tools translate their messages, and the labels
//! of their tables are translated as well. A machine set to Turkish prints
//! `İstenen` where an English machine prints `Requested`, and every word
//! freshen looks for (`Would remove`, `discharging`, `yes`) is gone. The
//! parsers then read the wrong columns, or nothing at all.
//!
//! freshen therefore runs every command whose output it parses with a neutral
//! locale:
//!
//! * on Unix and macOS `LC_ALL`, `LANG` and `LANGUAGE` are set to `C`, which
//!   keeps the messages English and the labels stable;
//! * on Windows the console is switched to UTF-8 (code page 65001) so that
//!   characters outside ASCII survive, and the tools that understand
//!   `--disable-interactivity` get that flag, so a tool never waits for an
//!   answer on a pipe.
//!
//! The neutral locale must not cost a tool its non-ASCII output, so the
//! encoding of the runtimes freshen starts (Python, Ruby) is pinned to UTF-8.
//!
//! The commands whose output is only shown to the user keep the locale of the
//! machine, so a German user still reads German messages.

use std::ffi::{OsStr, OsString};
use std::process::Command;

/// The locale every parsed command runs in.
pub const NEUTRAL_LOCALE: &str = "C";

/// The Windows code page for UTF-8.
#[cfg(windows)]
const UTF8_CODE_PAGE: u32 = 65001;

/// Variables that pin a command to the neutral locale.
///
/// `LC_ALL` decides the locale of the tool, `LANG` is the fallback for tools
/// that ignore `LC_ALL`, and `LANGUAGE` keeps a translated message catalogue
/// from overriding the two.
const LOCALE_VARIABLES: [(&str, &str); 3] = [
    ("LC_ALL", NEUTRAL_LOCALE),
    ("LANG", NEUTRAL_LOCALE),
    ("LANGUAGE", NEUTRAL_LOCALE),
];

/// Variables that make a tool write UTF-8.
///
/// With the neutral locale a tool that follows the locale of the machine
/// would write ASCII, and a name with an accent would break the parser.
/// These variables keep the output UTF-8.
const UTF8_VARIABLES: [(&str, &str); 2] = [("PYTHONUTF8", "1"), ("PYTHONIOENCODING", "utf-8")];

/// The encoding a tool written in Ruby is told to use.
///
/// Ruby takes its default encoding from the locale, and the `C` locale means
/// US-ASCII. A tool written in Ruby (Homebrew) would then fail to print a
/// name with an accent, so the encoding is pinned to UTF-8.
const RUBY_ENCODING: &str = "-EUTF-8:UTF-8";

/// Tools that understand a global flag that turns every prompt off, and the
/// flag itself.
///
/// A tool that waits for an answer never gets one: freshen gives every child
/// a closed standard input, so a prompt would only block the step until it
/// times out.
const NON_INTERACTIVE_TOOLS: [(&str, &str); 1] = [("winget", "--disable-interactivity")];

/// The environment a parsed command runs with.
#[must_use]
pub fn environment() -> Vec<(OsString, OsString)> {
    LOCALE_VARIABLES
        .iter()
        .chain(UTF8_VARIABLES.iter())
        .map(|(key, value)| (OsString::from(*key), OsString::from(*value)))
        .collect()
}

/// Applies the neutral locale to `command`.
///
/// The variables are set on the child, so they win over the ones freshen
/// inherited from the shell the user started it from. On Windows the console
/// is switched to UTF-8 as well, because the code page of the console decides
/// how a tool encodes the text it writes.
pub fn with_neutral_locale(command: &mut Command) {
    use_utf8_console();
    for (key, value) in environment() {
        command.env(key, value);
    }
    command.env("RUBYOPT", ruby_options());
}

/// The `RUBYOPT` value a parsed command runs with.
///
/// The options of the user are kept, and the encoding is pinned to UTF-8 so
/// that the neutral locale cannot cost a tool its non-ASCII output.
fn ruby_options() -> OsString {
    let mut options = std::env::var("RUBYOPT").unwrap_or_default();
    if !options.is_empty() {
        options.push(' ');
    }
    options.push_str(RUBY_ENCODING);

    options.into()
}

/// The flag that turns every prompt off for `program`, when it knows one.
///
/// The name is matched without its directory and without the `.exe` suffix,
/// because a tool can be started through its full path.
#[must_use]
pub fn non_interactive_flag(program: &OsStr) -> Option<&'static str> {
    let program = program.to_string_lossy();
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program.as_ref());
    let name = name.strip_suffix(".exe").unwrap_or(name);

    NON_INTERACTIVE_TOOLS
        .iter()
        .find(|(tool, _)| tool.eq_ignore_ascii_case(name))
        .map(|(_, flag)| *flag)
}

/// Switches the console to UTF-8 so that every tool freshen runs can print
/// characters outside ASCII.
///
/// On Windows the code page of the console decides how a program encodes the
/// text it writes, and the default code page mangles accented characters.
/// The other platforms already use UTF-8, so this does nothing there.
pub fn use_utf8_console() {
    #[cfg(windows)]
    set_utf8_code_page();
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn set_utf8_code_page() {
    // SAFETY: both calls take a code page, touch no memory and cannot fail in
    // a way that leaves anything behind; they only report whether the console
    // had a code page to change.
    unsafe {
        let _ = windows_sys::Win32::System::Console::SetConsoleOutputCP(UTF8_CODE_PAGE);
        let _ = windows_sys::Win32::System::Console::SetConsoleCP(UTF8_CODE_PAGE);
    }
}

#[cfg(test)]
mod tests {
    use super::{environment, non_interactive_flag, NEUTRAL_LOCALE, NON_INTERACTIVE_TOOLS};
    use std::ffi::OsStr;

    fn variables() -> Vec<(String, String)> {
        environment()
            .into_iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn the_neutral_locale_pins_the_language_variables() {
        for name in ["LC_ALL", "LANG", "LANGUAGE"] {
            let value = variables()
                .into_iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value);
            assert_eq!(value.as_deref(), Some(NEUTRAL_LOCALE), "{name}");
        }
    }

    #[test]
    fn a_turkish_environment_is_overridden() {
        let mut command = std::process::Command::new("true");
        command.env("LC_ALL", "tr_TR.UTF-8");
        super::with_neutral_locale(&mut command);

        let variables = command
            .get_envs()
            .filter_map(|(key, value)| {
                Some((
                    key.to_string_lossy().into_owned(),
                    value?.to_string_lossy().into_owned(),
                ))
            })
            .collect::<Vec<_>>();

        let locale = variables
            .iter()
            .find(|(key, _)| key == "LC_ALL")
            .map(|(_, value)| value.as_str());
        assert_eq!(locale, Some(NEUTRAL_LOCALE));
    }

    #[test]
    fn the_ruby_encoding_is_pinned_to_utf8() {
        let options = super::ruby_options();

        assert!(options.to_string_lossy().ends_with(super::RUBY_ENCODING));
    }

    #[test]
    fn only_the_tools_that_know_the_flag_get_it() {
        for (tool, flag) in NON_INTERACTIVE_TOOLS {
            assert_eq!(non_interactive_flag(OsStr::new(tool)), Some(flag));
            assert_eq!(
                non_interactive_flag(OsStr::new(&format!("{tool}.exe"))),
                Some(flag)
            );
            assert_eq!(
                non_interactive_flag(OsStr::new(&format!("C:/tools/{tool}.exe"))),
                Some(flag)
            );
        }

        assert_eq!(non_interactive_flag(OsStr::new("brew")), None);
        assert_eq!(non_interactive_flag(OsStr::new("docker")), None);
        assert_eq!(non_interactive_flag(OsStr::new("mise")), None);
    }
}
