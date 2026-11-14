# Frequently asked questions

## Installation and first run

### How do I install freshen?

The one-line installer detects your operating system and architecture, downloads
the matching prebuilt binary, verifies its SHA-256 checksum before installing
anything, and installs `freshen` to `/usr/local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/yildirimarda/freshen/main/install.sh | sh
```

You can also install with Homebrew, Scoop, winget, Chocolatey, the AUR
PKGBUILD, the Nix flake, or `cargo binstall freshen`. See
[Quick start](quick-start.md).

### What does the first run do?

Run `freshen --dry-run` first: it detects every installed tool, plans what
would change, and changes nothing. Then run `freshen` interactively — it asks
before quitting apps that a cask upgrade will replace.

## Steps and platforms

### Which steps run on my platform?

Every step declares the platforms it supports (`macos`, `linux`, `windows`).
A step is skipped when its tool is not installed or when it does not run on the
current platform. `freshen --list-steps` shows every known step, whether it was
detected on this machine, and why it would be skipped.

Platform-specific behaviour is documented in the [platform notes]:
[macOS](platform-notes/macos.md), [Linux](platform-notes/linux.md),
[Windows](platform-notes/windows.md).

### Can I disable a step?

Yes. Add the step id to `[steps]` in your configuration file:

```toml
[steps]
brew-casks = false
```

A disabled step is reported as skipped with the reason "disabled in the
configuration file". Disabling a step also skips any steps that declare it as a
prerequisite. Step ids come from `freshen --list-steps`.

### Can I run only some steps?

Yes: `freshen --only brew,uv` runs only the named steps. `freshen --skip
brew-casks` skips the named steps.

## Check, update and cleanup

### What is the difference between `--check` and a normal run?

`--check` queries every detected step read-only and prints what is outdated
(names, current → new version) without changing anything. It exits with code
`2` when updates are pending and `0` when everything is up to date. A normal
run actually performs the upgrades.

### What does `--cleanup` do?

`--cleanup` opts in to destructive cleanup: pruning caches, old versions and
dangling container images. It never runs unless you pass the flag.

### Can I undo an upgrade?

Before a run, freshen records installed versions per step to a timestamped
snapshot. The run summary prints rollback hints — the exact commands to return
to the previous versions recorded in the snapshot. Old snapshots are pruned
with a configurable retention count (default keeps a small recent window).

## Output and exit codes

### What do the step statuses mean?

| Status | Meaning |
|---|---|
| `ok` | The step completed successfully |
| `skipped` | The tool is not installed, the step is disabled, or a prerequisite is missing |
| `failed` | The step ran but did not complete |
| `timed out` | The step ran out of time (`--step-timeout`) — distinct from `failed` |

A failing step never aborts the run. Failures are collected and listed in the
final summary, and the run exits with code `1`.

### What are the exit codes?

| Code | Meaning |
|---|---|
| `0` | Everything completed, or `--check` found no updates |
| `1` | A step failed or timed out; also startup errors |
| `2` | `--check` found updates pending |
| `3` | A preflight check failed, so no step ran |
| `130` | The run was interrupted (SIGINT/SIGTERM) |

## Configuration

### Where is the configuration file?

| Platform | Location |
|---|---|
| macOS, Linux | `$XDG_CONFIG_HOME/freshen/config.toml`, else `~/.config/freshen/config.toml` |
| Windows | `%APPDATA%\freshen\config.toml` |

The file is optional. A different file can be used with `freshen --config
/path/to/config.toml`. See [Configuration reference](config.md) for the full
schema.

### Does freshen collect telemetry?

No. freshen collects no telemetry and sends no data itself. Only the tools it
runs (and the optional self-update check) use the network. The self-update
check can be disabled.
