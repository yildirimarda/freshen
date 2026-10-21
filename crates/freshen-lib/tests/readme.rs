use std::fs;
use std::path::PathBuf;

use freshen_lib::{supported_session_managers, supported_terminals};

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
