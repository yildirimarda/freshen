//! End-to-end check that a second run of freshen changes nothing.
//!
//! Every mutating step is exercised against stateful stubs. The stubs record
//! the installed versions, running containers and image digests in files under
//! `FRESHEN_STUB_STATE` and only change them when a command really has work to
//! do. The first run brings the machine up to date; the second run must leave
//! that state byte-for-byte identical and report no further upgrades.
//!
//! The Homebrew, uv, mise, Docker and Colima cases each have their own test so
//! a regression names the step that stopped being idempotent.

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

fn state(tools: &StubTools, name: &str) -> String {
    let path = tools.home().join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read state {}: {error}", path.display()))
}

fn write_state(tools: &StubTools, name: &str, value: &str) {
    fs::write(tools.home().join(name), value)
        .unwrap_or_else(|error| panic!("failed to write state {name}: {error}"));
}

fn installed_version(tools: &StubTools) -> String {
    state(tools, "formula-version")
}

fn run_steps(tools: &StubTools, steps: &str) -> Output {
    tools
        .command()
        .env_remove("ZELLIJ")
        .args(["--only", steps, "--yes", "--cleanup"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
}

fn run(tools: &StubTools) -> Output {
    run_steps(tools, STEPS)
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

fn install_stateful_uv(tools: &StubTools) {
    write_state(tools, "uv-tool-version", "1.0.0\n");
    write_state(tools, "uv-cache-bytes", "1024\n");

    tools.set_script(
        "uv",
        r#"state="$FRESHEN_STUB_STATE"
IFS= read -r tool_version < "$state/uv-tool-version"
IFS= read -r cache_bytes < "$state/uv-cache-bytes"
case "$*" in
  "tool list")
    printf 'demo-tool v%s\n' "$tool_version"
    ;;
  "tool upgrade --all")
    if [ "$tool_version" = "1.0.0" ]; then
      printf '2.0.0\n' > "$state/uv-tool-version"
      printf 'Updated demo-tool v1.0.0 -> v2.0.0\n'
    else
      printf 'All tools are up to date\n'
    fi
    ;;
  "cache prune")
    if [ "$cache_bytes" != "0" ]; then
      printf '0\n' > "$state/uv-cache-bytes"
      printf 'Pruned %s bytes from cache\n' "$cache_bytes"
    else
      printf 'Nothing to prune\n'
    fi
    ;;
esac"#,
    );
}

#[test]
fn a_second_run_does_not_upgrade_uv_tools_or_prune_the_cache_again() {
    let tools = StubTools::new(&["uv"]);
    install_stateful_uv(&tools);

    let first = run_steps(&tools, "uv");
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert_eq!(state(&tools, "uv-tool-version"), "2.0.0\n");
    assert_eq!(state(&tools, "uv-cache-bytes"), "0\n");
    let first_calls = tools.step_calls();
    assert_eq!(
        first_calls,
        ["uv tool upgrade --all", "uv cache prune"],
        "first run calls"
    );
    assert!(
        stdout(&first).contains("Pruned 1024 bytes from cache"),
        "first run output: {}",
        stdout(&first)
    );

    let second = run_steps(&tools, "uv");
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert_eq!(
        state(&tools, "uv-tool-version"),
        "2.0.0\n",
        "the second run changed the installed tool version"
    );
    assert_eq!(
        state(&tools, "uv-cache-bytes"),
        "0\n",
        "the second run changed the cache state"
    );

    let all_calls = tools.step_calls();
    let second_calls = &all_calls[first_calls.len()..];
    assert_eq!(
        second_calls,
        ["uv tool upgrade --all", "uv cache prune"],
        "second run calls"
    );
    assert!(
        stdout(&second).contains("Nothing to prune"),
        "second run output: {}",
        stdout(&second)
    );
}

fn install_stateful_mise(tools: &StubTools) {
    write_state(tools, "mise-version", "2024.1.0\n");
    write_state(tools, "mise-node-version", "20.0.0\n");
    write_state(tools, "mise-unused", "2\n");

    tools.set_script(
        "brew",
        r#"case "$*" in
  "list --formula mise")
    exit 1
    ;;
esac"#,
    );
    tools.set_script(
        "mise",
        r#"state="$FRESHEN_STUB_STATE"
IFS= read -r mise_version < "$state/mise-version"
IFS= read -r node_version < "$state/mise-node-version"
IFS= read -r unused < "$state/mise-unused"
case "$*" in
  "ls")
    printf 'node %s\n' "$node_version"
    ;;
  "where "*)
    ;;
  "self-update -y")
    if [ "$mise_version" = "2024.1.0" ]; then
      printf '2024.2.0\n' > "$state/mise-version"
      printf 'mise updated to 2024.2.0\n'
    else
      printf 'mise is already up to date\n'
    fi
    ;;
  "upgrade")
    if [ "$node_version" = "20.0.0" ]; then
      printf '21.0.0\n' > "$state/mise-node-version"
      printf 'node 20.0.0 -> 21.0.0\n'
    else
      printf 'All tools are up to date\n'
    fi
    ;;
  "prune -y")
    if [ "$unused" != "0" ]; then
      printf '0\n' > "$state/mise-unused"
      printf 'Removed %s unused versions\n' "$unused"
    else
      printf 'Nothing to prune\n'
    fi
    ;;
esac"#,
    );
}

#[test]
fn a_second_run_does_not_update_mise_or_prune_again() {
    let tools = StubTools::new(&["brew", "mise"]);
    install_stateful_mise(&tools);

    let first = run_steps(&tools, "mise");
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert_eq!(state(&tools, "mise-version"), "2024.2.0\n");
    assert_eq!(state(&tools, "mise-node-version"), "21.0.0\n");
    assert_eq!(state(&tools, "mise-unused"), "0\n");
    let first_calls = tools.step_calls();
    assert!(
        first_calls.contains(&"mise self-update -y".to_owned()),
        "first run calls: {first_calls:?}"
    );
    assert!(
        first_calls.contains(&"mise upgrade".to_owned()),
        "first run calls: {first_calls:?}"
    );
    assert!(
        first_calls.contains(&"mise prune -y".to_owned()),
        "first run calls: {first_calls:?}"
    );
    assert!(
        stdout(&first).contains("Removed 2 unused versions"),
        "first run output: {}",
        stdout(&first)
    );

    let second = run_steps(&tools, "mise");
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert_eq!(
        state(&tools, "mise-version"),
        "2024.2.0\n",
        "the second run changed the mise version"
    );
    assert_eq!(
        state(&tools, "mise-node-version"),
        "21.0.0\n",
        "the second run changed the installed node version"
    );
    assert_eq!(
        state(&tools, "mise-unused"),
        "0\n",
        "the second run changed the unused-version state"
    );

    let all_calls = tools.step_calls();
    let second_calls = &all_calls[first_calls.len()..];
    assert!(
        second_calls.contains(&"mise upgrade".to_owned()),
        "the second run did not re-check tools: {second_calls:?}"
    );
    assert!(
        second_calls.contains(&"mise prune -y".to_owned()),
        "the second run did not re-check unused versions: {second_calls:?}"
    );
    assert!(
        stdout(&second).contains("Nothing to prune"),
        "second run output: {}",
        stdout(&second)
    );
}

fn install_stateful_colima_and_docker(tools: &StubTools) {
    write_state(tools, "colima-version", "0.5.0\n");
    write_state(tools, "colima-running", "1\n");
    write_state(tools, "web-running", "1\n");
    write_state(tools, "lonely-running", "1\n");
    write_state(tools, "image-digest", "sha256:old\n");
    write_state(tools, "dangling-images", "1\n");

    tools.set_script(
        "brew",
        r#"state="$FRESHEN_STUB_STATE"
IFS= read -r colima_version < "$state/colima-version"
case "$*" in
  "outdated --formula --quiet")
    if [ "$colima_version" = "0.5.0" ]; then
      printf 'colima\n'
    fi
    ;;
  "outdated --formula")
    if [ "$colima_version" = "0.5.0" ]; then
      printf 'colima (0.5.0) < 0.6.0\n'
    fi
    ;;
  "upgrade --formula")
    printf '0.6.0\n' > "$state/colima-version"
    ;;
esac"#,
    );
    tools.set_script(
        "colima",
        r#"state="$FRESHEN_STUB_STATE"
IFS= read -r colima_running < "$state/colima-running"
case "$*" in
  "status")
    if [ "$colima_running" = "1" ]; then
      exit 0
    fi
    exit 1
    ;;
  "stop")
    printf '0\n' > "$state/colima-running"
    ;;
  "start")
    printf '1\n' > "$state/colima-running"
    ;;
esac"#,
    );
    tools.set_script(
        "docker",
        r#"state="$FRESHEN_STUB_STATE"
IFS= read -r colima_running < "$state/colima-running"
IFS= read -r web_running < "$state/web-running"
IFS= read -r lonely_running < "$state/lonely-running"
IFS= read -r image_digest < "$state/image-digest"
IFS= read -r dangling < "$state/dangling-images"
case "$*" in
  "compose ls --format json")
    if [ "$colima_running" = "1" ] && [ "$web_running" = "1" ]; then
      printf '[{"Name":"web","ConfigFiles":"/srv/web/compose.yml,/srv/web/override.yml"}]\n'
    else
      printf '[]\n'
    fi
    ;;
  "ps --format "*)
    if [ "$colima_running" = "1" ] && [ "$lonely_running" = "1" ]; then
      printf 'def\tlonely\t\n'
    fi
    ;;
  "compose -p web "*)
    case "$*" in
      *" down")
        printf '0\n' > "$state/web-running"
        ;;
      *" up -d")
        printf '1\n' > "$state/web-running"
        ;;
      *" pull --quiet")
        if [ "$image_digest" = "sha256:old" ]; then
          printf 'sha256:new\n' > "$state/image-digest"
          printf 'Pulled newer image for web\n'
        else
          printf 'Image for web is up to date\n'
        fi
        ;;
    esac
    ;;
  "stop "*)
    printf '0\n' > "$state/lonely-running"
    ;;
  "start "*)
    printf '1\n' > "$state/lonely-running"
    ;;
  "image prune -f")
    if [ "$dangling" != "0" ]; then
      printf '0\n' > "$state/dangling-images"
      printf 'Deleted %s dangling image\n' "$dangling"
    else
      printf 'Nothing to prune\n'
    fi
    ;;
esac"#,
    );
}

#[test]
fn a_second_run_does_not_restart_colima_or_change_containers_or_images() {
    let tools = StubTools::new(&["brew", "colima", "docker"]);
    install_stateful_colima_and_docker(&tools);

    let steps = "brew,brew-formulae,colima-stop,colima-start,docker";
    let first = run_steps(&tools, steps);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert_eq!(state(&tools, "colima-version"), "0.6.0\n");
    assert_eq!(state(&tools, "colima-running"), "1\n");
    assert_eq!(state(&tools, "web-running"), "1\n");
    assert_eq!(state(&tools, "lonely-running"), "1\n");
    assert_eq!(state(&tools, "image-digest"), "sha256:new\n");
    assert_eq!(state(&tools, "dangling-images"), "0\n");
    let first_calls = tools.step_calls();
    for expected in [
        "colima stop",
        "colima start",
        "docker stop def",
        "docker start def",
        "docker image prune -f",
    ] {
        assert!(
            first_calls.iter().any(|call| call == expected),
            "first run did not issue {expected}: {first_calls:?}"
        );
    }
    assert!(
        first_calls
            .iter()
            .any(|call| call.contains(" compose -p web ") && call.ends_with(" down")),
        "first run did not stop the compose project: {first_calls:?}"
    );
    assert!(
        first_calls
            .iter()
            .any(|call| call.contains(" compose -p web ") && call.ends_with(" up -d")),
        "first run did not restore the compose project: {first_calls:?}"
    );
    assert!(
        stdout(&first).contains("Deleted 1 dangling image"),
        "first run output: {}",
        stdout(&first)
    );
    assert!(
        stdout(&first).contains("stopping containers and colima"),
        "first run output: {}",
        stdout(&first)
    );

    let second = run_steps(&tools, steps);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    for (name, expected) in [
        ("colima-version", "0.6.0\n"),
        ("colima-running", "1\n"),
        ("web-running", "1\n"),
        ("lonely-running", "1\n"),
        ("image-digest", "sha256:new\n"),
        ("dangling-images", "0\n"),
    ] {
        assert_eq!(
            state(&tools, name),
            expected,
            "the second run changed {name}"
        );
    }

    let all_calls = tools.step_calls();
    let second_calls = &all_calls[first_calls.len()..];
    assert!(
        !second_calls
            .iter()
            .any(|call| call == "colima stop" || call == "colima start"),
        "the second run restarted colima: {second_calls:?}"
    );
    assert!(
        !second_calls
            .iter()
            .any(|call| call == "brew upgrade --formula"),
        "the second run upgraded colima again: {second_calls:?}"
    );
    assert!(
        !stdout(&second).contains("stopping containers and colima"),
        "second run output: {}",
        stdout(&second)
    );
    assert!(
        stdout(&second).contains("Nothing to prune"),
        "second run output: {}",
        stdout(&second)
    );
}
