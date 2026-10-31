#![allow(unused)]
//! Shared transcript helpers for uv, mise and tooling stub tests.

/// Named insertion: uv query, upgrade and prune steps.
pub fn uv_steps() -> &'static [&'static str] {
    &[
        "uv tool list --outdated",
        "uv tool upgrade --all",
        "uv cache prune",
    ]
}

/// Named insertion: full uv transcript built from named insertions.
pub fn uv_transcript() -> Vec<&'static str> {
    uv_steps().to_vec()
}

/// Named insertion: mise steps with self-update (brew not installed).
pub fn mise_full_steps() -> &'static [&'static str] {
    &[
        "mise outdated",
        "mise self-update -y",
        "mise upgrade",
        "mise ls --prunable --json",
        "mise prune -y",
    ]
}

/// Named insertion: full mise transcript with self-update.
pub fn mise_full_transcript() -> Vec<&'static str> {
    mise_full_steps().to_vec()
}

/// Named insertion: mise steps with brew check (no self-update).
pub fn mise_brew_steps() -> &'static [&'static str] {
    &[
        "mise outdated",
        "brew list --formula mise",
        "mise upgrade",
        "mise ls --prunable --json",
        "mise prune -y",
    ]
}

/// Named insertion: mise steps with both brew check and self-update
/// (brew check fails, so self-update runs too).
pub fn mise_brew_failed_steps() -> &'static [&'static str] {
    &[
        "mise outdated",
        "brew list --formula mise",
        "mise self-update -y",
        "mise upgrade",
        "mise ls --prunable --json",
        "mise prune -y",
    ]
}

/// Named insertion: full mise transcript with failed brew check.
pub fn mise_brew_failed_transcript() -> Vec<&'static str> {
    mise_brew_failed_steps().to_vec()
}

/// Named insertion: full mise transcript with brew.
pub fn mise_brew_transcript() -> Vec<&'static str> {
    mise_brew_steps().to_vec()
}

/// Named insertion: stale node process table queries.
pub fn stale_node_ps_steps() -> &'static [&'static str] {
    &["ps -axo pid=,ppid=,command=", "ps -o comm= -p 1"]
}

/// Named insertion: Mac App Store account check.
pub fn mas_account() -> &'static [&'static str] {
    &["mas account"]
}

/// Named insertion: Mac App Store outdated query.
pub fn mas_outdated() -> &'static [&'static str] {
    &["mas outdated"]
}

/// Named insertion: Mac App Store upgrade.
pub fn mas_upgrade() -> &'static [&'static str] {
    &["mas upgrade"]
}

/// Named insertion: full Mac App Store upgrade transcript.
pub fn mas_full_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(mas_account());
    t.extend_from_slice(mas_outdated());
    t.extend_from_slice(mas_upgrade());
    t.extend_from_slice(mas_outdated());
    t
}

/// Named insertion: Mac App Store check-mode transcript.
pub fn mas_check_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(mas_account());
    t.extend_from_slice(mas_outdated());
    t
}

/// Named insertion: destructive cleanup commands (mutating).
pub fn destructive_cleanup_commands() -> &'static [&'static str] {
    &[
        "brew autoremove",
        "brew cleanup --prune=all",
        "uv cache prune",
        "mise prune -y",
        "docker image prune -f",
    ]
}

/// Named insertion: destructive cleanup dry-run commands (read-only).
pub fn destructive_dry_run_commands() -> &'static [&'static str] {
    &[
        "brew autoremove --dry-run",
        "brew cleanup --dry-run --prune=all",
    ]
}

/// Named insertion: macOS system updates list query.
pub fn softwareupdate_list() -> &'static [&'static str] {
    &["softwareupdate --list"]
}

/// Named insertion: macOS system updates install all.
pub fn softwareupdate_install_all() -> &'static [&'static str] {
    &["sudo -n softwareupdate --install --all"]
}

/// Named insertion: macOS system updates install command line tools.
pub fn softwareupdate_install_command_line() -> &'static [&'static str] {
    &["sudo -n softwareupdate --install Command Line Tools for Xcode"]
}

/// Named insertion: macOS updates check-mode transcript (single list query).
pub fn macos_updates_check_transcript() -> Vec<&'static str> {
    softwareupdate_list().to_vec()
}

/// Named insertion: macOS updates report transcript.
pub fn macos_updates_report_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(softwareupdate_list());
    t.extend_from_slice(softwareupdate_list());
    t
}

/// Named insertion: macOS updates install transcript.
pub fn macos_updates_install_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(softwareupdate_list());
    t.extend_from_slice(softwareupdate_install_all());
    t.extend_from_slice(softwareupdate_list());
    t
}

/// Named insertion: macOS updates command-line-tools transcript.
pub fn macos_updates_command_line_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(softwareupdate_list());
    t.extend_from_slice(softwareupdate_install_command_line());
    t.extend_from_slice(softwareupdate_list());
    t
}

/// Named insertion: uv network retry transcript (transient error retries).
pub fn uv_network_retry_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(&["uv tool list --outdated"]);
    t.extend_from_slice(&["uv tool upgrade --all"]);
    t.extend_from_slice(&["uv tool upgrade --all"]);
    t
}

/// Named insertion: uv network single-attempt transcript.
pub fn uv_network_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(&["uv tool list --outdated"]);
    t.extend_from_slice(&["uv tool upgrade --all"]);
    t
}

/// Named insertion: brew-formulae full transcript (query, upgrade, check).
pub fn brew_formulae_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(&["brew outdated --formula"]);
    t.extend_from_slice(&["brew list --pinned --formula"]);
    t.extend_from_slice(&["brew outdated --formula --quiet"]);
    t.extend_from_slice(&["brew list --pinned --formula"]);
    t.extend_from_slice(&["brew services list"]);
    t.extend_from_slice(&["brew upgrade --formula"]);
    t.extend_from_slice(&["brew outdated --formula"]);
    t
}

/// Named insertion: brew formula steps.
pub fn brew_formula_steps() -> &'static [&'static str] {
    &[
        "brew outdated --formula",
        "brew outdated --formula --quiet",
        "brew list --pinned --formula",
        "brew services list",
        "brew upgrade --formula",
    ]
}

/// Named insertion: brew cask query steps.
pub fn brew_cask_query_steps() -> &'static [&'static str] {
    &[
        "brew outdated --cask --greedy",
        "brew outdated --cask --greedy --quiet",
        "brew list --pinned --cask",
    ]
}

/// Named insertion: brew autoremove steps.
pub fn brew_autoremove_steps() -> &'static [&'static str] {
    &["brew autoremove --dry-run", "brew autoremove"]
}

/// Named insertion: brew cleanup steps.
pub fn brew_cleanup_steps() -> &'static [&'static str] {
    &[
        "brew cleanup --dry-run --prune=all",
        "brew cleanup --prune=all",
    ]
}

/// Named insertion: full brew transcript (update, formulae, casks, autoremove, cleanup).
pub fn brew_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(&["brew update"]);
    t.extend_from_slice(brew_formula_steps());
    t.extend_from_slice(brew_cask_query_steps());
    t.extend_from_slice(&["brew info --cask --json=v2 firefox"]);
    t.extend_from_slice(&["brew upgrade --cask"]);
    t.extend_from_slice(brew_autoremove_steps());
    t.extend_from_slice(brew_cleanup_steps());
    t
}

/// Named insertion: apt-get update, upgrade and cleanup steps (with --cleanup).
pub fn apt_steps() -> &'static [&'static str] {
    &[
        "apt-get -s upgrade",
        "apt-get update",
        "apt-get upgrade -y",
        "apt-get autoremove -y",
        "apt-get autoclean",
    ]
}

/// Named insertion: full apt transcript (simulated upgrade query, update,
/// upgrade, autoremove and autoclean).
pub fn apt_transcript() -> Vec<&'static str> {
    apt_steps().to_vec()
}

/// Named insertion: probe stub transcript.
pub fn probe_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(&["probe status"]);
    t.extend_from_slice(&["probe status"]);
    t.extend_from_slice(&["probe status"]);
    t
}

/// Named insertion: slow step transcript.
pub fn slow_transcript() -> Vec<&'static str> {
    vec!["slow"]
}

/// Named insertion: brew update-only transcript.
pub fn brew_update_transcript() -> Vec<&'static str> {
    ["brew update"].to_vec()
}

/// Named insertion: stub sequence transcript for the ordered run test.
pub fn stub_sequence_transcript() -> Vec<&'static str> {
    let mut t = Vec::new();
    t.extend_from_slice(&["brew update"]);
    t.extend_from_slice(&["brew upgrade --formula"]);
    t.extend_from_slice(&["brew upgrade --cask --greedy"]);
    t.extend_from_slice(&["uv tool upgrade --all"]);
    t.extend_from_slice(&["mise upgrade"]);
    t.extend_from_slice(&["docker image prune -f"]);
    t.extend_from_slice(&["colima start"]);
    t
}
