//! End-to-end check that a second run of freshen changes nothing.
//!
//! The stub `brew` models an installed formula whose version only changes when
//! `brew upgrade --formula` runs. The first run upgrades it; the second run
//! must leave the installed version alone and issue no upgrade command.

#![cfg(all(feature = "test-harness", unix))]

mod support;

use std::fs;
use std::process::Output;

use support::StubTools;

const STEPS: &str = "brew,brew-formulae,brew-casks,brew-autoremove,brew-cleanup";

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn installed_version(tools: &StubTools) -> String {
    let path = tools.home().join("formula-version");
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn run(tools: &StubTools) -> Output {
    tools
        .command()
        .env_remove("ZELLIJ")
        .args(["--only", STEPS, "--yes", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn install_stateful_brew(tools: &StubTools) {
    fs::write(tools.home().join("formula-version"), "1.7.1\n")
        .unwrap_or_else(|error| panic!("failed to write formula state: {error}"));

    tools.set_script(
        "brew",
        r#"state="$FRESHEN_STUB_STATE/formula-version"
IFS= read -r version < "$state"
case "$*" in
  "list --versions")
    printf 'jq %s\n' "$version"
    ;;
  "outdated --formula")
    if [ "$version" = "1.7.1" ]; then
      printf 'jq (1.7.1) < 1.8.0\n'
    fi
    ;;
  "outdated --formula --quiet")
    if [ "$version" = "1.7.1" ]; then
      printf 'jq\n'
    fi
    ;;
  "upgrade --formula")
    printf '1.8.0\n' > "$state"
    ;;
esac"#,
    );
}

#[test]
fn a_second_run_does_not_upgrade_anything_again() {
    let tools = StubTools::new(&["brew"]);
    install_stateful_brew(&tools);

    let first = run(&tools);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert_eq!(
        installed_version(&tools),
        "1.8.0\n",
        "first run calls: {:?}, stdout: {}, stderr: {}",
        tools.step_calls(),
        stdout(&first),
        stderr(&first)
    );
    let first_calls = tools.step_calls();
    assert!(
        first_calls
            .iter()
            .any(|call| call == "brew upgrade --formula"),
        "first run did not upgrade the formula: {first_calls:?}"
    );
    assert!(
        stdout(&first).contains("jq 1.7.1 -> 1.8.0"),
        "first run did not report the version change: {}",
        stdout(&first)
    );

    let second = run(&tools);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert_eq!(
        installed_version(&tools),
        "1.8.0\n",
        "the second run changed the installed version"
    );

    let all_calls = tools.step_calls();
    let second_calls = &all_calls[first_calls.len()..];
    assert!(
        !second_calls
            .iter()
            .any(|call| call == "brew upgrade --formula" || call == "brew upgrade --cask --greedy"),
        "the second run issued an upgrade: {second_calls:?}"
    );
    assert!(
        stdout(&second).contains("No outdated formulae."),
        "second run output: {}",
        stdout(&second)
    );
    assert!(
        !stdout(&second).contains("jq 1.7.1 -> 1.8.0"),
        "second run reported a version change: {}",
        stdout(&second)
    );
}
