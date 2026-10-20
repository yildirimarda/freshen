# freshen

One command to keep a developer machine current: **Homebrew**, **Docker (Colima) images**, **uv tools** and **mise** (Node & friends) — in the right order, without babysitting.

```bash
freshen
```

The implementation is the Rust workspace under `crates/`: `freshen-lib` holds all logic and the `freshen` binary is a thin CLI wrapper. It runs on macOS, Linux and Windows. The roadmap ([PLAN.md](PLAN.md)) covers the remaining steps, a config file, rollback snapshots and scheduling. Every step is optional: if a tool is not installed, its section is skipped.

## What it does

| Step | Commands |
|---|---|
| Homebrew | `brew update` → `brew upgrade --formula` → quit running apps → `brew upgrade --cask --greedy` → `brew autoremove` → `brew cleanup --prune=all` → reopen the apps it quit |
| Colima / Docker | If `colima`/`lima` itself is about to be upgraded: bring compose projects down, stop standalone containers, stop Colima, upgrade, start Colima, bring everything back up. Afterwards: `docker compose pull` + `up -d` for running compose projects (only changed containers are recreated) and `docker image prune -f` |
| uv | `uv tool upgrade --all` → `uv cache prune` |
| mise | `mise self-update` (only if mise was not installed via brew) → `mise upgrade` → `mise prune -y`, plus a warning for processes still running on an old Node version |

A failing step never aborts the run. Failures are collected and listed at the end (exit code 1).

### Details worth knowing

- **Formulae are upgraded before apps are closed**, so apps stay open during the slow part.
- **Apps are only quit when a cask upgrade will replace them.** Quit is graceful (AppleScript, 3 s timeout so a "Save changes?" dialog cannot hang the run). Apps that do not quit within `QUIT_TIMEOUT` are left alone unless `--force-kill` is given.
- **The terminal running freshen is never quit.** Detected via `__CFBundleIdentifier` (works with Terminal, iTerm2, Ghostty, ...).
- **Reopening uses `open -g`**, so apps do not steal focus.
- **`mise upgrade` is used without `--bump`** on purpose: with `node@lts` in your config, `--bump` would pin `lts` to a fixed version.
- **Zellij**: if zellij is outdated and you run freshen inside a session, you get a warning (server/client versions must match).

## Requirements

- macOS, Linux or Windows; a prebuilt `freshen` binary or Rust 1.81+ to build from source
- [Homebrew](https://brew.sh) — required for the Homebrew section
- Optional: `colima` + `docker`, `uv`, `mise`

## Usage

```bash
freshen                       # interactive: asks before quitting apps
freshen --yes                 # no prompt
freshen --force-kill          # kill apps that refuse to quit (unsaved data may be lost)
QUIT_TIMEOUT=40 freshen       # wait longer for apps to quit
freshen --dry-run             # detect and plan, change nothing
freshen --list-steps          # show every step, its detection and skip reason
freshen --only brew,uv --skip brew-casks   # select steps
```

| Variable | Default | Meaning |
|---|---|---|
| `QUIT_TIMEOUT` | `20` | Seconds to wait for an app to quit |

`--yes` and `--force-kill` replace the old `ASSUME_YES`/`FORCE_KILL` environment variables.

## Migrating from update-all.sh

The zsh implementation has been removed now that the Rust port reached parity, and `freshen` is the implementation. For one release `update-all.sh` remains as a deprecated POSIX `sh` wrapper: it forwards to `freshen` and translates `ASSUME_YES=1`/`FORCE_KILL=1` into `--yes`/`--force-kill`, so existing aliases keep working. The wrapper needs `freshen` on `PATH` (install it with `cargo install --path crates/freshen` or use a release binary) and will be removed in a later release.

| Old | New |
|---|---|
| `./update-all.sh` | `freshen` |
| `ASSUME_YES=1 ./update-all.sh` | `freshen --yes` |
| `FORCE_KILL=1 ./update-all.sh` | `freshen --force-kill` |
| `QUIT_TIMEOUT=40 ./update-all.sh` | `QUIT_TIMEOUT=40 freshen` |
| `alias update-all=.../update-all.sh` | `alias update-all=freshen` |

## Safety notes

- Run it while you are at the machine: it closes and reopens GUI apps.
- `--greedy` upgrades casks that auto-update themselves too. Casks with `version :latest` show up as outdated every time — that is Homebrew behaviour, not a bug here.
- Global npm packages do not follow a Node upgrade. List them in `~/.default-npm-packages` and mise reinstalls them for every new Node version.

## Development

The project is plan-driven: see [PLAN.md](PLAN.md) for what exists and what is next, [CONTRIBUTING.md](CONTRIBUTING.md) for how to contribute, and [AGENTS.md](AGENTS.md) if you use an autonomous coding agent ([agentloop](https://github.com/yildirimarda/agentloop)).

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features   # Rust workspace tests
bash tests/run-tests.sh     # repository-level checks
```

The Rust binary accepts the same options on macOS, Linux and Windows:

```bash
cargo run -p freshen -- --dry-run                    # detect, change nothing
cargo run -p freshen -- --list-steps                 # detect every step and explain skips
cargo run -p freshen -- --only brew,uv --skip casks  # select steps
cargo run -p freshen -- --yes --force-kill           # no prompts, kill stubborn apps
```

## License

MIT — see [LICENSE](LICENSE).
