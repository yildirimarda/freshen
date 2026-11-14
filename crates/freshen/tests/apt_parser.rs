//! Recorded-transcript fixtures for the `apt-get -s upgrade` parser.
//!
//! Each fixture is a recording of `apt-get -s upgrade` from a specific
//! distribution, stored under
//! `tests/fixtures/apt/<distribution>/apt-get--s-upgrade.txt`. The parser is
//! exercised against every recorded distribution so output drift between
//! distributions is caught by the recorded-transcript harness instead of
//! inline strings.

#![cfg(feature = "test-harness")]

mod support;

use freshen_lib::apt::parse_apt_simulated_upgrade;
use support::RecordedTranscripts;

/// `apt-get -s upgrade` recordings, one per distribution.
const DISTRIBUTIONS: &[(&str, &str)] = &[
    ("debian-12", "apt-get--s-upgrade"),
    ("ubuntu-22.04", "apt-get--s-upgrade"),
    ("kali-2024.2", "apt-get--s-upgrade"),
    ("linux-mint-21", "apt-get--s-upgrade"),
    ("raspberry-pi-os-12", "apt-get--s-upgrade"),
];

fn transcript(distribution: &str) -> String {
    let transcripts = RecordedTranscripts::default();
    transcripts.stdout("apt", distribution, "apt-get--s-upgrade")
}

#[test]
fn each_recorded_apt_distribution_parses_without_drift() {
    for (distribution, _) in DISTRIBUTIONS {
        let items = parse_apt_simulated_upgrade(&transcript(distribution));
        assert!(
            !items.is_empty(),
            "apt {distribution} `apt-get -s upgrade` should yield at least one outdated package"
        );
    }
}

#[test]
fn debian_12_bookworm_parses_curl_git_and_openssh_client() {
    let items = parse_apt_simulated_upgrade(&transcript("debian-12"));
    let names: Vec<_> = items.iter().map(|item| item.name()).collect();
    assert_eq!(names, ["curl", "git", "openssh-client"]);
    assert_eq!(items[0].current(), Some("7.88.1-10+deb12u5"));
    assert_eq!(items[0].latest(), Some("7.88.1-10+deb12u6"));
    assert_eq!(items[1].current(), Some("1:2.39.2-1.2"));
    assert_eq!(items[1].latest(), Some("1:2.39.5-1~deb12u1"));
    assert_eq!(items[2].current(), Some("1:8.9p1-3+deb12u2"));
    assert_eq!(items[2].latest(), Some("1:8.9p1-3+deb12u3"));
}

#[test]
fn ubuntu_22_04_jammy_parses_curl_and_git() {
    let items = parse_apt_simulated_upgrade(&transcript("ubuntu-22.04"));
    let names: Vec<_> = items.iter().map(|item| item.name()).collect();
    assert_eq!(names, ["curl", "git"]);
    assert_eq!(items[0].current(), Some("7.81.0-1ubuntu1.17"));
    assert_eq!(items[0].latest(), Some("7.88.0-1ubuntu1.3"));
    assert_eq!(items[1].current(), Some("1:2.34.1-1ubuntu1.10"));
    assert_eq!(items[1].latest(), Some("1:2.43.0-2ubuntu0.1"));
}

#[test]
fn kali_2024_2_parses_git_and_curl() {
    let items = parse_apt_simulated_upgrade(&transcript("kali-2024.2"));
    let names: Vec<_> = items.iter().map(|item| item.name()).collect();
    assert_eq!(names, ["git", "curl"]);
    assert_eq!(items[0].current(), Some("1:2.43.4-2kali1"));
    assert_eq!(items[0].latest(), Some("1:2.45.2-1kali1"));
    assert_eq!(items[1].current(), Some("7.88.1-10+kali1"));
    assert_eq!(items[1].latest(), Some("7.92.1-1"));
}

#[test]
fn linux_mint_21_parses_mintupdate_and_curl() {
    let items = parse_apt_simulated_upgrade(&transcript("linux-mint-21"));
    let names: Vec<_> = items.iter().map(|item| item.name()).collect();
    assert_eq!(names, ["mintupdate", "curl"]);
    assert_eq!(items[0].current(), Some("5.8.4.3"));
    assert_eq!(items[0].latest(), Some("5.8.4.4"));
    assert_eq!(items[1].current(), Some("7.81.0-1ubuntu1.17"));
    assert_eq!(items[1].latest(), Some("7.88.0-1ubuntu1.3"));
}

#[test]
fn raspberry_pi_os_12_parses_raspi_firmware_and_curl() {
    let items = parse_apt_simulated_upgrade(&transcript("raspberry-pi-os-12"));
    let names: Vec<_> = items.iter().map(|item| item.name()).collect();
    assert_eq!(names, ["raspi-firmware", "curl"]);
    assert_eq!(items[0].current(), Some("1:1.20240605-1"));
    assert_eq!(items[0].latest(), Some("1:1.20240704-1"));
    assert_eq!(items[1].current(), Some("7.88.1-10+deb12u5+rpt1"));
    assert_eq!(items[1].latest(), Some("7.88.1-10+deb12u6+rpt1"));
}
