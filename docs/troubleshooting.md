# Troubleshooting

This page lists the most common problems when running freshen, what they mean, and how to fix them. If your problem is not covered here, run `freshen --json` (or `--markdown`) and include the output when you [file an issue](https://github.com/yildirimarda/freshen/issues/new).

## The run does nothing — no steps were executed

### freshen says "command not found"

The `freshen` binary is not on your `PATH`. Re-run the installer, which installs to `/usr/local/bin` on macOS/Linux, or check that your package manager's bin directory is on `PATH` (Homebrew, Scoop, winget, Chocolatey). Restart your terminal after installing.

### Nothing happens and there is no output

freshen prints progress to stderr and a summary at the end. When you select `--json` or `--markdown`, the structured report goes to stdout and only human progress goes to stderr — make sure you are looking at both streams.

Run with `-v` / `--verbose` for more detail about what freshen is doing step by step.

### A step I expected is missing

Run `freshen --list-steps`. Each step shows:

- whether it was detected on this machine,
- whether it is enabled in your config file,
- its skip reason when it will not run.

A step is skipped when its tool is not installed, when it does not run on the current platform (`macos`, `linux`, `windows`), when it is disabled in `[steps]`, when one of its prerequisites is missing or disabled, or when it conflicts with another selected step. See [Configuration reference](config.md) for how to enable or disable steps.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Everything completed, or `--check` found no updates |
| `1` | A step failed or timed out; also startup errors (invalid arguments, held lock) |
| `2` | `--check` found updates pending |
| `3` | A preflight check failed, so no step ran |
| `130` | The run was interrupted (SIGINT/SIGTERM) |

A failing step never aborts the run. Failures are collected and listed in the final summary, and the run exits with code `1`.

## Step statuses

| Status | Meaning |
|---|---|
| `ok` | The step completed successfully |
| `skipped` | The tool is not installed, the step is disabled, or a prerequisite is missing |
| `failed` | The step ran but did not complete |
| `timed out` | The step ran out of time (`--step-timeout`) — distinct from `failed` |

## Preflight check failed (exit code 3)

Before any step runs, freshen checks that the network is reachable, that enough disk space is free, that the machine is not on battery below a threshold, and that the connection is not metered. Each check is skippable with a flag:

```bash
freshen --skip-network-check          # run even when the network is unreachable
freshen --skip-metered-check          # run even on a phone hotspot
freshen --min-free-disk-gib 5         # custom free-disk-space threshold
```

If a preflight check fails, no step runs. Fix the underlying problem (reconnect to the network, free disk space, plug in the power supply) or pass the corresponding skip flag.

## Lock file: another freshen run is in progress

freshen takes a lock file so two runs cannot overlap. If a previous run crashed while holding the lock, a new run fails with a message naming the PID that holds it.

Wait for the other run to finish, then try again. If you are sure no other freshen process is running, delete the stale lock file (freshen prints its path in the error message) and re-run.

## Automation denied when quitting apps (macOS)

When freshen quits an app through AppleScript, macOS may deny Automation permission (error `-1743`). The app is then treated as unquittable and the cask upgrade that needs it quit is skipped.

Grant Automation permission in **System Settings → Privacy & Security → Automation** for the terminal application that runs freshen (for example `Terminal`, `iTerm2` or `Ghostty`), then re-run. The step's error message names the terminal application so you know which entry to switch on.

## Windows: administrator rights needed

Some steps (Chocolatey, winget machine-wide installs, Windows Update) require elevation. When freshen is not running elevated, those steps are skipped with a "needs administrator" reason instead of failing.

Re-run from an elevated terminal (PowerShell or cmd.exe launched as Administrator). freshen requests elevation for a single step only when that step is selected and elevation is available.

## Winget, Chocolatey and Scoop

### winget reports no updates but I know one is pending

Run `freshen --check` which runs `winget source update` before the upgrade so the package index is current. If the index is stale, winget may not see available updates.

### Chocolatey upgrade fails midway

freshen upgrades Chocolatey itself first, then all packages, and records per-package failures in the summary. Re-run `freshen` — Chocolatey steps are idempotent, so a second run only touches packages that still need upgrading.

Pinned packages (`choco pin`) are reported as held back and are not upgraded.

## Roll back an upgrade

Before a run, freshen records installed versions per step to a timestamped snapshot. The run summary prints rollback hints — the exact commands to return to the previous versions recorded in the snapshot.

Snapshots live in the data directory (next to the config file) and are pruned with a configurable retention count (default keeps a small recent window). Use `--snapshot-retention N` to keep more or fewer.

## Docker, Colima, OrbStack and containers

If a container step fails, check that the runtime is running (`docker context show`, `orbctl status`, `rdctl status`). freshen stops the runtime before upgrading its package and restarts it afterwards, bringing compose projects and standalone containers back.

With `--cleanup`, freshen prunes dangling images (`docker image prune -f`). This never runs unless you pass the flag.

## Homebrew: doctor and bundle cleanup

`brew doctor` and `brew bundle cleanup` run as report-only checks: they print what they found and change nothing. If `brew doctor` reports problems, fix them (for example untapped formulae, broken symlinks or outdated Xcode Command Line Tools) and re-run freshen.

## Get more help

- [Quick start](quick-start.md) — install and run freshen for the first time
- [Configuration reference](config.md) — every `config.toml` key and how to use it
- [FAQ](faq.md) — frequently asked questions
- [Platform notes](platform-notes/macos.md) — platform-specific behaviour (macOS, Linux, Windows)
- [File an issue](https://github.com/yildirimarda/freshen/issues/new) — include the output of `freshen --json`
