# Windows notes

freshen on Windows updates Windows packages with winget, Chocolatey and Scoop,
Microsoft Store apps, Windows Update, WSL and container images — in the right
order, without babysitting.

## Package managers

| Step | Tool | Notes |
|---|---|---|
| `winget` | winget | `winget source update` refreshes the index, then `winget upgrade --all --silent --accept-*` upgrades everything. Exit codes map to ok / skipped / failed. |
| `chocolatey` | Chocolatey | Requires administrator rights. `choco outdated --limit-output` reports pending updates (pinned packages are held back). Chocolatey itself is upgraded first, then all packages. |
| `scoop` | Scoop | `scoop update` + `scoop update *`; `scoop cleanup *` and `scoop cache rm *` run only with `--cleanup`. |
| `microsoft-store` | Microsoft Store | App updates where a Store command line or API is available. |

Executable names are resolved through `PATHEXT`, so `uv` and `mise` are found as
`uv.exe` and `mise.exe`.

## Administrator elevation

freshen detects administrator rights and marks the steps that need elevation.
An un-elevated run skips elevated steps with a "needs administrator" reason
instead of failing. Elevation for a single step is requested through one UAC
prompt only when that step is selected.

## Windows Update and WSL

Windows Update is reported through the PSWindowsUpdate module when installed,
falling back to `usoclient`. A pending reboot is reported in the summary and
reports. `wsl --update` is an opt-in step; it shuts down every running WSL
distribution, so freshen warns and lists the running distributions first,
requiring confirmation or `--yes`.

Linux steps can also run inside a selected WSL distribution.

## Shells

PowerShell 7 is chosen when it exists and falls back to Windows PowerShell 5.1
otherwise. PowerShell-based steps run with a process-scoped execution policy
(`-NoProfile -ExecutionPolicy Bypass` only for freshen's own invocation); the
machine policy is never changed. `cmd.exe` shims are supported as launching
shells with `cmd`-compatible quoting.

## Configuration file

| Location | Value |
|---|---|
| Config | `%APPDATA%\freshen\config.toml` |
| Trusted commands | `trusted.toml` next to the configuration file |

See [Configuration reference](../config.md) for the full schema.
