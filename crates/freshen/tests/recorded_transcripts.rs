//! Test for the recorded-transcript fixture harness.
//!
//! Proves that parser tests can load recorded output per tool and version
//! instead of inlining canned strings.

#![cfg(feature = "test-harness")]

mod support;

use support::RecordedTranscripts;

#[test]
fn loads_a_recorded_transcript_per_tool_and_version() {
    let transcripts = RecordedTranscripts::default();

    let zypper = transcripts.stdout("zypper", "1.14", "zypper-ps-s");
    assert_eq!(zypper, "ssh.service\npostgresql.service\n");

    let dotnet = transcripts.stdout("dotnet", "8.0", "dotnet-tool-list-global");
    assert!(dotnet.contains("Package Id"));
    assert!(dotnet.contains("tool-one"));
    assert!(dotnet.contains("tool-two"));

    // The same tool at a different version is a distinct fixture.
    let path = transcripts.path("dotnet", "8.0", "dotnet-tool-list-global");
    assert!(path.ends_with("dotnet/8.0/dotnet-tool-list-global.txt"));
}

#[test]
#[should_panic(expected = "missing recorded transcript")]
fn a_missing_fixture_panics_with_a_clear_message() {
    let transcripts = RecordedTranscripts::default();
    transcripts.stdout("no-such-tool", "0.0", "no-such-command");
}
