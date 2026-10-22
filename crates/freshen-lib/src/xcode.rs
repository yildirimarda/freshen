//! The Xcode Command Line Tools on macOS.
//!
//! macOS ships the Command Line Tools (the compiler, the linker and the rest
//! of the tools a developer needs) and updates them with `softwareupdate`,
//! the same tool that lists the macOS system updates. The listing names them
//! `Command Line Tools for Xcode`.
//!
//! Homebrew needs an up-to-date version of them. Right after a macOS update
//! the installed ones are too old, and Homebrew then refuses to work:
//!
//! ```text
//! Error: Your Command Line Tools are too outdated.
//! Update them from Software Update in the App Store.
//! ```
//!
//! This module knows how the tools are named, how they are installed and how
//! the tools that need them complain about them.

/// The label `softwareupdate` uses for the Command Line Tools.
pub const CLT_LABEL: &str = "Command Line Tools for Xcode";

/// The reason shown for the Command Line Tools in a check report.
pub const CLT_REASON: &str = "Homebrew needs an up-to-date version";

/// The command that installs the Command Line Tools by hand.
///
/// The tools are installed with administrator rights, so the command needs
/// them.
#[must_use]
pub fn install_command() -> String {
    format!("sudo softwareupdate --install \"{CLT_LABEL}\"")
}

/// The phrases that mean that the Command Line Tools are too old.
///
/// Homebrew reports the problem in more than one way: as the error of the
/// command that needs the tools ("Error: Your Command Line Tools are too
/// outdated.") and as a finding of `brew doctor` ("A newer Command Line
/// Tools release is available."). The error of `xcrun` about the active
/// developer path is the other symptom of the same problem.
const OUTDATED_PHRASES: [&str; 5] = [
    "command line tools are too outdated",
    "command line tools are outdated",
    "command line tools are out of date",
    "newer command line tools release is available",
    "invalid active developer path",
];

/// Whether a name from the listing of `softwareupdate --list` names the
/// Command Line Tools.
///
/// The listing prints the name with the version in it
/// (`Command Line Tools for Xcode-16.4`) or behind it
/// (`Command Line Tools for Xcode 16.4`), so the name is matched at its
/// start.
#[must_use]
pub fn is_command_line_tools(name: &str) -> bool {
    name.starts_with(CLT_LABEL)
}

/// Whether the listing of `softwareupdate --list` names the Command Line
/// Tools.
#[must_use]
pub fn lists_command_line_tools(output: &str) -> bool {
    output.lines().any(|line| {
        let line = line.trim();
        let name = line
            .strip_prefix("* ")
            .or_else(|| line.strip_prefix("- "))
            .unwrap_or(line);
        let name = name.strip_prefix("Label: ").unwrap_or(name).trim();
        is_command_line_tools(name)
    })
}

/// Whether the output of a tool says that the Command Line Tools are too
/// old.
#[must_use]
pub fn is_outdated(output: &str) -> bool {
    let text = output.to_lowercase();
    if OUTDATED_PHRASES.iter().any(|phrase| text.contains(phrase)) {
        return true;
    }

    // Homebrew reports an outdated Xcode with the version in the sentence:
    // "Your Xcode (15.2) is outdated."
    text.contains("your xcode") && text.contains("is outdated")
}

/// The message that explains the problem and how to fix it.
#[must_use]
pub fn outdated_message() -> String {
    format!(
        "the Xcode Command Line Tools are out of date, which Homebrew needs after a macOS update: \
         install them with `{}` (or with `xcode-select --install`); \
         freshen installs them too when `softwareupdate` lists them, \
         so run freshen again to upgrade the packages",
        install_command()
    )
}

/// The message that says that the installation needs administrator rights.
#[must_use]
pub fn administrator_message() -> String {
    format!(
        "installing the Xcode Command Line Tools needs administrator rights: run `{}` by hand",
        install_command()
    )
}

#[cfg(test)]
mod tests {
    use super::{
        administrator_message, install_command, is_command_line_tools, is_outdated,
        lists_command_line_tools, outdated_message, CLT_LABEL,
    };

    #[test]
    fn recognises_the_command_line_tools_in_a_listing() {
        let listing = "Software Update found the following new or updated software:\n\
             * Label: Command Line Tools for Xcode-16.4\n\
             \x20   Title: Command Line Tools for Xcode, Version: 16.4, Size: 1.1GB, Recommended: YES,\n";
        assert!(lists_command_line_tools(listing));
    }

    #[test]
    fn recognises_the_command_line_tools_in_a_listing_with_the_version_in_the_name() {
        let listing = "Software Update found the following new or updated software:\n\
             \x20 * Command Line Tools for Xcode 16.4 (16.4)\n\
             \x20 * macOS Sonoma 14.5-23F79 (14.5)\n";
        assert!(lists_command_line_tools(listing));
    }

    #[test]
    fn a_listing_without_the_command_line_tools_does_not_name_them() {
        let listing = "Software Update found the following new or updated software:\n\
             * Label: macOS Sonoma 14.5-23F79\n\
             \x20   Title: macOS Sonoma Update, Version: 14.5, Size: 4.7GB, Recommended: YES, Action: restart,\n";
        assert!(!lists_command_line_tools(listing));
        assert!(!lists_command_line_tools(""));
        assert!(!lists_command_line_tools("No new software available.\n"));
    }

    #[test]
    fn recognises_the_name_of_the_command_line_tools() {
        assert!(is_command_line_tools("Command Line Tools for Xcode-16.4"));
        assert!(is_command_line_tools("Command Line Tools for Xcode 16.4"));
        assert!(!is_command_line_tools("macOS Sonoma 14.5-23F79"));
    }

    #[test]
    fn recognises_the_homebrew_error_about_the_outdated_command_line_tools() {
        assert!(is_outdated(
            "Error: Your Command Line Tools are too outdated. \
             Update them from Software Update in the App Store."
        ));
        assert!(is_outdated(
            "Error: A newer Command Line Tools release is available."
        ));
        assert!(is_outdated(
            "Error: Your Command Line Tools are out of date."
        ));
        assert!(is_outdated(
            "xcrun: error: invalid active developer path (/Library/Developer/CommandLineTools)"
        ));
        assert!(is_outdated("Your Xcode (15.2) is outdated."));
    }

    #[test]
    fn other_failures_are_not_mistaken_for_the_outdated_command_line_tools() {
        assert!(!is_outdated(
            "Error: No available formula with the name \"jq\"."
        ));
        assert!(!is_outdated("Error: Could not update Homebrew."));
        assert!(!is_outdated(""));
    }

    #[test]
    fn the_message_names_the_command_that_installs_the_tools() {
        let message = outdated_message();
        assert!(message.contains("Command Line Tools"), "message: {message}");
        assert!(message.contains(&install_command()), "message: {message}");
        assert!(message.contains("out of date"), "message: {message}");
    }

    #[test]
    fn the_administrator_message_names_the_command_that_installs_the_tools() {
        let message = administrator_message();
        assert!(
            message.contains("administrator rights"),
            "message: {message}"
        );
        assert!(message.contains(&install_command()), "message: {message}");
    }

    #[test]
    fn the_install_command_installs_the_label_of_the_tools() {
        assert!(install_command().contains(CLT_LABEL));
    }
}
