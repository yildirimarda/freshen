# freshen documentation

One command to keep a developer machine current: **Homebrew**, **container images** (Colima, OrbStack, Docker Desktop, Rancher Desktop, Podman or native Docker Engine), **uv tools**, **mise** (Node & friends), **Mac App Store apps** and **macOS system updates** — in the right order, without babysitting.

```bash
freshen
```

## Start here

| Page | What you will find |
|---|---|
| [Quick start](quick-start.md) | Install freshen, run it for the first time, and understand the output |
| [Configuration reference](config.md) | The `config.toml` file: where it lives, every key, validation errors, environment variable overrides |

## What freshen does

Every step is optional — if a tool is not installed, its section is skipped, and a failing step never aborts the run; failures are collected and listed at the end (exit code 1).

| Step | Commands |
|---|---|
| Homebrew | `brew update` → `brew upgrade --formula` → quit running apps → `brew upgrade --cask` (with `--greedy` only for casks that need it) → reopen the apps it quit. With `--cleanup`: `brew autoremove` → `brew cleanup --prune=all` before reopening. `brew doctor` and `brew bundle cleanup` report what they found and change nothing |
| Containers | Detects the active runtime (Colima, OrbStack, Docker Desktop, Rancher Desktop, Podman or native Docker Engine) from `DOCKER_HOST`, `docker context show` or well-known sockets; stops it before upgrading its package and restarts it afterwards, bringing compose projects and standalone containers back. With `--cleanup`: `docker image prune -f`. Override with `--container-runtime` |
| uv | `uv tool upgrade --all`. With `--cleanup`: `uv cache prune` |
| mise | `mise self-update` (only if mise was not installed via brew) → `mise upgrade`, plus a warning for processes still running on an old Node, Python, Java or Go version, naming the app that started them. With `--cleanup`: `mise prune -y` |
| Mac App Store | `mas outdated` to list the App Store apps with a pending update → `mas upgrade` to install them. macOS only, and skipped when no Apple Account is signed in |
| macOS system updates | `softwareupdate --list` to list the macOS updates that are available. They are only reported, never installed; `--install-macos-updates` installs them. The Xcode Command Line Tools are installed when macOS offers them, because Homebrew needs them. macOS only |

## Installation

freshen is distributed as prebuilt binaries for macOS, Linux and Windows, with a SHA-256 checksums file attached to every GitHub Release. The quickest way to install is the one-line installer:

```bash
curl -fsSL https://raw.githubusercontent.com/yildirimarda/freshen/main/install.sh | sh
```

Install a specific release with `FRESHEN_VERSION` (defaults to `latest`):

```bash
FRESHEN_VERSION=0.1.0 sh install.sh
```

You can also install with Homebrew (`brew install yildirimarda/homebrew-tap/freshen`), Scoop, winget, Chocolatey, the AUR PKGBUILD, the Nix flake, or `cargo binstall freshen`.

## Next steps

* [Quick start](quick-start.md) — install and run freshen for the first time
* [Configuration reference](config.md) — every `config.toml` key and how to use it
* [PLAN.md](https://github.com/yildirimarda/freshen/blob/main/PLAN.md) — the project roadmap
* [CONTRIBUTING.md](https://github.com/yildirimarda/freshen/blob/main/CONTRIBUTING.md) — how to contribute
