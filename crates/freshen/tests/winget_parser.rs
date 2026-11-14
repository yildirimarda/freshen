//! Recorded-transcript fixtures for the `winget upgrade` parser.
//!
//! Each fixture is a recording of `winget upgrade` from a specific winget
//! version, stored under `tests/fixtures/winget/<version>/winget-upgrade.txt`.
//! The parser is exercised against every recorded version so output drift
//! between winget releases is caught by the recorded-transcript harness
//! instead of inline strings.

#![cfg(feature = "test-harness")]

mod support;

use freshen_lib::winget::{parse_winget_packages, parse_winget_upgrade};
use support::RecordedTranscripts;

/// `winget upgrade` recordings, one per winget version.
const VERSIONS: &[(&str, &str)] = &[
    ("1.0.0", "winget-upgrade"),
    ("1.5.0", "winget-upgrade"),
    ("1.6.0", "winget-upgrade"),
];

fn transcript(version: &str) -> String {
    let transcripts = RecordedTranscripts::default();
    transcripts.stdout("winget", version, "winget-upgrade")
}

#[test]
fn each_recorded_winget_version_parses_without_drift() {
    for (version, _) in VERSIONS {
        let items = parse_winget_upgrade(&transcript(version));
        assert!(
            !items.is_empty(),
            "winget {version} `winget upgrade` should yield at least one outdated package"
        );
    }
}

#[test]
fn winget_1_0_0_lists_7zip_and_git() {
    let items = parse_winget_upgrade(&transcript("1.0.0"));
    let names: Vec<_> = items.iter().map(|item| item.name()).collect();
    assert_eq!(names, ["7zip", "Git"]);
    assert_eq!(items[0].current(), None);
    assert_eq!(items[0].latest(), Some("24.08"));
    assert_eq!(items[1].name(), "Git");
    assert_eq!(items[1].latest(), Some("2.45.2"));
}

#[test]
fn winget_1_5_0_parses_names_with_spaces() {
    let packages = parse_winget_packages(&transcript("1.5.0"));
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].name, "Microsoft Visual C++ Redistributable");
    assert_eq!(packages[0].id, "Microsoft.VC++.Redist");
    assert_eq!(packages[0].version, "14.42.34512");
    assert_eq!(packages[1].name, "Google Chrome");
    assert_eq!(packages[1].id, "Google.Chrome");
    assert_eq!(packages[1].version, "120.0.6099.109");
}

#[test]
fn winget_1_6_0_lists_powertoys_and_vlc() {
    let items = parse_winget_upgrade(&transcript("1.6.0"));
    let names: Vec<_> = items.iter().map(|item| item.name()).collect();
    assert_eq!(names, ["PowerToys", "VLC"]);
    assert_eq!(items[0].latest(), Some("0.80.0"));
    assert_eq!(items[1].latest(), Some("3.0.20"));
}
