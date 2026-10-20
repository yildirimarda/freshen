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
- **Every run starts with preflight checks**: the network must be reachable, at least 1 GiB must be free, the battery must be at or above 20% while discharging, and the connection must not be metered or a hotspot. A failed check stops before any step runs (exit code 3) and lists every problem. Each check can be disabled with `--skip-network-check`, `--skip-disk-check`, `--skip-battery-check` or `--skip-metered-check`; `--min-free-disk-gib` and `--battery-threshold` change the thresholds. `--dry-run` and `--check` skip preflight because they change nothing.
- **Another package manager must not be running.** Before changing anything, freshen checks for Homebrew, apt/dpkg (including a held dpkg or apt lock) and winget. It waits up to 30 seconds for the other process to finish, then stops with exit code 3 and names the process and its PID. Use `--package-manager-wait <SECONDS>` to change the wait (0 aborts immediately) or `--skip-package-manager-check` to bypass the check. `--dry-run` and `--check` skip it because they change nothing.
- **Every run records a pre-upgrade snapshot** of installed versions (`brew list --versions`, `brew bundle dump`, `uv tool list`, `mise ls`, Docker images) in a timestamped file under the platform state directory (`~/.local/state/freshen/snapshots` on Linux, `~/Library/Application Support/freshen/snapshots` on macOS, `%LOCALAPPDATA%\freshen\snapshots` on Windows). Set `FRESHEN_SNAPSHOT_DIR` to override the location. The summary prints the commands that reinstall the recorded versions.
- **Only one update runs at a time.** A second run stops with an error naming the PID that holds the lock. The lock lives under the platform state directory (`FRESHEN_LOCK_DIR` overrides it), and `--dry-run`/`--check` do not take it.

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
freshen --check               # report outdated items, change nothing (exit 2 if updates are pending)
freshen --json                # print the report as JSON (before/after versions per step)
freshen --markdown            # print the report as Markdown
freshen --list-steps          # show every step, its detection and skip reason
freshen --only brew,uv --skip brew-casks   # select steps
freshen --skip-metered-check  # run even on a phone hotspot
freshen --min-free-disk-gib 5 --battery-threshold 30   # custom preflight thresholds
freshen --package-manager-wait 0                      # abort instead of waiting for another package manager
```

| Variable | Default | Meaning |
|---|---|---|
| `QUIT_TIMEOUT` | `20` | Seconds to wait for an app to quit |
| `FRESHEN_SKIP_PREFLIGHT` | unset | Set to `1` to disable every preflight and package-manager check |
| `FRESHEN_LOCK_DIR` | platform state directory | Override where the single-run lock file lives |

`--yes` and `--force-kill` replace the old `ASSUME_YES`/`FORCE_KILL` environment variables.

## Migrating from update-all.sh

The zsh implementation and its deprecated POSIX `sh` compatibility wrapper have been removed now that the Rust port reached parity, and `freshen` is the implementation. Existing aliases and scripts should call `freshen` directly; install it with `cargo install --path crates/freshen` or use a release binary.

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
cargo run -p freshen -- --check                      # report outdated items, exit 2 when pending
cargo run -p freshen -- --json                       # machine-readable report with version changes
cargo run -p freshen -- --markdown                   # Markdown report with version changes
cargo run -p freshen -- --list-steps                 # detect every step and explain skips
cargo run -p freshen -- --only brew,uv --skip casks  # select steps
cargo run -p freshen -- --yes --force-kill           # no prompts, kill stubborn apps
cargo run -p freshen -- --skip-network-check         # bypass one preflight check
cargo run -p freshen -- --package-manager-wait 0     # do not wait for another package manager
```

## License

MIT — see [LICENSE](LICENSE).
