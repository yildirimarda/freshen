//! Recorded-transcript fixtures for the Homebrew JSON parsers.
//!
//! Each fixture is a recording of `brew info --cask --json=v2` from a
//! specific Homebrew version, stored under
//! `tests/fixtures/brew/<version>/brew-info-cask-json-v2.txt`. The parsers are
//! exercised against every recorded version so output drift between Homebrew
//! releases is caught by the recorded-transcript harness instead of inline
//! strings.

#![cfg(feature = "test-harness")]

mod support;

use freshen_lib::homebrew::{cask_app_artifacts, cask_metadata};
use support::RecordedTranscripts;

/// `brew info --cask --json=v2` recordings, one per Homebrew version.
const VERSIONS: &[(&str, &str)] = &[
    ("4.0.0", "brew-info-cask-json-v2"),
    ("4.1.0", "brew-info-cask-json-v2"),
    ("4.2.0", "brew-info-cask-json-v2"),
    ("4.3.0", "brew-info-cask-json-v2"),
];

fn transcript(version: &str) -> String {
    let transcripts = RecordedTranscripts::default();
    transcripts.stdout("brew", version, "brew-info-cask-json-v2")
}

#[test]
fn each_recorded_brew_version_parses_without_drift() {
    for (version, _) in VERSIONS {
        let json = transcript(version);
        cask_metadata(&json).unwrap_or_else(|error| {
            panic!("brew {version} `brew info --cask --json=v2` should parse: {error}")
        });
    }
}

#[test]
fn brew_4_0_0_reports_the_app_artifacts() {
    let metadata = cask_metadata(&transcript("4.0.0"))
        .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

    let casks: Vec<_> = metadata
        .casks()
        .iter()
        .map(|cask| cask.token().unwrap_or(""))
        .collect();
    assert_eq!(casks, ["firefox", "google-chrome", "font-hack"]);

    assert_eq!(metadata.casks()[0].apps(), ["Firefox.app"]);
    assert_eq!(metadata.casks()[1].apps(), ["Google Chrome.app"]);
    assert!(metadata.casks()[2].apps().is_empty());

    assert_eq!(metadata.apps(), ["Firefox.app", "Google Chrome.app"]);
    assert_eq!(
        cask_app_artifacts(&transcript("4.0.0"))
            .unwrap_or_else(|error| panic!("transcript should parse: {error}")),
        ["Firefox.app", "Google Chrome.app"]
    );

    assert!(!metadata.needs_administrator("firefox"));
    assert!(!metadata.needs_greedy("firefox"));
    assert_eq!(metadata.greedy_reason("firefox"), None);
}

#[test]
fn brew_4_1_0_parses_version_and_auto_updates() {
    let metadata = cask_metadata(&transcript("4.1.0"))
        .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

    let firefox = &metadata.casks()[0];
    assert_eq!(firefox.token(), Some("firefox"));
    assert_eq!(firefox.version(), Some("134.0"));
    assert!(!firefox.auto_updates());
    assert!(!metadata.needs_greedy("firefox"));
    assert_eq!(metadata.greedy_reason("firefox"), None);

    let spotify = &metadata.casks()[1];
    assert_eq!(spotify.version(), Some("1.2.60"));
    assert!(spotify.auto_updates());
    assert!(metadata.needs_greedy("spotify"));
    assert_eq!(
        metadata.greedy_reason("spotify").as_deref(),
        Some("auto_updates true; needs --greedy")
    );

    let nightly = &metadata.casks()[2];
    assert_eq!(nightly.version(), Some("latest"));
    assert!(!nightly.auto_updates());
    assert!(metadata.needs_greedy("nightly"));
    assert_eq!(
        metadata.greedy_reason("nightly").as_deref(),
        Some("version :latest; needs --greedy")
    );

    let both = &metadata.casks()[3];
    assert_eq!(both.version(), Some("latest"));
    assert!(both.auto_updates());
    assert!(metadata.needs_greedy("both"));
    assert_eq!(
        metadata.greedy_reason("both").as_deref(),
        Some("version :latest and auto_updates true; needs --greedy")
    );

    assert!(!metadata.needs_greedy("unknown-cask"));
    assert_eq!(metadata.greedy_reason("unknown-cask"), None);
}

#[test]
fn brew_4_2_0_reports_administrator_casks() {
    let metadata = cask_metadata(&transcript("4.2.0"))
        .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

    assert!(!metadata.needs_administrator("firefox"));
    assert!(metadata.needs_administrator("1password"));
    assert!(!metadata.needs_administrator("unknown-cask"));

    assert_eq!(metadata.apps(), ["Firefox.app"]);
}

#[test]
fn brew_4_3_0_reads_uninstall_stanzas() {
    let metadata = cask_metadata(&transcript("4.3.0"))
        .unwrap_or_else(|error| panic!("transcript should parse: {error}"));

    let tokens: Vec<_> = metadata
        .casks()
        .iter()
        .map(|cask| cask.token().unwrap_or(""))
        .collect();
    assert_eq!(
        tokens,
        [
            "zoom",
            "menu-bar",
            "pkg-tool",
            "multi",
            "top-level",
            "agent-only",
            "agents"
        ]
    );

    // A `signal` uninstall entry still names the bundle id to stop.
    assert_eq!(metadata.casks()[0].quit_bundle_ids(), ["us.zoom.xos"]);
    assert!(metadata.casks()[0].apps().is_empty());
    assert!(metadata.needs_administrator("zoom"));

    // `quit` and `launchctl` in the artifact `uninstall` stanza.
    assert_eq!(
        metadata.casks()[1].quit_bundle_ids(),
        ["com.example.menubar"]
    );
    assert_eq!(metadata.casks()[1].launch_agents(), ["com.example.helper"]);
    assert_eq!(metadata.casks()[1].apps(), ["Menu Bar.app"]);

    // `quit` as an object with a `bundle` key.
    assert_eq!(
        metadata.casks()[2].quit_bundle_ids(),
        ["com.example.pkgtool"]
    );
    assert!(metadata.needs_administrator("pkg-tool"));

    // A `quit` array of several bundle ids.
    assert_eq!(
        metadata.casks()[3].quit_bundle_ids(),
        ["com.example.one", "com.example.two"]
    );

    // `uninstall` at the cask top level is read too.
    assert_eq!(
        metadata.casks()[4].quit_bundle_ids(),
        ["com.example.toplevel"]
    );
    assert!(metadata.needs_administrator("top-level"));

    // `launchctl` names agents to stop with `launchctl kickstart`.
    assert_eq!(metadata.casks()[5].launch_agents(), ["com.example.agent"]);
    assert_eq!(
        metadata.casks()[6].launch_agents(),
        ["com.example.one", "com.example.two", "com.example.three"]
    );
}
