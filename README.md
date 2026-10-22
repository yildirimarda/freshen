# freshen

One command to keep a developer machine current: **Homebrew**, **Docker (Colima) images**, **uv tools**, **mise** (Node & friends) and **Mac App Store apps** — in the right order, without babysitting.

```bash
freshen
```

The implementation is the Rust workspace under `crates/`: `freshen-lib` holds all logic and the `freshen` binary is a thin CLI wrapper. It runs on macOS, Linux and Windows. The roadmap ([PLAN.md](PLAN.md)) covers the remaining steps, a config file, rollback snapshots and scheduling. Every step is optional: if a tool is not installed, its section is skipped.

## What it does

| Step | Commands |
|---|---|
| Homebrew | `brew update` → `brew upgrade --formula` → quit running apps → `brew upgrade --cask` (with `--greedy` only for casks that need it) → reopen the apps it quit. With `--cleanup`: `brew autoremove` → `brew cleanup --prune=all` before reopening |
| Colima / Docker | If `colima`/`lima` itself is about to be upgraded: bring compose projects down, stop standalone containers, stop Colima, upgrade, start Colima, bring everything back up. Afterwards: `docker compose pull` + `up -d` for running compose projects (only changed containers are recreated). With `--cleanup`: `docker image prune -f` |
| uv | `uv tool upgrade --all`. With `--cleanup`: `uv cache prune` |
| mise | `mise self-update` (only if mise was not installed via brew) → `mise upgrade`, plus a warning for processes still running on an old Node, Python, Java or Go version, naming the app that started them. With `--cleanup`: `mise prune -y` |
| Mac App Store | `mas outdated` to list the App Store apps with a pending update → `mas upgrade` to install them. macOS only, and skipped when no Apple Account is signed in |

A failing step never aborts the run. Failures are collected and listed at the end (exit code 1).

### Details worth knowing

- **Formulae are upgraded before apps are closed**, so apps stay open during the slow part.
- **Apps are only quit when a cask upgrade will replace them.** Quit is graceful (AppleScript, 3 s timeout so a "Save changes?" dialog cannot hang the run). Apps that do not quit within `QUIT_TIMEOUT` are left alone unless `--force-kill` is given.
- **An app that updates itself is waited for, not quit.** Apps that update themselves (Sparkle, or the auto-updater of an Electron app) replace their own bundle while they run, so quitting them mid-update would leave a broken app behind. freshen detects the running updater, waits up to `UPDATE_TIMEOUT` seconds for the update to finish and only then quits the app. An app whose update does not finish in time is left running and is not reopened.
- **A denied Automation permission is explained, not ignored.** When macOS refuses to send Apple events (error -1743), freshen names the System Settings pane to allow: System Settings > Privacy & Security > Automation.
- **A cask that needs an administrator password is skipped when there is no terminal.** Homebrew installs a macOS installer package (`pkg`) or a `sudo` script with `sudo`, which asks for the password on the terminal. A run without a terminal (a cron job, a CI job) skips those casks and names the reason instead of waiting for a password that never comes, and the message tells you which `brew upgrade --cask` command to run by hand.
- **The terminal running freshen is never quit.** Detection uses `__CFBundleIdentifier`, `TERM_PROGRAM`, environment markers and, on Linux and Windows, the parent process chain. See [supported terminals](#supported-terminals-and-session-managers).
- **Reopening uses `open -g`**, so apps do not steal focus.
- **Apps without a Dock icon are handled too.** A menu-bar app (an agent that sets `LSUIElement`) and a login item (a helper in `Contents/Library/LoginItems`) run without an icon in the Dock, so macOS does not address them with an Apple event. freshen ends their process instead (`kill -TERM`, `kill -9` with `--force-kill`), which needs no Automation permission, and starts them again in the background: a menu-bar app with `open -g -j -b`, a login item by its path (`open -g -j -a`), the way macOS starts it at login. A login item that the app started again by itself is left alone.
- **Reopening keeps the apps the way they were.** Before an app is quit, freshen asks macOS whether the app is hidden and where its window is. A hidden app is reopened hidden (`open -g -j`), and the window of a visible app is put back where it was, so the app comes back on the display it was on (macOS decides which Space the window joins). The state is read through System Events, so macOS asks once for the permission to control it; without that permission the apps are simply reopened in the background as before.
- **`mise upgrade` is used without `--bump`** on purpose: with `node@lts` in your config, `--bump` would pin `lts` to a fixed version.
- **The Mac App Store is only updated when an account is signed in.** Installing an App Store app needs an Apple Account that is signed in to the App Store. When none is signed in, freshen skips the step (`not signed in`) instead of failing the run, and tells you to sign in with the App Store app and run `mas upgrade` by hand. The step needs [`mas`](https://github.com/mas-cli/mas) on `PATH`; `mas` 5 and newer have no `account` command, so on those versions freshen cannot ask and simply tries the upgrade. `mas` 5 and newer also install App Store apps with administrator rights, so the upgrade needs cached `sudo` credentials; without them freshen says so and points at `sudo mas upgrade`.
- **Long-running sessions are never broken silently.** If an upgrade would break a session or interrupt a running Docker build, freshen prints a warning naming what will be interrupted. See [supported session managers](#supported-terminals-and-session-managers).
- **Every run starts with preflight checks**: the network must be reachable, at least 1 GiB must be free, the battery must be at or above 20% while discharging, and the connection must not be metered or a hotspot. A failed check stops before any step runs (exit code 3) and lists every problem. Each check can be disabled with `--skip-network-check`, `--skip-disk-check`, `--skip-battery-check` or `--skip-metered-check`; `--min-free-disk-gib` and `--battery-threshold` change the thresholds. `--dry-run` and `--check` skip preflight because they change nothing.
- **Another package manager must not be running.** Before changing anything, freshen checks for Homebrew, apt/dpkg (including a held dpkg or apt lock) and winget. It waits up to 30 seconds for the other process to finish, then stops with exit code 3 and names the process and its PID. Use `--package-manager-wait <SECONDS>` to change the wait (0 aborts immediately) or `--skip-package-manager-check` to bypass the check. `--dry-run` and `--check` skip it because they change nothing.
- **Every run records a pre-upgrade snapshot** of installed versions (`brew list --versions`, `brew list --cask --versions`, `brew bundle dump`, `uv tool list`, `mise ls`, Docker images) in a timestamped file under the platform state directory (`~/.local/state/freshen/snapshots` on Linux, `~/Library/Application Support/freshen/snapshots` on macOS, `%LOCALAPPDATA%\freshen\snapshots` on Windows). Set `FRESHEN_SNAPSHOT_DIR` to override the location. The summary prints the commands that reinstall the recorded versions. Only the most recent snapshots are kept; `--snapshot-retention` changes the window (the default is 10, and `0` keeps every snapshot).
- **Destructive cleanup is opt-in.** Cache pruning, old-version pruning, `brew autoremove`, `brew cleanup --prune=all` and `docker image prune` only run with `--cleanup`. Without it freshen prints why each one was skipped. When enabled, every item a cleanup removes is echoed and the reclaimed space is reported when the tool provides it.
- **A hung step cannot hang the whole run.** `--step-timeout <SECONDS>` bounds each step (detection, check, upgrade and cleanup) and `--timeout <SECONDS>` bounds the whole run. A step that runs out of time is stopped, reported as `timed out` rather than `failed`, and anything it stopped is restored; `0` disables a timeout.
- **`--check` queries independent steps concurrently.** Up to `--jobs N` read-only queries run at the same time (the default is 4, and `0` is treated as 1). The report keeps the run order and the summary counts are deterministic regardless of which query finishes first.
- **Transient download errors are retried with exponential backoff.** A connection reset, timeout or temporary HTTP 5xx/429 response is retried up to twice before the step gives up. If the package registry is permanently unreachable (DNS failure, refused connection or no route), the step is reported as `skipped` with the reason `registry unreachable` instead of failing the run.
- **Only one update runs at a time.** A second run stops with an error naming the PID that holds the lock. The lock lives under the platform state directory (`FRESHEN_LOCK_DIR` overrides it), and `--dry-run`/`--check` do not take it.
- **Tools are asked for their output in a neutral locale.** Every command whose output freshen parses runs with `LC_ALL=C` and `LANG=C` (and, on Windows, with a UTF-8 console and without interactivity), so a machine set to another language cannot change the columns of a table or the words freshen looks for. The commands whose output is only shown to you keep the language of your machine.

### Supported terminals and session managers

Supported terminal emulators: Apple Terminal, iTerm2, Ghostty, kitty, WezTerm, Alacritty, Hyper, Warp, Tabby, Rio, Termius, Visual Studio Code, Zed, Zed Preview, Cursor, Windsurf, JetBrains, IntelliJ IDEA, IntelliJ IDEA CE, PyCharm, PyCharm CE, WebStorm, CLion, GoLand, RubyMine, PhpStorm, DataGrip, Rider, AppCode, RustRover, Fleet, Android Studio, GNOME Terminal, Konsole, Windows Terminal, Windows Console, xterm, Xfce Terminal, LXTerminal, MATE Terminal, Terminator, Tilix, urxvt, foot, GNOME Console, st.

Session managers checked before an upgrade: zellij, tmux, OpenSSH, GNU screen, byobu, dtach, abduco, mosh.

## Requirements

- macOS, Linux or Windows; a prebuilt `freshen` binary or Rust 1.81+ to build from source
- [Homebrew](https://brew.sh) — required for the Homebrew section
- Optional: `colima` + `docker`, `uv`, `mise`, `mas` (Mac App Store, macOS only)

## Usage

```bash
freshen                       # interactive: asks before quitting apps
freshen --yes                 # no prompt
freshen --force-kill          # kill apps that refuse to quit (unsaved data may be lost)
freshen --cleanup             # also prune caches, old versions and dangling images
QUIT_TIMEOUT=40 freshen       # wait longer for apps to quit
UPDATE_TIMEOUT=120 freshen    # wait longer for an app that updates itself
freshen --dry-run             # detect and plan, change nothing
freshen --check               # report outdated items, change nothing (exit 2 if updates are pending)
freshen --check --jobs 8      # query up to eight steps at the same time
freshen --json                # print the report as JSON (before/after versions per step)
freshen --markdown            # print the report as Markdown
freshen --list-steps          # show every step, its detection and skip reason
freshen --only brew,uv --skip brew-casks   # select steps
freshen --snapshot-retention 5             # keep only the five newest snapshots
freshen --step-timeout 600 --timeout 3600  # stop hung steps or a long run
freshen --skip-metered-check  # run even on a phone hotspot
freshen --min-free-disk-gib 5 --battery-threshold 30   # custom preflight thresholds
freshen --package-manager-wait 0                      # abort instead of waiting for another package manager
```

| Variable | Default | Meaning |
|---|---|---|
| `QUIT_TIMEOUT` | `20` | Seconds to wait for an app to quit |
| `UPDATE_TIMEOUT` | `60` | Seconds to wait for an app that is updating itself to finish its update |
| `FRESHEN_SKIP_PREFLIGHT` | unset | Set to `1` to disable every preflight and package-manager check |
| `FRESHEN_LOCK_DIR` | platform state directory | Override where the single-run lock file lives |

`--yes` and `--force-kill` replace the old `ASSUME_YES`/`FORCE_KILL` environment variables.

### Proxies and custom certificates

The network preflight and every update step use the same environment, so one
configuration covers the whole run:

| Variable | Meaning |
|---|---|
| `HTTPS_PROXY` / `https_proxy` | Proxy for the HTTPS preflight. Takes precedence over `HTTP_PROXY`. |
| `HTTP_PROXY` / `http_proxy` | Fallback proxy when `HTTPS_PROXY` is unset. |
| `NO_PROXY` / `no_proxy` | Comma- or space-separated hosts, domains (`.example.com`), wildcards (`*.example.com`) and optional `:port` values that bypass the proxy. `*` disables proxying. |
| `SSL_CERT_FILE`, `SSL_CERT_DIR` | Custom CA file or OpenSSL-style directory trusted by the preflight and by tools that use the platform TLS stack. |
| `CURL_CA_BUNDLE`, `REQUESTS_CA_BUNDLE`, `NIX_SSL_CERT_FILE`, `NODE_EXTRA_CA_CERTS` | Additional CA bundles trusted by the preflight. |

The preflight opens a `CONNECT` tunnel through an `http://` or `https://`
proxy and validates the target's TLS certificate; credentials in the proxy URL
are sent as `Proxy-Authorization`. A configured CA bundle that cannot be read
or parsed fails the network check instead of being ignored. Without a proxy,
or when `NO_PROXY` matches the target, the check connects directly.

Every step inherits the process environment. freshen never clears it: it only
adds the variables a tool needs (for example `HOMEBREW_NO_AUTO_UPDATE`) and
removes the few it must override, so `brew`, `uv`, `mise`, `docker`, `colima`
and the `curl`/`git` processes they start see the same proxy and certificate
variables. Export them in your shell or service manager before running
`freshen`.

The commands whose output freshen parses are the one exception: they run with
`LC_ALL=C` and `LANG=C` so that a translated table cannot break a parser (see
[Details worth knowing](#details-worth-knowing)).

## Configuration

freshen reads an optional TOML file from `~/.config/freshen/config.toml`
(macOS, Linux) or `%APPDATA%\freshen\config.toml` (Windows). Every key is
optional, and a key that is wrong is refused with an error that points at the
offending line:

```toml
# ~/.config/freshen/config.toml

[preflight]
network = false
min_free_disk_gib = 5

[run]
jobs = 8

[steps]
brew-casks = false

[exclude]
casks = ["dropbox"]

[apps]
never_quit = ["com.docker"]
```

The `[steps]` section enables or disables single steps, the `[include]` and
`[exclude]` sections choose the casks, packages, containers and compose
projects a run may touch, and the `[apps]` section names the applications that
are never quit (`never_quit`) or always quit without asking (`always_quit`).
The `NEVER_QUIT` and `ALWAYS_QUIT` environment variables hold the same app
lists.

A flag given on the command line wins over the same key in the file, and the
file wins over the built-in default. `freshen --config /path/to/config.toml`
reads a different file. The full schema is documented in
[docs/config.md](docs/config.md).

## Exit codes

The exit codes are part of the command-line contract and are stable across releases:

| Code | Meaning |
|---|---|
| `0` | Success: every step that ran completed, or `--check` found no updates |
| `1` | A step failed or timed out; also used for startup errors (invalid arguments, a held lock, an unsupported platform) |
| `2` | `--check` found updates pending |
| `3` | A preflight check or the other-package-manager check failed, so no step ran |
| `130` | The run was interrupted by SIGINT or SIGTERM |

`--help` prints the same table. Scripts can rely on these values, for example `freshen --check || [ "$?" -eq 2 ]` to detect pending updates.

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
- **`--greedy` is only used for the casks that need it.** Casks with `auto_updates true` or `version :latest` are the ones Homebrew hides without it; `--check` names the reason next to the cask. Casks with `version :latest` still show up as outdated every time — that is Homebrew behaviour, not a bug here.
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
cargo run -p freshen -- --check --jobs 8             # query up to eight steps at the same time
cargo run -p freshen -- --json                       # machine-readable report with version changes
cargo run -p freshen -- --markdown                   # Markdown report with version changes
cargo run -p freshen -- --list-steps                 # detect every step and explain skips
cargo run -p freshen -- --only brew,uv --skip casks  # select steps
cargo run -p freshen -- --yes --force-kill           # no prompts, kill stubborn apps
cargo run -p freshen -- --cleanup                    # opt in to destructive cleanup
cargo run -p freshen -- --step-timeout 600 --timeout 3600  # bound each step and the whole run
cargo run -p freshen -- --skip-network-check         # bypass one preflight check
cargo run -p freshen -- --package-manager-wait 0     # do not wait for another package manager
```

## License

MIT — see [LICENSE](LICENSE).
