//! The macOS system updates step: `softwareupdate --list` and
//! `softwareupdate --install`.
//!
//! macOS updates its own system software with `softwareupdate`, the tool
//! behind "System Settings > General > Software Update". It lists the updates
//! that are available (`softwareupdate --list`) and installs them
//! (`softwareupdate --install`). Only the list works without administrator
//! rights, so the step can always report what is available.
//!
//! Installing a macOS update is the biggest change freshen can make: it takes
//! a long time, it needs administrator rights and the machine may have to
//! restart afterwards. The step therefore only reports the available updates,
//! unless `--install-macos-updates` asks for the installation.
//!
//! The step only runs on macOS, because `softwareupdate` exists only there.

use std::sync::{Arc, Mutex};

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Operation, OutdatedItem, Step, StepError, StepId};

/// The macOS system updates step.
pub const MACOS_UPDATES: StepId = "macos-updates";

const MACOS_PLATFORMS: [Platform; 1] = [Platform::Macos];

/// The reason shown for an update that the machine has to restart for.
const RESTART_REASON: &str = "needs a restart";

/// The reason shown for an update that the machine has to shut down for.
const SHUTDOWN_REASON: &str = "needs a shutdown";

/// The phrases that mean the machine has to restart to finish an update.
///
/// macOS marks an update that needs a restart with `Action: restart` in the
/// list it prints, and it says that the machine has to restart after an
/// update was installed.
const RESTART_PHRASES: [&str; 7] = [
    "action: restart",
    "[restart]",
    "restart computer",
    "restart required",
    "reboot required",
    "will restart",
    "needs a restart",
];

/// The phrases that mean the installation wanted a password it could not get.
///
/// `softwareupdate` needs administrator rights for everything but the list.
/// freshen never hands its terminal to a package manager, so the installation
/// runs through `sudo -n`: it uses cached credentials when there are any and
/// fails right away when a password would be needed.
const ADMINISTRATOR_PHRASES: [&str; 5] = [
    "sudo:",
    "requires admin",
    "administrator",
    "permission denied",
    "operation not permitted",
];

/// The phrase macOS prints when it asks whether it may install the updates.
///
/// freshen never hands its terminal to a package manager, so a question that
/// needs an answer ends the installation instead of waiting for one.
const CONFIRMATION_PHRASES: [&str; 1] = ["install all available software?"];

/// Registers the macOS system updates step, after every other step.
///
/// The macOS system updates are independent of the other tools, they are the
/// slowest update there is and the machine may have to restart afterwards, so
/// they run last: the rest of the machine is up to date even when the macOS
/// update needs the user's attention.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(MacosUpdatesStep::new())
}

/// The output of one `softwareupdate` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftwareUpdateOutput {
    success: bool,
    text: String,
}

impl SoftwareUpdateOutput {
    /// A command that finished successfully.
    #[must_use]
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            success: true,
            text: text.into(),
        }
    }

    /// A command that failed.
    #[must_use]
    pub fn failure(text: impl Into<String>) -> Self {
        Self {
            success: false,
            text: text.into(),
        }
    }

    /// Whether the command finished successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Everything the command printed, on stdout and on stderr.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// The `softwareupdate` commands the macOS system updates step needs.
///
/// The trait exists so the step can be tested against recorded transcripts
/// without a Mac and without the real `softwareupdate` binary.
pub trait SoftwareUpdateClient: Send + Sync {
    /// Whether the `softwareupdate` command is installed.
    fn installed(&self) -> bool;

    /// The output of `softwareupdate --list`, which lists the pending updates.
    fn list(&self) -> SoftwareUpdateOutput;

    /// The output of the installation of the pending updates.
    fn install(&self) -> SoftwareUpdateOutput;
}

/// The real `softwareupdate` command line.
struct SoftwareUpdateCommandClient;

impl SoftwareUpdateClient for SoftwareUpdateCommandClient {
    fn installed(&self) -> bool {
        crate::executable::command_exists("softwareupdate")
    }

    fn list(&self) -> SoftwareUpdateOutput {
        let command = CommandSpec::new("softwareupdate")
            .args(["--list"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES);

        run(command)
    }

    fn install(&self) -> SoftwareUpdateOutput {
        // `sudo -n` never asks for a password: the installation either uses
        // cached credentials or fails right away.
        let command = CommandSpec::new("sudo")
            .args(["-n", "softwareupdate", "--install", "--all"])
            .neutral_locale();

        run(command)
    }
}

/// Runs a `softwareupdate` command and collects everything it printed.
fn run(command: CommandSpec) -> SoftwareUpdateOutput {
    match command.run() {
        Ok(output) => SoftwareUpdateOutput::success(combined(output.stdout(), output.stderr())),
        Err(error) => {
            let printed = error
                .output()
                .map(|output| combined(output.stdout(), output.stderr()))
                .unwrap_or_default();
            if printed.trim().is_empty() {
                SoftwareUpdateOutput::failure(error.to_string())
            } else {
                SoftwareUpdateOutput::failure(printed)
            }
        }
    }
}

/// Joins what a command printed on stdout and on stderr.
fn combined(stdout: &str, stderr: &str) -> String {
    let mut text = stdout.to_owned();
    if !stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(stderr);
    }
    text
}

/// One update of the list `softwareupdate --list` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingUpdate {
    label: String,
    version: Option<String>,
    action: Option<String>,
}

/// The macOS system updates step.
struct MacosUpdatesStep {
    client: Arc<dyn SoftwareUpdateClient>,
    /// The output of the last list, so that a run which only reports the
    /// updates does not ask the update server a second time.
    last_list: Mutex<Option<String>>,
}

impl MacosUpdatesStep {
    fn new() -> Self {
        Self {
            client: Arc::new(SoftwareUpdateCommandClient),
            last_list: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn with_client(client: Arc<dyn SoftwareUpdateClient>) -> Self {
        Self {
            client,
            last_list: Mutex::new(None),
        }
    }

    /// The failure of a `softwareupdate` command that did not finish
    /// successfully.
    fn failure(&self, operation: Operation, output: &SoftwareUpdateOutput) -> StepError {
        let printed = output.text().trim();
        let message = if needs_administrator(printed) {
            "installing macOS updates needs administrator rights: run `sudo freshen --install-macos-updates` (or `sudo softwareupdate --install --all`) by hand"
                .to_owned()
        } else if operation == Operation::Check {
            format!("`softwareupdate --list` failed: {printed}")
        } else if asks_for_confirmation(printed) {
            "the macOS update installation asked for a confirmation: run `sudo softwareupdate --install --all` by hand"
                .to_owned()
        } else {
            format!("`softwareupdate --install` failed: {printed}")
        };
        StepError::new(MACOS_UPDATES, operation, message)
    }

    /// Lists the available updates and remembers what the tool printed.
    fn list(&self, operation: Operation) -> Result<String, StepError> {
        let output = self.client.list();
        if !output.is_success() {
            return Err(self.failure(operation, &output));
        }

        let text = output.text().to_owned();
        if let Ok(mut last) = self.last_list.lock() {
            *last = Some(text.clone());
        }
        Ok(text)
    }

    /// Reports the available updates without installing anything.
    ///
    /// The list of the run before the step is reused, so the update server is
    /// not asked for the same list twice.
    fn report(&self) -> Result<(), StepError> {
        let cached = self.last_list.lock().ok().and_then(|last| last.clone());
        let text = match cached {
            Some(text) => text,
            None => self.list(Operation::Run)?,
        };

        let items = parse_software_update_list(&text);
        human!(
            "macos-updates: {} update(s) available; nothing installed (use --install-macos-updates)",
            items.len()
        );
        for item in &items {
            human!("  - {item}");
        }
        if pending_reboot(&text) {
            human!("macos-updates: installing them will restart the machine");
        }
        Ok(())
    }
}

impl Step for MacosUpdatesStep {
    fn id(&self) -> StepId {
        MACOS_UPDATES
    }

    fn display_name(&self) -> &str {
        "macOS system updates"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        if self.client.installed() {
            Ok(Detection::Installed)
        } else {
            Ok(Detection::NotInstalled)
        }
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, _options: &RunOptions) -> Result<CheckReport, StepError> {
        let text = self.list(Operation::Check)?;
        Ok(CheckReport::from_items(parse_software_update_list(&text)))
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if !options.install_macos_updates() {
            return self.report();
        }

        human!("macos-updates: install (macOS system updates)");
        let output = self.client.install();
        if !output.is_success() {
            return Err(self.failure(Operation::Run, &output));
        }
        if pending_reboot(output.text()) {
            human!("macos-updates: the machine has to restart to finish the update");
        }
        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &MACOS_PLATFORMS
    }
}

/// Whether the output of a `softwareupdate` command says that the machine has
/// to restart to finish an update.
///
/// macOS marks an update that needs a restart with `Action: restart` in the
/// list it prints, and it says that the machine has to restart after an
/// update was installed. Both mean the same thing to the user: the machine is
/// not fully up to date until it restarts.
#[must_use]
pub fn pending_reboot(output: &str) -> bool {
    contains_any(output, &RESTART_PHRASES)
}

/// Whether the output of a `softwareupdate` command says that the
/// installation wanted a password it could not get.
fn needs_administrator(output: &str) -> bool {
    contains_any(output, &ADMINISTRATOR_PHRASES)
}

/// Whether the output of a `softwareupdate` command says that the tool asked
/// whether it may install the updates.
fn asks_for_confirmation(output: &str) -> bool {
    contains_any(output, &CONFIRMATION_PHRASES)
}

/// Whether `text` contains one of the phrases, ignoring the case.
fn contains_any(text: &str, phrases: &[&str]) -> bool {
    let text = text.to_lowercase();
    phrases.iter().any(|phrase| text.contains(phrase))
}

/// Parses the output of `softwareupdate --list` into the pending updates.
///
/// The tool prints one line per update, marked with a `*` for a recommended
/// update and a `-` for one that is not, and one indented line per update
/// with its details. Three formats are in the wild:
///
/// ```text
/// Software Update found the following new or updated software:
///    * Label: MacBookAirEFIUpdate2.4-2.4
///         Title: MacBook Air EFI Firmware Update, Version: 2.4, Size: 3817K, Recommended: YES, Action: restart,
///    * macOS Sonoma 14.5-23F79 (14.5)
///    * macOS 10.14.1 Update
///         macOS 10.14.1 Update (10.14.1), 199140K [recommended] [restart]
/// ```
#[must_use]
pub fn parse_software_update_list(output: &str) -> Vec<OutdatedItem> {
    parse_pending_updates(output)
        .into_iter()
        .map(|update| {
            let mut item = OutdatedItem::new(update.label);
            if let Some(version) = update.version {
                item = item.with_latest(version);
            }
            match update.action.as_deref() {
                Some("restart") => item.with_reason(RESTART_REASON),
                Some("shut down") => item.with_reason(SHUTDOWN_REASON),
                _ => item,
            }
        })
        .collect()
}

/// Parses the output of `softwareupdate --list` into the pending updates.
fn parse_pending_updates(output: &str) -> Vec<PendingUpdate> {
    let mut updates: Vec<PendingUpdate> = Vec::new();
    let mut expecting_details = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(label) = update_label(trimmed) {
            let (label, version) = split_version(label);
            updates.push(PendingUpdate {
                label,
                version,
                action: None,
            });
            expecting_details = true;
            continue;
        }

        if !expecting_details {
            continue;
        }
        expecting_details = false;

        let Some(update) = updates.last_mut() else {
            continue;
        };
        if update.version.is_none() {
            update.version = details_version(trimmed);
        }
        if update.action.is_none() {
            update.action = details_action(trimmed).map(str::to_owned);
        }
    }

    updates
}

/// The label of an update line, when the line names an update.
///
/// An update line starts with a `*` for a recommended update or a `-` for one
/// that is not recommended.
fn update_label(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("* ")
        .or_else(|| line.strip_prefix("- "))?;
    let rest = rest.strip_prefix("Label: ").unwrap_or(rest);
    let rest = rest.trim();
    (!rest.is_empty()).then_some(rest)
}

/// Splits the version off the end of an update label.
///
/// The newer formats put the version in parentheses (`macOS Sonoma 14.5
/// (14.5)`), the older ones leave it out and print it in the details line.
fn split_version(label: &str) -> (String, Option<String>) {
    let Some(open) = label.rfind('(') else {
        return (label.to_owned(), None);
    };
    let Some(version) = label[open + 1..].strip_suffix(')') else {
        return (label.to_owned(), None);
    };

    let version = version.trim();
    if !looks_like_version(version) {
        return (label.to_owned(), None);
    }
    (label[..open].trim().to_owned(), Some(version.to_owned()))
}

/// Whether the text looks like a version rather than a word in parentheses.
fn looks_like_version(text: &str) -> bool {
    !text.is_empty() && text.chars().any(|character| character.is_ascii_digit())
}

/// The version in the details line of an update.
///
/// The newer formats print `Version: 14.5,`, the older ones a parenthesized
/// version (`macOS 10.14.1 Update (10.14.1), 199140K`).
fn details_version(details: &str) -> Option<String> {
    if let Some((_, rest)) = details.split_once("Version: ") {
        let version = rest.split(',').next().unwrap_or(rest).trim();
        if looks_like_version(version) {
            return Some(version.to_owned());
        }
    }

    let open = details.rfind('(')?;
    let version = details[open + 1..].split(')').next()?.trim();
    looks_like_version(version).then(|| version.to_owned())
}

/// The action macOS asks for after an update.
///
/// The newer formats print `Action: restart,`, the older ones `[restart]`.
fn details_action(details: &str) -> Option<&'static str> {
    if let Some((_, rest)) = details.split_once("Action: ") {
        let action = rest.split(',').next().unwrap_or(rest).trim();
        match action {
            "restart" => return Some("restart"),
            "shut down" => return Some("shut down"),
            _ => {}
        }
    }

    if details.contains("[restart]") {
        return Some("restart");
    }
    if details.contains("[shut down]") {
        return Some("shut down");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        parse_software_update_list, pending_reboot, MacosUpdatesStep, SoftwareUpdateClient,
        SoftwareUpdateOutput, MACOS_UPDATES,
    };
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::registry::Registry;
    use crate::step::{Detection, Operation, Step};
    use std::sync::{Arc, Mutex};

    /// A `softwareupdate` that replays recorded transcripts.
    struct FakeSoftwareUpdate {
        list: SoftwareUpdateOutput,
        install: SoftwareUpdateOutput,
        calls: Mutex<Vec<String>>,
    }

    impl FakeSoftwareUpdate {
        fn new() -> Self {
            Self {
                list: SoftwareUpdateOutput::success(""),
                install: SoftwareUpdateOutput::success(""),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .map(|calls| calls.clone())
                .unwrap_or_default()
        }

        fn record(&self, call: &str) {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(call.to_owned());
            }
        }
    }

    impl SoftwareUpdateClient for FakeSoftwareUpdate {
        fn installed(&self) -> bool {
            true
        }

        fn list(&self) -> SoftwareUpdateOutput {
            self.record("list");
            self.list.clone()
        }

        fn install(&self) -> SoftwareUpdateOutput {
            self.record("install");
            self.install.clone()
        }
    }

    fn step_with(client: &Arc<FakeSoftwareUpdate>) -> MacosUpdatesStep {
        MacosUpdatesStep::with_client(client.clone())
    }

    #[test]
    fn parses_the_updates_of_the_current_format() {
        let items = parse_software_update_list(
            "Software Update Tool\n\
             \n\
             Finding available software\n\
             Software Update found the following new or updated software:\n\
             * Label: macOS Sonoma 14.5-23F79\n\
             \x20    Title: macOS Sonoma Update, Version: 14.5, Size: 4.7GB, Recommended: YES, Action: restart,\n\
             * Label: ProAppsQTCodecs-1.0\n\
             \x20    Title: ProApps QuickTime codecs, Version: 1.0, Size: 968K, Recommended: YES,\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "macOS Sonoma 14.5-23F79");
        assert_eq!(items[0].latest(), Some("14.5"));
        assert_eq!(items[0].reason(), Some("needs a restart"));
        assert_eq!(items[1].name(), "ProAppsQTCodecs-1.0");
        assert_eq!(items[1].latest(), Some("1.0"));
        assert_eq!(items[1].reason(), None);
    }

    #[test]
    fn parses_the_updates_of_the_format_with_the_version_in_the_label() {
        let items = parse_software_update_list(
            "Software Update found the following new or updated software:\n\
             \x20  * macOS Sonoma 14.5-23F79 (14.5)\n\
             \x20  * Command Line Tools for Xcode-14.3 (14.3)\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "macOS Sonoma 14.5-23F79");
        assert_eq!(items[0].latest(), Some("14.5"));
        assert_eq!(items[1].name(), "Command Line Tools for Xcode-14.3");
        assert_eq!(items[1].latest(), Some("14.3"));
    }

    #[test]
    fn parses_the_updates_of_the_old_format() {
        let items = parse_software_update_list(
            "Software Update found the following new or updated software:\n\
             \x20  * macOS 10.14.1 Update\n\
             \x20      macOS 10.14.1 Update (10.14.1), 199140K [recommended] [restart]\n\
             \x20  - iCal-1.0.2\n\
             \x20      iCal, (1.0.2), 6520K\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "macOS 10.14.1 Update");
        assert_eq!(items[0].latest(), Some("10.14.1"));
        assert_eq!(items[0].reason(), Some("needs a restart"));
        assert_eq!(items[1].name(), "iCal-1.0.2");
        assert_eq!(items[1].latest(), Some("1.0.2"));
    }

    #[test]
    fn an_update_that_needs_a_shutdown_is_reported_as_such() {
        let items = parse_software_update_list(
            "* Label: BridgeOSUpdateCustomer\n\
             \x20    Title: BridgeOSUpdateCustomer, Version: 10.15.0.1.1, Size: 390674K, Recommended: YES, Action: shut down,\n",
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].reason(), Some("needs a shutdown"));
    }

    #[test]
    fn a_list_without_updates_has_no_items() {
        assert!(parse_software_update_list("").is_empty());
        assert!(parse_software_update_list("No new software available.\n").is_empty());
        assert!(
            parse_software_update_list("Software Update Tool\n\nFinding available software\n")
                .is_empty()
        );
    }

    #[test]
    fn a_line_that_is_not_an_update_is_ignored() {
        let items = parse_software_update_list(
            "Software Update Tool\n\nFinding available software\nSoftware Update found the following new or updated software:\n",
        );

        assert!(items.is_empty());
    }

    #[test]
    fn recognises_a_pending_reboot() {
        assert!(pending_reboot(
            "Title: macOS Sonoma Update, Version: 14.5, Recommended: YES, Action: restart,"
        ));
        assert!(pending_reboot(
            "macOS 10.14.1 Update (10.14.1), 199140K [restart]"
        ));
        assert!(pending_reboot(
            "Restart computer to finish installing this update"
        ));
        assert!(pending_reboot("The system will restart in five minutes"));
        assert!(!pending_reboot("No new software available."));
        assert!(!pending_reboot("Installing macOS Sonoma Update\nDone.\n"));
        assert!(!pending_reboot(""));
    }

    #[test]
    fn the_step_reports_the_available_updates() {
        let client = Arc::new(FakeSoftwareUpdate {
            list: SoftwareUpdateOutput::success(
                "* Label: macOS Sonoma 14.5-23F79\n\
                 \x20    Title: macOS Sonoma Update, Version: 14.5, Recommended: YES, Action: restart,\n",
            ),
            ..FakeSoftwareUpdate::new()
        });
        let step = step_with(&client);

        let report = step
            .check()
            .unwrap_or_else(|error| panic!("check failed: {error}"));

        assert_eq!(report.pending(), 1);
        assert_eq!(report.items()[0].name(), "macOS Sonoma 14.5-23F79");
        assert_eq!(client.calls(), ["list"]);
    }

    #[test]
    fn the_step_installs_nothing_without_the_opt_in() {
        let client = Arc::new(FakeSoftwareUpdate::new());
        let step = step_with(&client);

        assert!(step.run().is_ok());
        assert_eq!(client.calls(), ["list"]);
    }

    #[test]
    fn the_report_reuses_the_list_of_the_check() {
        let client = Arc::new(FakeSoftwareUpdate::new());
        let step = step_with(&client);

        assert!(step.check().is_ok());
        assert!(step.run().is_ok());

        assert_eq!(client.calls(), ["list"]);
    }

    #[test]
    fn the_step_installs_the_updates_with_the_opt_in() {
        let client = Arc::new(FakeSoftwareUpdate::new());
        let step = step_with(&client);

        let options = RunOptions::new().with_install_macos_updates(true);

        assert!(step.run_with_options(&options).is_ok());
        assert_eq!(client.calls(), ["install"]);
    }

    #[test]
    fn a_failed_list_is_reported() {
        let client = Arc::new(FakeSoftwareUpdate {
            list: SoftwareUpdateOutput::failure("Error: the update server is unreachable\n"),
            ..FakeSoftwareUpdate::new()
        });
        let step = step_with(&client);

        let error = step
            .check()
            .err()
            .unwrap_or_else(|| panic!("the check should fail"));

        assert_eq!(error.step(), MACOS_UPDATES);
        assert_eq!(error.operation(), Operation::Check);
        assert!(
            error.message().contains("the update server is unreachable"),
            "message: {}",
            error.message()
        );
    }

    #[test]
    fn a_failed_installation_names_the_administrator_rights() {
        let client = Arc::new(FakeSoftwareUpdate {
            install: SoftwareUpdateOutput::failure("sudo: a password is required\n"),
            ..FakeSoftwareUpdate::new()
        });
        let step = step_with(&client);

        let options = RunOptions::new().with_install_macos_updates(true);

        let error = step
            .run_with_options(&options)
            .err()
            .unwrap_or_else(|| panic!("the installation should fail"));

        assert_eq!(error.operation(), Operation::Run);
        assert!(
            error.message().contains("needs administrator rights"),
            "message: {}",
            error.message()
        );
    }

    #[test]
    fn a_failed_installation_reports_the_output_of_the_tool() {
        let client = Arc::new(FakeSoftwareUpdate {
            install: SoftwareUpdateOutput::failure("Error: the update could not be installed\n"),
            ..FakeSoftwareUpdate::new()
        });
        let step = step_with(&client);

        let options = RunOptions::new().with_install_macos_updates(true);

        let error = step
            .run_with_options(&options)
            .err()
            .unwrap_or_else(|| panic!("the installation should fail"));

        assert!(
            error
                .message()
                .contains("the update could not be installed"),
            "message: {}",
            error.message()
        );
    }

    #[test]
    fn an_installation_that_asked_for_a_confirmation_is_reported() {
        let client = Arc::new(FakeSoftwareUpdate {
            install: SoftwareUpdateOutput::failure("Install all available software? (y/N)\n"),
            ..FakeSoftwareUpdate::new()
        });
        let step = step_with(&client);

        let options = RunOptions::new().with_install_macos_updates(true);

        let error = step
            .run_with_options(&options)
            .err()
            .unwrap_or_else(|| panic!("the installation should fail"));

        assert!(
            error.message().contains("asked for a confirmation"),
            "message: {}",
            error.message()
        );
    }

    #[test]
    fn the_step_only_runs_on_macos() {
        let registry = super::register(Registry::new());

        assert_eq!(registry.validate(), Ok(()));
        let step = registry
            .steps()
            .find(|step| step.id() == MACOS_UPDATES)
            .unwrap_or_else(|| panic!("{MACOS_UPDATES} is not registered"));
        assert_eq!(step.platforms(), [Platform::Macos]);
    }

    #[test]
    fn the_step_is_not_part_of_a_run_on_other_platforms() {
        let registry = super::register(Registry::new());

        for platform in [Platform::Linux, Platform::Windows] {
            let ids: Vec<&str> = registry
                .ordered_for(platform)
                .unwrap_or_else(|error| panic!("registry order failed: {error}"))
                .iter()
                .map(|step| step.id())
                .collect();
            assert!(!ids.contains(&MACOS_UPDATES), "{platform}");
        }
    }

    #[test]
    fn a_missing_tool_skips_the_step() {
        struct MissingSoftwareUpdate;

        impl SoftwareUpdateClient for MissingSoftwareUpdate {
            fn installed(&self) -> bool {
                false
            }

            fn list(&self) -> SoftwareUpdateOutput {
                SoftwareUpdateOutput::success("")
            }

            fn install(&self) -> SoftwareUpdateOutput {
                SoftwareUpdateOutput::success("")
            }
        }

        let step = MacosUpdatesStep::with_client(Arc::new(MissingSoftwareUpdate));

        let detection = step
            .detect()
            .unwrap_or_else(|error| panic!("detection failed: {error}"));

        assert_eq!(detection, Detection::NotInstalled);
    }
}
