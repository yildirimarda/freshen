use std::fs;
use std::path::PathBuf;

use freshen_lib::{supported_session_managers, supported_terminals};
use serde_json::Value;

fn readme() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

#[test]
fn readme_lists_every_supported_terminal() {
    let readme = readme();
    let expected = format!(
        "Supported terminal emulators: {}.",
        supported_terminals().join(", ")
    );

    assert!(
        readme.contains(&expected),
        "README.md must list every terminal from the detection table.\nExpected:\n{expected}"
    );
}

#[test]
fn readme_lists_every_supported_session_manager() {
    let readme = readme();
    let expected = format!(
        "Session managers checked before an upgrade: {}.",
        supported_session_managers().join(", ")
    );

    assert!(
        readme.contains(&expected),
        "README.md must list every session manager from the guard table.\nExpected:\n{expected}"
    );
}

#[test]
fn readme_no_longer_singles_out_zellij_or_two_terminals() {
    let readme = readme();

    assert!(
        !readme.contains("**Zellij**"),
        "README.md must not keep the zellij-only bullet"
    );
    assert!(
        !readme.contains("works with Terminal, iTerm2, Ghostty"),
        "README.md must not keep the Ghostty/iTerm2 example list"
    );
}

fn demo_cast_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/assets/freshen-demo.cast")
}

#[test]
fn readme_links_a_terminal_recording() {
    let readme = readme();

    assert!(
        readme.contains("asciinema"),
        "README.md must link to a terminal recording (asciinema)"
    );
    assert!(
        readme.contains("docs/assets/freshen-demo.cast"),
        "README.md must reference the demo cast file"
    );
}

#[test]
fn demo_cast_is_a_valid_asciinema_cast() {
    let path = demo_cast_path();
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let mut lines = content.lines();
    let header: Value = match lines.next() {
        Some(line) => serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("cast header is valid JSON: {error}")),
        None => panic!("cast has a header line"),
    };
    assert_eq!(
        header.get("version").and_then(Value::as_u64),
        Some(2),
        "cast header declares asciinema version 2"
    );
    for (index, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(line)
            .unwrap_or_else(|_| panic!("cast line {} is valid JSON", index + 2));
        assert!(
            event.is_array(),
            "cast line {} is an event array [time, kind, data]",
            index + 2
        );
        let arr = match event.as_array() {
            Some(arr) => arr,
            None => panic!(
                "cast line {} is an event array [time, kind, data]",
                index + 2
            ),
        };
        assert!(
            arr.len() >= 3,
            "cast line {} has [time, kind, data]",
            index + 2
        );
        assert!(
            arr[1].is_string(),
            "cast line {} event kind is a string",
            index + 2
        );
    }
}
