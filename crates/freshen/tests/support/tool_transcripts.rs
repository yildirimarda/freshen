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
