//! Recorded-transcript fixtures for the `docker compose ls --format json`
//! parser.
//!
//! Each fixture is a recording of `docker compose ls --format json` from a
//! specific Docker Compose version, stored under
//! `tests/fixtures/docker/<version>/docker-compose-ls.txt`. The parser is
//! exercised against every recorded version so output drift between Docker
//! Compose releases is caught by the recorded-transcript harness instead of
//! inline strings.

#![cfg(feature = "test-harness")]

mod support;

use freshen_lib::{parse_compose_projects, ComposeProject};
use support::RecordedTranscripts;

/// `docker compose ls --format json` recordings, one per Docker Compose version.
const VERSIONS: &[(&str, &str)] = &[
    ("2.20.0", "docker-compose-ls"),
    ("2.24.5", "docker-compose-ls"),
    ("2.29.0", "docker-compose-ls"),
];

fn transcript(version: &str) -> String {
    let transcripts = RecordedTranscripts::default();
    transcripts.stdout("docker", version, "docker-compose-ls")
}

#[test]
fn each_recorded_compose_version_parses_without_drift() {
    for (version, _) in VERSIONS {
        let projects = parse_compose_projects(&transcript(version));
        assert!(
            !projects.is_empty(),
            "docker compose {version} `docker compose ls --format json` should yield at least one project"
        );
    }
}

#[test]
fn compose_2_20_0_lists_web_and_db() {
    let projects = parse_compose_projects(&transcript("2.20.0"));
    let names: Vec<_> = projects.iter().map(ComposeProject::name).collect();
    assert_eq!(names, ["web", "db"]);
    assert_eq!(
        projects[0].config_files(),
        &["/srv/web/compose.yml", "/srv/web/override.yml"]
    );
    assert_eq!(projects[1].config_files(), &["/srv/db/compose.yml"]);
}

#[test]
fn compose_2_24_5_reads_the_skip_label() {
    let projects = parse_compose_projects(&transcript("2.24.5"));
    let names: Vec<_> = projects.iter().map(ComposeProject::name).collect();
    assert_eq!(names, ["web", "cache"]);
    assert!(projects[0].has_label("freshen.com/skip"));
    assert!(!projects[1].has_label("freshen.com/skip"));
}

#[test]
fn compose_2_29_0_reads_empty_labels() {
    let projects = parse_compose_projects(&transcript("2.29.0"));
    let names: Vec<_> = projects.iter().map(ComposeProject::name).collect();
    assert_eq!(names, ["monitoring", "worker"]);
    assert!(projects[1].labels().is_empty());
    assert_eq!(
        projects[0].config_files(),
        &[
            "/srv/monitoring/compose.yml",
            "/srv/monitoring/override.yml"
        ]
    );
}
