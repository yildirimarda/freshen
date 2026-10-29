//! The Mac App Store step: `mas outdated` and `mas upgrade`.
//!
//! `mas` (mas-cli) is the command-line interface of the Mac App Store. It
//! lists the App Store apps that have a pending update (`mas outdated`) and
//! installs them (`mas upgrade`).
//!
//! Installing an App Store app needs an Apple Account that is signed in to the
//! App Store. A machine without a signed-in account is not broken, there is
//! simply nothing to update, so the step is skipped instead of failing the
//! run. The sign-in state is read with `mas account`, the only `mas` command
//! that answers it; `mas` 5 and newer dropped that command, and then the
//! state stays unknown and the upgrade is attempted.
//!
//! `mas` 1.8.3 answered "Not signed in" on macOS 12 even when an account was
//! signed in (mas-cli issue 417). `mas` 1.8.4 and newer answer that the
//! command is not supported on that macOS version, which freshen reads as an
//! unknown state and not as a missing account.
//!
//! The step only runs on macOS, because the Mac App Store exists only there.

use std::sync::Arc;

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{
    CheckReport, Detection, Operation, OutdatedItem, SkipReason, Step, StepError, StepId,
};

/// The Mac App Store step.
pub const MAS: StepId = "mas";

const APP_STORE_PLATFORMS: [Platform; 1] = [Platform::Macos];

/// The phrases `mas` uses when no Apple Account is signed in.
///
/// `mas account` answers "Not signed in" (mas 1) and "Not signed in to an
/// Apple Account in the Mac App Store" (mas 2 to 4).
const NOT_SIGNED_IN_PHRASES: [&str; 4] = [
    "not signed in",
    "not signed-in",
    "please sign in",
    "must be signed in",
];

/// The phrases that show the upgrade wanted a password it could not get.
///
/// `mas` 5 and newer install App Store apps with administrator rights and
/// run themselves through `sudo`. freshen never hands its terminal to a
/// package manager, so the upgrade fails when no sudo credentials are
/// cached.
const ADMINISTRATOR_PHRASES: [&str; 3] = [
    "a terminal is required",
    "no askpass program specified",
    "sudo:",
];

/// Registers the Mac App Store step, after every other step.
///
/// The App Store apps are independent of the other tools, and their update is
/// the slowest one, so it runs last: the rest of the machine is up to date
/// even when the App Store upgrade needs the user's attention.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(MacAppStoreStep::new())
}

/// The output of one `mas` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasOutput {
    success: bool,
    text: String,
}

impl MasOutput {
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

/// The `mas` commands the Mac App Store step needs.
///
/// The trait exists so the step can be tested against recorded transcripts
/// without a Mac App Store and without the real `mas` binary.
pub trait MasClient: Send + Sync {
    /// Whether the `mas` command is installed.
    fn installed(&self) -> bool;

    /// The output of `mas account`, which names the signed-in account.
    ///
    /// The command answers "Not signed in" when no account is signed in, and
    /// does not exist at all in `mas` 5 and newer.
    fn account(&self) -> MasOutput;

    /// The output of `mas outdated`, which lists the pending updates.
    fn outdated(&self) -> MasOutput;

    /// The output of `mas upgrade`, which installs the pending updates.
    fn upgrade(&self) -> MasOutput;

    /// The output of `mas upgrade <id>`, which installs the selected updates.
    fn upgrade_ids(&self, ids: &[&str]) -> MasOutput;
}

/// The real `mas` command line.
struct MasCommandClient;

impl MasClient for MasCommandClient {
    fn installed(&self) -> bool {
        crate::executable::command_exists("mas")
    }

    fn account(&self) -> MasOutput {
        run_mas(&["account"])
    }

    fn outdated(&self) -> MasOutput {
        run_mas(&["outdated"])
    }

    fn upgrade(&self) -> MasOutput {
        run_mas(&["upgrade"])
    }

    fn upgrade_ids(&self, ids: &[&str]) -> MasOutput {
        let mut args = vec!["upgrade"];
        args.extend(ids);
        run_mas(&args)
    }
}

/// Runs a `mas` command and collects everything it printed.
fn run_mas(arguments: &[&str]) -> MasOutput {
    let command = CommandSpec::new("mas")
        .args(arguments)
        .neutral_locale()
        .retry_transient_network(NETWORK_RETRIES);

    match command.run() {
        Ok(output) => MasOutput::success(combined(output.stdout(), output.stderr())),
        Err(error) => {
            let printed = error
                .output()
                .map(|output| combined(output.stdout(), output.stderr()))
                .unwrap_or_default();
            if printed.trim().is_empty() {
                MasOutput::failure(error.to_string())
            } else {
                MasOutput::failure(printed)
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

/// Whether an Apple Account is signed in to the App Store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignInState {
    /// An account is signed in.
    SignedIn,
    /// No account is signed in.
    NotSignedIn,
    /// The installed `mas` cannot answer the question.
    Unknown,
}

/// The Mac App Store step.
struct MacAppStoreStep {
    client: Arc<dyn MasClient>,
}

impl MacAppStoreStep {
    fn new() -> Self {
        Self {
            client: Arc::new(MasCommandClient),
        }
    }

    #[cfg(test)]
    fn with_client(client: Arc<dyn MasClient>) -> Self {
        Self { client }
    }

    /// Whether an Apple Account is signed in to the App Store.
    fn sign_in_state(&self) -> SignInState {
        let output = self.client.account();
        if output.is_success() {
            return SignInState::SignedIn;
        }
        if reports_not_signed_in(output.text()) {
            return SignInState::NotSignedIn;
        }
        SignInState::Unknown
    }

    /// The failure of a `mas` command that did not finish successfully.
    fn failure(&self, operation: Operation, output: &MasOutput) -> StepError {
        let printed = output.text().trim();
        let message = if reports_not_signed_in(printed) {
            "not signed in to the Mac App Store: sign in with the App Store app, then run `mas upgrade` by hand".to_owned()
        } else if needs_administrator(printed) {
            "the Mac App Store upgrade needs administrator rights: run `sudo mas upgrade` by hand"
                .to_owned()
        } else {
            format!("`mas` failed: {printed}")
        };
        StepError::new(MAS, operation, message)
    }
}

impl Step for MacAppStoreStep {
    fn id(&self) -> StepId {
        MAS
    }

    fn display_name(&self) -> &str {
        "Mac App Store"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        if !self.client.installed() {
            return Ok(Detection::NotInstalled);
        }
        match self.sign_in_state() {
            SignInState::NotSignedIn => Ok(Detection::Unavailable(SkipReason::NotSignedIn)),
            SignInState::SignedIn | SignInState::Unknown => Ok(Detection::Installed),
        }
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        self.check_with_options(&RunOptions::new())
    }

    fn check_with_options(&self, options: &RunOptions) -> Result<CheckReport, StepError> {
        let output = self.client.outdated();
        if !output.is_success() {
            return Err(self.failure(Operation::Check, &output));
        }
        let filter = options.filters().appstore();
        let text = output.text();
        let filtered_text = if filter.is_empty() {
            text.to_owned()
        } else {
            text.lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('{') {
                        return true;
                    }
                    let id = trimmed.split_whitespace().next().unwrap_or("");
                    filter.includes(id)
                })
                .map(|line| line.to_owned())
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(CheckReport::from_items(parse_mas_outdated(&filtered_text)))
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        let filter = options.filters().appstore();
        if filter.is_empty() {
            human!("mas: upgrade (Mac App Store apps)");
            let output = self.client.upgrade();
            if !output.is_success() {
                return Err(self.failure(Operation::Run, &output));
            }
            return Ok(());
        }

        let outdated_output = self.client.outdated();
        if !outdated_output.is_success() {
            return Err(self.failure(Operation::Run, &outdated_output));
        }

        let ids: Vec<&str> = outdated_output
            .text()
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('{') {
                    return None;
                }
                let id = trimmed.split_whitespace().next()?;
                if filter.includes(id) {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();

        if ids.is_empty() {
            human!("mas: no selected apps to upgrade (Mac App Store apps)");
            return Ok(());
        }

        for id in ids {
            human!("mas: upgrade {} (Mac App Store app)", id);
            let output = self.client.upgrade_ids(&[id]);
            if !output.is_success() {
                return Err(self.failure(Operation::Run, &output));
            }
        }

        Ok(())
    }

    fn platforms(&self) -> &'static [Platform] {
        &APP_STORE_PLATFORMS
    }
}

/// Whether the output of a `mas` command says that no account is signed in.
fn reports_not_signed_in(text: &str) -> bool {
    contains_any(text, &NOT_SIGNED_IN_PHRASES)
}

/// Whether the output of a `mas` command says that it wanted a password.
fn needs_administrator(text: &str) -> bool {
    contains_any(text, &ADMINISTRATOR_PHRASES)
}

/// Whether `text` contains one of the phrases, ignoring the case.
fn contains_any(text: &str, phrases: &[&str]) -> bool {
    let text = text.to_lowercase();
    phrases.iter().any(|phrase| text.contains(phrase))
}

/// Parses the output of `mas outdated`.
///
/// Two formats are in the wild. `mas` 1 to 6 print one line per app as
/// `<id>  <name>  (<version> -> <new version>)`, with the columns padded to
/// the width of the longest entry. `mas` 7 prints one JSON object per app.
fn parse_mas_outdated(output: &str) -> Vec<OutdatedItem> {
    output.lines().filter_map(parse_mas_outdated_line).collect()
}

fn parse_mas_outdated_line(line: &str) -> Option<OutdatedItem> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if line.starts_with('{') {
        return parse_mas_outdated_json(line);
    }
    parse_mas_outdated_text(line)
}

/// Parses one `<id>  <name>  (<version> -> <new version>)` line.
fn parse_mas_outdated_text(line: &str) -> Option<OutdatedItem> {
    let open = line.rfind('(')?;
    let close = line.rfind(')')?;
    if close <= open {
        return None;
    }

    let (current, latest) = line[open + 1..close].split_once("->")?;

    // The head is the app id and the app name; the name is padded to the
    // width of the longest name, so it is trimmed on both ends.
    let head = line[..open].trim_end();
    let (_id, name) = head.split_once(char::is_whitespace)?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    Some(item(name, current.trim(), latest.trim()))
}

/// Parses one JSON object of the `mas` 7 output.
fn parse_mas_outdated_json(line: &str) -> Option<OutdatedItem> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let name = value.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }

    let current = value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let latest = value
        .get("newVersion")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();

    Some(item(name, current, latest))
}

/// Builds the outdated item of an app, leaving out the versions that are
/// unknown.
fn item(name: &str, current: &str, latest: &str) -> OutdatedItem {
    let mut item = OutdatedItem::new(name);
    if !current.is_empty() {
        item = item.with_current(current);
    }
    if !latest.is_empty() {
        item = item.with_latest(latest);
    }
    item
}

#[cfg(test)]
mod tests {
    use super::{
        parse_mas_outdated, reports_not_signed_in, MacAppStoreStep, MasClient, MasOutput, MAS,
    };
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::policy::{ItemFilters, NameFilter};
    use crate::registry::Registry;
    use crate::step::{Detection, SkipReason, Step};
    use std::sync::{Arc, Mutex};

    /// A `mas` that replays recorded transcripts.
    struct FakeMas {
        account: MasOutput,
        outdated: MasOutput,
        upgrade: MasOutput,
        calls: Mutex<Vec<String>>,
    }

    impl FakeMas {
        fn new() -> Self {
            Self {
                account: MasOutput::success("someone@example.com\n"),
                outdated: MasOutput::success(""),
                upgrade: MasOutput::success(""),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn signed_out() -> Self {
            Self {
                account: MasOutput::failure("Error: Not signed in\n"),
                ..Self::new()
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

    impl MasClient for FakeMas {
        fn installed(&self) -> bool {
            true
        }

        fn account(&self) -> MasOutput {
            self.record("account");
            self.account.clone()
        }

        fn outdated(&self) -> MasOutput {
            self.record("outdated");
            self.outdated.clone()
        }

        fn upgrade(&self) -> MasOutput {
            self.record("upgrade");
            self.upgrade.clone()
        }

        fn upgrade_ids(&self, ids: &[&str]) -> MasOutput {
            self.record(&format!("upgrade {}", ids.join(" ")));
            self.upgrade.clone()
        }
    }

    fn step_with(client: &Arc<FakeMas>) -> MacAppStoreStep {
        MacAppStoreStep::with_client(client.clone())
    }

    #[test]
    fn parses_the_outdated_apps_of_mas_one_to_six() {
        let items = parse_mas_outdated(
            "409203825  Numbers  (4.2 -> 4.3.1)\n\
             409183694  Keynote  (7.2 -> 7.3.1)\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "Numbers");
        assert_eq!(items[0].current(), Some("4.2"));
        assert_eq!(items[0].latest(), Some("4.3.1"));
        assert_eq!(items[1].name(), "Keynote");
    }

    #[test]
    fn parses_the_outdated_apps_of_mas_one_without_padding() {
        let items = parse_mas_outdated("409203825 Numbers (4.2 -> 4.3.1)\n");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "Numbers");
        assert_eq!(items[0].current(), Some("4.2"));
        assert_eq!(items[0].latest(), Some("4.3.1"));
    }

    #[test]
    fn parses_the_outdated_apps_of_mas_seven() {
        let items = parse_mas_outdated(
            "{\"bundleID\":\"com.apple.iWork.Numbers\",\"name\":\"Numbers\",\"newVersion\":\"4.3.1\",\"version\":\"4.2\"}\n",
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "Numbers");
        assert_eq!(items[0].current(), Some("4.2"));
        assert_eq!(items[0].latest(), Some("4.3.1"));
    }

    #[test]
    fn an_app_name_with_parentheses_is_parsed() {
        let items = parse_mas_outdated("409203825  Numbers (Beta)  (4.2 -> 4.3.1)\n");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "Numbers (Beta)");
        assert_eq!(items[0].current(), Some("4.2"));
        assert_eq!(items[0].latest(), Some("4.3.1"));
    }

    #[test]
    fn ignores_output_without_outdated_apps() {
        assert!(parse_mas_outdated("").is_empty());
        assert!(parse_mas_outdated("Everything is up-to-date.\n").is_empty());
        assert!(parse_mas_outdated("not json {\n").is_empty());
    }

    #[test]
    fn recognises_the_answer_of_a_machine_without_a_signed_in_account() {
        assert!(reports_not_signed_in("Not signed in"));
        assert!(reports_not_signed_in(
            "Error: Not signed in to an Apple Account in the Mac App Store"
        ));
        assert!(reports_not_signed_in("Please sign in to the App Store"));
        assert!(!reports_not_signed_in(
            "This command is not supported on this macOS version due to changes in macOS"
        ));
        assert!(!reports_not_signed_in(""));
    }

    #[test]
    fn the_step_reports_the_outdated_apps() {
        let client = Arc::new(FakeMas {
            outdated: MasOutput::success("409203825  Numbers  (4.2 -> 4.3.1)\n"),
            ..FakeMas::new()
        });
        let step = step_with(&client);

        let report = step
            .check()
            .unwrap_or_else(|error| panic!("check failed: {error}"));

        assert_eq!(report.pending(), 1);
        assert_eq!(report.items()[0].name(), "Numbers");
        assert_eq!(client.calls(), ["outdated"]);
    }

    #[test]
    fn the_step_upgrades_the_apps() {
        let client = Arc::new(FakeMas::new());
        let step = step_with(&client);

        assert!(step.run().is_ok());
        assert_eq!(client.calls(), ["upgrade"]);
    }

    #[test]
    fn a_machine_without_a_signed_in_account_is_skipped() {
        let client = Arc::new(FakeMas::signed_out());
        let step = step_with(&client);

        let detection = step
            .detect()
            .unwrap_or_else(|error| panic!("detection failed: {error}"));

        assert_eq!(detection, Detection::Unavailable(SkipReason::NotSignedIn));
    }

    #[test]
    fn a_machine_with_a_signed_in_account_is_detected() {
        let client = Arc::new(FakeMas::new());
        let step = step_with(&client);

        let detection = step
            .detect()
            .unwrap_or_else(|error| panic!("detection failed: {error}"));

        assert_eq!(detection, Detection::Installed);
    }

    #[test]
    fn a_machine_whose_mas_cannot_answer_the_sign_in_question_is_detected() {
        let client = Arc::new(FakeMas {
            account: MasOutput::failure("Error: Unknown subcommand 'account'\n"),
            ..FakeMas::new()
        });
        let step = step_with(&client);

        let detection = step
            .detect()
            .unwrap_or_else(|error| panic!("detection failed: {error}"));

        assert_eq!(detection, Detection::Installed);
    }

    #[test]
    fn a_failed_upgrade_names_the_missing_account() {
        let client = Arc::new(FakeMas {
            upgrade: MasOutput::failure("Error: Not signed in\n"),
            ..FakeMas::new()
        });
        let step = step_with(&client);

        let error = step
            .run()
            .err()
            .unwrap_or_else(|| panic!("the upgrade should fail"));

        assert_eq!(error.step(), MAS);
        assert!(
            error
                .message()
                .contains("not signed in to the Mac App Store"),
            "message: {}",
            error.message()
        );
    }

    #[test]
    fn a_failed_check_reports_the_output_of_mas() {
        let client = Arc::new(FakeMas {
            outdated: MasOutput::failure("Error: the App Store is unreachable\n"),
            ..FakeMas::new()
        });
        let step = step_with(&client);

        let error = step
            .check()
            .err()
            .unwrap_or_else(|| panic!("the check should fail"));

        assert!(
            error.message().contains("the App Store is unreachable"),
            "message: {}",
            error.message()
        );
    }

    #[test]
    fn an_upgrade_that_wanted_a_password_names_the_administrator_rights() {
        let client = Arc::new(FakeMas {
            upgrade: MasOutput::failure("sudo: a terminal is required to read the password\n"),
            ..FakeMas::new()
        });
        let step = step_with(&client);

        let error = step
            .run()
            .err()
            .unwrap_or_else(|| panic!("the upgrade should fail"));

        assert!(
            error.message().contains("needs administrator rights"),
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
            .find(|step| step.id() == MAS)
            .unwrap_or_else(|| panic!("{MAS} is not registered"));
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
            assert!(!ids.contains(&MAS), "{platform}");
        }
    }

    #[test]
    fn the_check_applies_the_include_list_to_the_app_store_apps() {
        let client = Arc::new(FakeMas {
            outdated: MasOutput::success(
                "409203825  Numbers  (4.2 -> 4.3.1)\n409183694  Keynote  (7.2 -> 7.3.1)\n",
            ),
            ..FakeMas::new()
        });
        let step = step_with(&client);

        let report = step
            .check_with_options(&RunOptions::new().with_filters(
                ItemFilters::new().with_appstore(NameFilter::new().with_include(["409203825"])),
            ))
            .unwrap_or_else(|error| panic!("check failed: {error}"));

        assert_eq!(report.pending(), 1);
        assert_eq!(report.items()[0].name(), "Numbers");
    }

    #[test]
    fn the_check_applies_the_exclude_list_to_the_app_store_apps() {
        let client = Arc::new(FakeMas {
            outdated: MasOutput::success(
                "409203825  Numbers  (4.2 -> 4.3.1)\n409183694  Keynote  (7.2 -> 7.3.1)\n",
            ),
            ..FakeMas::new()
        });
        let step = step_with(&client);

        let report = step
            .check_with_options(&RunOptions::new().with_filters(
                ItemFilters::new().with_appstore(NameFilter::new().with_exclude(["409203825"])),
            ))
            .unwrap_or_else(|error| panic!("check failed: {error}"));

        assert_eq!(report.pending(), 1);
        assert_eq!(report.items()[0].name(), "Keynote");
    }

    #[test]
    fn the_run_upgrades_only_the_selected_apps_by_their_app_store_id() {
        let client = Arc::new(FakeMas {
            outdated: MasOutput::success("409203825  Numbers  (4.2 -> 4.3.1)\n"),
            upgrade: MasOutput::success(""),
            ..FakeMas::new()
        });
        let step = step_with(&client);

        assert!(step
            .run_with_options(&RunOptions::new().with_filters(
                ItemFilters::new().with_appstore(NameFilter::new().with_include(["409203825"])),
            ))
            .is_ok());

        assert_eq!(client.calls(), ["outdated", "upgrade 409203825"]);
    }

    #[test]
    fn the_run_skips_the_upgrade_when_no_app_matches_the_filter() {
        let client = Arc::new(FakeMas {
            outdated: MasOutput::success("409203825  Numbers  (4.2 -> 4.3.1)\n"),
            upgrade: MasOutput::success(""),
            ..FakeMas::new()
        });
        let step = step_with(&client);

        assert!(step
            .run_with_options(&RunOptions::new().with_filters(
                ItemFilters::new().with_appstore(NameFilter::new().with_exclude(["409203825"])),
            ))
            .is_ok());

        assert_eq!(client.calls(), ["outdated"]);
    }
}
