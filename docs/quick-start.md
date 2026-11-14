# Quick start

Install freshen, run it for the first time, and understand what it does.

## 1. Install freshen

The one-line installer detects your operating system and architecture, downloads the matching prebuilt binary, verifies its SHA-256 checksum against the release's `checksums.txt` **before** installing anything, and installs `freshen` to `/usr/local/bin` (override with `FRESHEN_BIN_DIR`):

```bash
curl -fsSL https://raw.githubusercontent.com/yildirimarda/freshen/main/install.sh | sh
```

Install a specific release with `FRESHEN_VERSION` (defaults to `latest`):

```bash
FRESHEN_VERSION=0.1.0 sh install.sh
```

You can also install with Homebrew (`brew install yildirimarda/homebrew-tap/freshen`), Scoop, winget, Chocolatey, the AUR PKGBUILD, the Nix flake, or `cargo binstall freshen`.

Verify the installation:

```bash
freshen --help
```

## 2. Run a dry run first

A dry run detects every installed tool, plans what would change, and changes nothing:

```bash
freshen --dry-run
```

Use this before the first real run to see which steps freshen will run and which it will skip because the tool is not installed.

## 3. Check what is outdated

`--check` queries every detected step read-only and prints what is outdated (names, current → new version) without changing anything. It exits with code `2` when updates are pending:

```bash
freshen --check
```

Query up to eight steps at the same time:

```bash
freshen --check --jobs 8
```

Get a machine-readable report:

```bash
freshen --check --json
```

## 4. Run the update

Run freshen interactively — it asks before quitting apps that a cask upgrade will replace:

```bash
freshen
```

Skip the confirmation prompt:

```bash
freshen --yes
```

Also prune caches, old versions and dangling images (destructive, opt-in):

```bash
freshen --cleanup
```

Kill apps that refuse to quit within the timeout (unsaved data may be lost):

```bash
freshen --force-kill
```

## 5. Understand the output

Every step reports one of these statuses:

| Status | Meaning |
|---|---|
| `ok` | The step completed successfully |
| `skipped` | The tool is not installed, the step is disabled, or a prerequisite is missing |
| `failed` | The step ran but did not complete (exit code 1 overall) |
| `timed out` | The step ran out of time (`--step-timeout`) — distinct from `failed` |

A failing step never aborts the run. All failures are collected and listed in the final summary, and the run exits with code `1`.

Exit codes:

| Code | Meaning |
|---|---|
| `0` | Everything completed, or `--check` found no updates |
| `1` | A step failed or timed out; also startup errors (invalid arguments, held lock) |
| `2` | `--check` found updates pending |
| `3` | A preflight check failed, so no step ran |
| `130` | The run was interrupted (SIGINT/SIGTERM) |

## 6. Create a configuration file (optional)

freshen reads an optional TOML file from:

| Platform | Location |
|---|---|
| macOS, Linux | `$XDG_CONFIG_HOME/freshen/config.toml`, else `~/.config/freshen/config.toml` |
| Windows | `%APPDATA%\freshen\config.toml` |

Every section and key is optional. A file that sets a single value is valid:

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

A different file can be used with `freshen --config /path/to/config.toml`.

See the [Configuration reference](config.md) for the full schema, validation errors and environment variable overrides.

## 7. Common flags

```bash
freshen --list-steps          # show every step, its detection and skip reason
freshen --only brew,uv        # run only the named steps
freshen --skip brew-casks     # skip the named steps
freshen --dry-run             # detect and plan, change nothing
freshen --check               # report outdated items, change nothing
freshen --json                # print the report as JSON
freshen --markdown            # print the report as Markdown
freshen --step-timeout 600    # bound each step (0 disables)
freshen --timeout 3600        # bound the whole run (0 disables)
freshen --skip-metered-check  # run even on a phone hotspot
freshen --min-free-disk-gib 5 # custom free-disk-space threshold
freshen --snapshot-retention 5  # keep only the five newest snapshots
```

## Next step

Read the [Configuration reference](config.md) for the full `config.toml` schema.
