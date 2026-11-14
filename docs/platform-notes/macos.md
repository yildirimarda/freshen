# macOS notes

freshen on macOS updates Homebrew formulae and casks, container images, uv and
mise tools, Mac App Store apps and macOS system updates — in the right order,
without babysitting.

## Homebrew

Homebrew is detected at its standard prefix: `/opt/homebrew` on Apple Silicon
and `/usr/local` on Intel. Both prefixes are found and deduplicated, so a run
works whichever one (or both) is on `PATH`.

Cask upgrades quit the running apps a cask will replace, then reopen them
afterwards. The terminal that runs freshen is never quit, and apps on the
`never_quit` list are left running. See [Configuration reference](../config.md) for
the `[apps]` section.

With `--cleanup`, `brew autoremove` and `brew cleanup --prune=all` run before
the apps are reopened. `brew doctor` and `brew bundle cleanup` are report-only:
they print what they found and change nothing.

## Mac App Store

The `mas` step lists outdated App Store apps with `mas outdated` and installs
them with `mas upgrade`. It is macOS only and is skipped when no Apple Account
is signed in.

## macOS system updates

`softwareupdate --list` lists the macOS updates that are available. They are
reported, never installed by default; pass `--install-macos-updates` to install
them. The Xcode Command Line Tools are installed when macOS offers them,
because Homebrew needs them.

## Container runtimes

The active container runtime is detected from `DOCKER_HOST`, `docker context
show` or well-known socket paths: Colima, OrbStack, Docker Desktop, Rancher
Desktop, Podman or native Docker Engine. It is stopped before its package is
upgraded and started again afterwards, bringing compose projects and standalone
containers back. Override the detected runtime with `--container-runtime`.

## Configuration file

| Location | Value |
|---|---|
| Config | `$XDG_CONFIG_HOME/freshen/config.toml`, else `~/.config/freshen/config.toml` |
| Trusted commands | `trusted.toml` next to the configuration file |

See [Configuration reference](../config.md) for the full schema.
