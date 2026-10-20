# 0002. Choose the core dependencies

- Status: accepted
- Date: 2026-10-20

## Context

PLAN.md Milestone 2 ports `update-all.sh` to the Rust workspace. The port
needs a small set of well-trodden crates for the cross-cutting concerns every
step shares: command-line parsing, configuration, error handling, signal
handling, executable lookup and progress output. Picking them now — before
the steps are written — keeps every module on the same stack and lets the
dependency checks (cargo-deny) gate new additions from the start.

## Decision

Add these dependencies, all MSRV-compatible with the workspace's
`rust-version` (1.81) and published under permissive licenses:

- `clap` (with `derive`) — the `freshen` binary's command-line interface.
- `serde` (with `derive`) + `toml` — the config file schema and parsing.
- `thiserror` in `freshen-lib` and `anyhow` in the `freshen` binary — errors.
- `ctrlc` — SIGINT/SIGTERM handling with cleanup.
- `which` — locating tools on `PATH`.
- `indicatif` — progress bars and spinners.

### Error handling split

`freshen-lib` returns precise `thiserror` error enums so each step's failure
mode is a distinct, matchable type; the thin `freshen` binary converts them
into user-facing messages with `anyhow` context. This matches the project
style rule ("errors via `thiserror`/`anyhow`") and keeps the library free of
`anyhow`'s boxed errors, which the step abstraction's typed failures need.

### CLI

`clap` with the `derive` feature gives a declarative command-line spec, help
text, shell completions and structured validation with no hand-written
parser. It is the de-facto standard for Rust CLIs and will carry the planned
`--dry-run`, `--only`, `--skip`, `--yes`, `--force-kill`, `--check`,
`--json`, `--list-steps` flags and the `schedule`, `logs`, `history`
subcommands (Milestones 2, 3, 9, 10).

### Config

PLAN.md Milestone 9 introduces a TOML config file. `serde` + `toml` is the
standard pairing: `toml` parses straight into typed `#[derive(Deserialize)]`
structs, and validation errors can point at the offending line. `toml` 0.8's
`serde` feature uses `toml_edit` under the hood, so error spans stay good.

### Signals

`ctrlc` is the smallest well-maintained crate for registering a
SIGINT/SIGTERM handler on macOS, Linux and Windows. Milestone 3's "restore
everything that was stopped on interrupt" behaviour is built on it.

### Tool detection

The zsh script uses `command -v`; `which` is the cross-platform equivalent —
it searches `PATH` (honouring Windows `PATHEXT`) and reports where a tool
lives, which the step `detect()` methods and the planned `--doctor` need.

### Progress

`indicatif` renders progress bars and spinners when attached to a terminal
and degrades to plain output when piped or in CI (required by the "rich
terminal output" item in Milestone 9), and honours `NO_COLOR`.

## Alternatives considered

- **Hand-rolled argument parsing (`std::env`)** — fine for two flags, not for
  the flag set above; we would reimplement `--help`, validation and
  completions. Not chosen.
- **`structopt` / `argh`** — `structopt` is unmaintained (superseded by
  `clap` derive); `argh` is smaller but lacks subcommand and completions
  ergonomics. Not chosen.
- **`toml_edit` directly instead of `toml`** — `toml_edit` is for editing
  files; freshen only reads config, so `toml` is the right layer (it
  delegates to `toml_edit` internally). Not chosen.
- **`anyhow` only, everywhere** — convenient but loses the typed error enums
  the step abstraction and failure-collection semantics need; the library
  stays on `thiserror`. Not chosen.
- **`signal-hook` instead of `ctrlc`** — more powerful (multiple handlers,
  flag-based), but heavier than the single top-level handler freshen needs.
  `ctrlc` is enough and simpler. Not chosen.
- **`std::env::current_exe`-style lookup instead of `which`** — does not
  search `PATH` and misses `PATHEXT` handling on Windows. Not chosen.
- **`console` instead of `indicatif`** — lower-level styling only; no bars.
  Not chosen.
- **No progress output (`eprintln`)** — loses the live status the zsh script
  gives; `indicatif`'s plain mode is needed for CI anyway. Not chosen.

## Consequences

- Every future module uses the same stack; this ADR is the reference for
  "which crate did we pick and why".
- `cargo-deny` (added to CI alongside this ADR) enforces the permissive
  license allow-list and checks advisories, so new dependencies must fit the
  policy.
- Slightly more build time and binary size than a hand-rolled CLI, in
  exchange for correctness and maintainability.
- This ADR is the decision, not the wiring: the crates are added to
  `Cargo.toml` as the corresponding modules land (Milestones 2+), so each
  addition is tied to code that actually uses it.
