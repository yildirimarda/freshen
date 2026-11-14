# Release testing checklist

Run this checklist on **macOS, Linux and Windows** before every minor release
of `freshen`. It is a **real run**: `--check` and `--dry-run` only plan, so the
checklist ends with an actual upgrade run and a rollback check.

Record the result in the sign-off table at the end and file any regression as
an issue before the release is cut.

## Before you start (all platforms)

1. Install the release candidate binary (the one that will ship), not a debug
   build from `cargo run`. The one-line installer or the GitHub Release asset
   is fine.
2. Confirm the tools you expect to be updated are installed on this machine:
   Homebrew on macOS, the distribution package manager on Linux, winget on
   Windows, plus any of `uv`, `mise`, `docker`/`colima`/etc. that you want
   exercised.
3. Make sure you are at the machine: a real run closes and reopens GUI apps.
4. Know where snapshots live (created by every mutating run):
   * macOS: `~/Library/Application Support/freshen/snapshots/`
   * Linux: `~/.local/state/freshen/snapshots/` (or `$XDG_STATE_HOME/freshen/snapshots`)
   * Windows: `%LOCALAPPDATA%\freshen\snapshots\`
5. Exit codes: `0` ok, `1` a step failed or timed out, `2` updates pending
   (`--check`), `3` a preflight check failed, `130` interrupted. See
   [Troubleshooting](troubleshooting.md).

## The release run (every platform, in this order)

### 1. Read-only check

```bash
freshen --check
```

**Expected:** nothing changes. `freshen` prints each outdated item and ends
with either `Everything is up to date.` (exit code `0`) or a list of pending
updates followed by `N update(s) pending across N step(s).` (exit code `2`).

**Verify:** the listed outdated items match what you expect on this machine.

### 2. Dry run

```bash
freshen --dry-run
```

**Expected:** freshen detects every installed step, prints what it would do,
and changes nothing. No snapshot file is created.

**Verify:** every tool you expected is shown as detected/installed, and no
step you rely on is reported as `skipped` for a reason other than "not
installed".

### 3. Real run

```bash
freshen
```

(Use `--yes` to skip the interactive app-quit confirmation.)

**Expected:** steps run in order, apps that a cask upgrade will replace are
quit and reopened, the container runtime is stopped and restarted around its
package upgrade, and the run ends with a summary like:

```
Container runtime: colima
Summary: 6 ok, 0 planned, 2 skipped, 0 failed, 0 timed out, 0 interrupted (12.4s)
  brew (Homebrew) ok (1.2s)
  brew-formulae (Homebrew formulae) ok (4.3s) - 1 change
          - node@22 22.11.0 -> 22.12.0
  brew-casks (Homebrew casks) ok (6.1s) - 2 changes
          - slack 4.41.0 -> 4.42.0
          - docker 4.34.0 -> 4.35.0
  mise (mise) ok (2.8s) - 1 change
          - node@lts 20.18.0 -> 22.12.0
  uv (uv) ok (0.9s) - skipped: nothing to do
Snapshot: /Users/you/Library/Application Support/freshen/snapshots/2024-01-15T14-30-00Z.txt
Rollback hints (previous versions from the snapshot):
  brew install node@22
  brew install --cask slack@4.41.0
  uv tool install ruff==0.5.0
```

**Verify:**

* Every step that should run reports `ok` (or `skipped: not installed` when
  the tool truly is absent).
* No step reports `failed` or `timed out`.
* Exit code is `0` (or `1` only when a step genuinely failed on this machine).
* The quit/reopen behaviour works: the apps a cask replaces are back after the
  run, and the terminal that runs freshen is still running.
* A snapshot file is created and the rollback hints name the previous
  versions.

## Rollback check

The pre-upgrade snapshot is the rollback safety net. On every platform:

1. Open the newest snapshot file in the snapshots directory above.
2. Confirm it contains a header (`# freshen pre-upgrade snapshot`) and one
   section per package manager that ran (`brew`, `uv`, `mise`, `chocolatey`,
   …) with the commands that were captured.
3. Confirm the run summary printed `Rollback hints` and that each hint names
   the previous version, e.g.:
   ```sh
   brew install node@22
   brew install --cask slack@4.41.0
   uv tool install ruff==0.5.0
   mise install node@lts@20.18.0
   choco upgrade git --version 2.45.0
   docker pull image@sha256:...
   ```
4. Pick **one non-critical package** from a recorded section and run its rollback
   command by hand to confirm it reinstalls the recorded previous version.
   Do not roll back packages you still need at the current version.
5. Confirm `freshen history` lists the run you just made (from the snapshots).

The snapshot is also what lets you undo a bad release: re-running the rollback
hints returns every recorded package to its pre-release version.

## macOS checklist

| Item | What to verify |
|---|---|
| Homebrew detected | `freshen --list-steps` shows `brew`/`brew-formulae`/`brew-casks` as `installed` (not `not installed`) |
| Both architectures (Apple Silicon + Intel Rosetta) | if both `/opt/homebrew` and `/usr/local` exist, each is discovered and deduplicated; the prefix is named in output |
| Formulae upgrade | `brew-formulae` reports `ok` with `name old -> new` changes |
| Cask upgrade quits/reopens apps | `brew-casks` reports `ok`; the apps it replaces are back after the run; the terminal running freshen is still running |
| App never-quit list | apps in `never_quit` (or `NEVER_QUIT`) are left running and their cask is still upgraded |
| macOS system updates | `softwareupdate --list` output is reported, never installed by default |
| Mac App Store | `mas outdated`/`mas upgrade` runs when signed in; skipped with `not signed in` otherwise |
| Container runtime | Colima / OrbStack / Docker Desktop / Rancher Desktop / Podman detected; stopped before its package upgrades and started again afterwards |
| Snapshot location | `~/Library/Application Support/freshen/snapshots/<timestamp>.txt` |
| Rollback hints | `brew install …`, `brew install --cask …@…`, `uv tool install …==…`, `mise install …@…` |
| Exit code | `0` on a clean run |

Suggested command:

```bash
freshen --yes
```

then the rollback check above.

## Linux checklist

| Item | What to verify |
|---|---|
| Distro package manager | apt/dnf/pacman/zypper/apk detected; the matching step runs (`apt-get update` → `apt-get upgrade -y`, `dnf upgrade -y`, `pacman -Syu --noconfirm`, …) |
| Step skipped when absent | a package manager that is not installed reports `skipped: not installed`, not `failed` |
| Reboot needed | `needrestart` or `/var/run/reboot-required` is reported when a restart is pending |
| Services needing restart | listed in the summary after an upgrade |
| uv / mise | report `ok` with version changes, or `skipped: not installed` |
| Flatpak / Snap | run when installed, skipped when absent |
| Homebrew on Linux | detected at `/home/linuxbrew/.linuxbrew`; cask handling is skipped instead of reporting missing cask support |
| Container runtime | detected and stopped/restarted around its package upgrade |
| Snapshot location | `~/.local/state/freshen/snapshots/<timestamp>.txt` |
| Rollback hints | `apt`/`dnf`/`pacman` entries in the snapshot file plus `uv tool install …==…` and `mise install …@…` |
| Exit code | `0` on a clean run |

Suggested command:

```bash
freshen --yes
```

then the rollback check above.

## Windows checklist

| Item | What to verify |
|---|---|
| winget | `winget source update` runs, then `winget upgrade --all --silent --accept-*`; exit codes map to ok / skipped / failed |
| Chocolatey | requires administrator rights; an un-elevated run reports `skipped: needs administrator`; an elevated run upgrades Chocolatey itself first, then all packages |
| Scoop | `scoop update` + `scoop update *`; `scoop cleanup *` and `scoop cache rm *` only with `--cleanup` |
| Microsoft Store | app updates reported where a Store command line or API is available |
| Windows Update | reported through PSWindowsUpdate (or `usoclient` fallback); a pending reboot is reported |
| `wsl --update` | opt-in: freshen warns and lists running distributions first; requires confirmation or `--yes` |
| Executable resolution | `uv`/`mise` found through `PATHEXT` as `uv.exe`/`mise.exe` |
| Container runtime | Docker Desktop (WSL2 backend) or Podman Desktop detected and stopped/restarted around its package upgrade |
| Administrator elevation | steps that need elevation are marked; an un-elevated run skips them instead of failing |
| Snapshot location | `%LOCALAPPDATA%\freshen\snapshots\<timestamp>.txt` |
| Rollback hints | `choco upgrade <name> --version <version>` for Chocolatey; `uv tool install …==…` / `mise install …@…` for uv/mise |
| Exit code | `0` on a clean run |

Suggested command (elevated terminal when Chocolatey should run):

```powershell
freshen --yes
```

then the rollback check above.

## Sign-off

| Platform | Tester | Date | freshen version | Exit code | Result | Notes |
|---|---|---|---|---|---|---|
| macOS | | | | | ☐ pass ☐ fail | |
| Linux | | | | | ☐ pass ☐ fail | |
| Windows | | | | | ☐ pass ☐ fail | |

Only merge the minor release when all three rows are `pass` and any failures
have been fixed and re-tested.
