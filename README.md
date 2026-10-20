# freshen

One command to keep a developer machine current. Today it targets macOS: **Homebrew**, **Docker (Colima) images**, **uv tools** and **mise** (Node & friends) — in the right order, without babysitting.

```bash
./update-all.sh
```

Currently a single zsh script, no dependencies beyond the tools it updates. The roadmap ([PLAN.md](PLAN.md)) is a Rust rewrite with Linux and Windows support, a config file, `--check`/`--dry-run`, rollback snapshots and scheduling. Every step is optional: if a tool is not installed, its section is skipped.

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
- **Apps are only quit when a cask upgrade will replace them.** Quit is graceful (AppleScript, 3 s timeout so a "Save changes?" dialog cannot hang the script). Apps that do not quit within `QUIT_TIMEOUT` are left alone unless `FORCE_KILL=1`.
- **The terminal running the script is never quit.** Detected via `__CFBundleIdentifier` (works with Terminal, iTerm2, Ghostty, ...).
- **Reopening uses `open -g`**, so apps do not steal focus.
- **`mise upgrade` is used without `--bump`** on purpose: with `node@lts` in your config, `--bump` would pin `lts` to a fixed version.
- **Zellij**: if zellij is outdated and you run the script inside a session, you get a warning (server/client versions must match).

## Requirements

- macOS, zsh (the default shell)
- [Homebrew](https://brew.sh) — required for the Homebrew section
- Optional: `colima` + `docker`, `uv`, `mise`
- `python3` (ships with the Xcode command line tools) for parsing `brew info --json` / `docker compose ls --format json`

## Usage

```bash
chmod +x update-all.sh
./update-all.sh                       # interactive: asks before quitting apps
ASSUME_YES=1 ./update-all.sh          # no prompt
FORCE_KILL=1 ./update-all.sh          # kill apps that refuse to quit (unsaved data may be lost)
QUIT_TIMEOUT=40 ./update-all.sh       # wait longer for apps to quit
```

| Variable | Default | Meaning |
|---|---|---|
| `ASSUME_YES` | `0` | Do not ask before quitting apps for cask upgrades |
| `FORCE_KILL` | `0` | `kill -9` apps that did not quit within the timeout |
| `QUIT_TIMEOUT` | `20` | Seconds to wait for an app to quit |

Handy alias:

```bash
alias update-all="$HOME/path/to/freshen/update-all.sh"
```

## Safety notes

- Run it while you are at the machine: it closes and reopens GUI apps.
- `--greedy` upgrades casks that auto-update themselves too. Casks with `version :latest` show up as outdated every time — that is Homebrew behaviour, not a bug here.
- Global npm packages do not follow a Node upgrade. List them in `~/.default-npm-packages` and mise reinstalls them for every new Node version.

## Development

The project is plan-driven: see [PLAN.md](PLAN.md) for what exists and what is next, [CONTRIBUTING.md](CONTRIBUTING.md) for how to contribute, and [AGENTS.md](AGENTS.md) if you use an autonomous coding agent ([agentloop](https://github.com/yildirimarda/agentloop)).

```bash
zsh -n update-all.sh        # syntax check (what CI runs)
cargo test --all-features   # Rust workspace tests
cargo clippy --all-targets --all-features -- -D warnings
```

The Rust binary accepts the same options on macOS, Linux and Windows:

```bash
cargo run -p freshen -- --dry-run                    # detect, change nothing
cargo run -p freshen -- --only brew,uv --skip casks  # select steps
cargo run -p freshen -- --yes --force-kill           # no prompts, kill stubborn apps
```

## License

MIT — see [LICENSE](LICENSE).
