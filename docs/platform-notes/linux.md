# Linux notes

freshen on Linux updates the distribution's packages, container images, uv and
mise tools, Flatpak, Snap and more — in the right order, without babysitting.

## Distribution package manager

freshen detects the distribution's package manager and runs the matching step,
skipping it when the tool is not installed:

| Distribution | Step | Commands |
|---|---|---|
| Debian / Ubuntu | `apt` | `apt-get update` → `apt-get upgrade -y` |
| Fedora | `dnf` | `dnf upgrade -y` |
| RHEL / older Fedora | `yum` | `yum upgrade -y` (fallback when dnf is absent) |
| Arch | `pacman` | `pacman -Syu --noconfirm` |
| openSUSE | `zypper` | `zypper refresh` → `zypper update -y` |
| Alpine | `apk` | `apk update` → `apk upgrade` (opt-in) |

With `--cleanup`, the step also removes unneeded packages: `apt-get
autoremove -y` + `autoclean`, `dnf autoremove -y`, `pacman` orphan removal,
`flatpak uninstall --unused`, `snap` cleanup, etc.

## Reboot and services

After an upgrade, freshen reports whether a restart is needed (`needrestart` or
`/var/run/reboot-required`) and lists services that need a restart
(`systemctl`, `needrestart`). An opt-in flag can restart the detected services,
restoring their previous enabled/running state even on error or interrupt.

## Homebrew on Linux

Homebrew is also detected at `/home/linuxbrew/.linuxbrew` in addition to the
macOS prefixes. On Linux, Homebrew has no casks, so cask handling is skipped
instead of reporting missing cask support.

## Flatpak, Snap and more

`flatpak update -y` runs when Flatpak is installed; `snap refresh` runs when
Snap is installed. Both are skipped when absent. Optional steps include Nix
(`home-manager switch`), firmware updates through `fwupdmgr` (report-only by
default), emerge (Gentoo), AUR helpers (`yay`/`paru`) and Termux on Android.

## Desktop applications

A running desktop application detected from its `.desktop` entry is stopped with
`kill -TERM` (then force-killed after a timeout) before its package is upgraded,
and restarted with `gtk-launch` afterwards.

## Configuration file

| Location | Value |
|---|---|
| Config | `$XDG_CONFIG_HOME/freshen/config.toml`, else `~/.config/freshen/config.toml` |
| Trusted commands | `trusted.toml` next to the configuration file |

See [Configuration reference](../config.md) for the full schema.
