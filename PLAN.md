# freshen Project Plan

<!--
  `./run.sh` (agentloop) takes the first unchecked item. Keep the format exact:
    · headings:  ## Milestone N: name
    · todo:      - [ ] text
    · done:      - [x] text
  One pull request per item, independently testable, ordered by dependency.
-->

## Milestone 0: Foundation

- [x] Single zsh script `update-all.sh` with failure collection and a final summary
- [x] Create base repository structure (README, LICENSE, CONTRIBUTING, CHANGELOG, .gitignore)
- [x] Set up CI: zsh syntax check, workflow YAML validity
- [x] Add `docs/` (index, ADR template) and `tests/` (`run-tests.sh` checking zsh syntax and PLAN.md format)
- [x] Set up release-please for versioned releases from Conventional Commits
- [x] Create AGENTS.md / CLAUDE.md with project guidelines for autonomous agents
- [x] Choose the project name `freshen` (cross-platform, Rust) and rename the repository

## Milestone 1: Current Features

- [x] Homebrew: update, upgrade formulae, upgrade casks with `--greedy`, autoremove, `cleanup --prune=all`
- [x] Quit running apps that a cask upgrade will replace (graceful AppleScript quit with timeout, optional `FORCE_KILL=1`), then reopen them with `open -g`
- [x] Never quit the terminal that runs the script (`__CFBundleIdentifier`)
- [x] Interactive confirmation before quitting apps, `ASSUME_YES=1` to skip
- [x] Colima/lima upgrade handling: compose down, stop standalone containers, stop colima, upgrade, start, bring everything back up
- [x] Docker: `compose pull` + `up -d` for running compose projects, then `image prune -f`
- [x] uv: `tool upgrade --all` and `cache prune`
- [x] mise: self-update (non-brew installs), `upgrade` (no `--bump` so `node@lts` stays), `prune -y`
- [x] Upgrade formulae first and casks afterwards, so apps stay open during the slow formula upgrade
- [x] Detect which apps a cask will replace by reading `brew info --cask --json=v2` (`app` artifacts, including `{"target": ...}` forms) in `/Applications` and `~/Applications`
- [x] Detect a running app by its main executable (`Contents/MacOS/`) instead of AppleScript "is running", so helper/agent processes sharing the bundle id do not cause false positives
- [x] Skip apps that are not installed or have no bundle id; record the bundle ids of apps that were quit so exactly those are reopened
- [x] Warn about processes still running on an old Node version that block `mise prune`
- [x] Warn when zellij is upgraded from inside a zellij session

## Milestone 2: Cross-Platform Core

Goal: one tool for macOS, Linux and Windows, written in Rust and shipped as a single binary. This milestone replaces the zsh script as the implementation while keeping its behaviour (parity tests). The zsh script stays as-is until then; no further features are added to it.

- [x] Write ADR 0001 in `docs/adr/` recording the decision to implement in Rust (single static binary, no runtime to install, first-class Windows/macOS/Linux support, strong subprocess and signal handling, easy cross-compilation) and the alternatives considered (Go, Python via `uv tool install`) with the reasons they were not chosen
- [x] Scaffold a Cargo workspace (library crate with all logic, thin binary crate `freshen`), pin the toolchain with `rust-toolchain.toml`, set the MSRV, and wire `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test` into CI and the AGENTS.md project reference
- [x] Choose and document the core dependencies in the ADR (candidates: `clap` for the CLI, `serde` + `toml` for config, `anyhow`/`thiserror` for errors, `ctrlc` for signals, `which` for tool detection, `indicatif` for progress) and add `cargo-deny` (licenses, advisories) to CI
- [x] Define the step abstraction: each step has an id, display name, `detect()` (is the tool installed), `check()` (what is outdated, read-only), `run()` (upgrade), `cleanup()` and a list of conflicts/prerequisites
- [x] Implement a step registry with ordering, dependencies (e.g. brew formulae before casks, mise after brew) and per-step platform filter (`macos`, `linux`, `windows`)
- [x] Implement a subprocess runner with live output streaming, per-step timeout, environment control, optional retry with backoff, and capture of stdout/stderr for the log
- [x] Implement an OS abstraction layer: detect OS/distro/arch, privilege level, interactive vs non-interactive terminal, and the terminal application running the tool
- [x] Implement failure collection and the final summary (steps ok / skipped / failed, duration per step) with the same exit-code semantics as the zsh script
- [x] Add `--dry-run`, `--only`, `--skip`, `--yes`, `--force-kill` with identical semantics on every platform
- [x] Add a test harness that runs the built binary against stub `brew`, `uv`, `mise`, `docker`, `colima` executables on `PATH` and asserts the call order (tests never invoke the real tools)
- [x] Add a test that a failing step is recorded, does not abort the run, and produces exit code 1 with the step listed in the summary
- [x] Add `--list-steps` showing every known step, whether it was detected on this machine, and why it would be skipped
- [x] Port the Homebrew steps (including formulae-before-casks ordering and cask artifact detection) with parity tests against recorded command transcripts
- [x] Port the app quit/reopen logic (graceful quit, timeout, optional force kill, never quit the host terminal) with parity tests
- [x] Port the Colima/lima/Docker orchestration with parity tests
- [x] Port the uv and mise steps with parity tests
- [x] Port the warnings: processes still running on an old Node version (with the app that started them) and zellij upgraded from inside a zellij session
- [x] Remove the zsh implementation once parity tests pass and document the migration in README and CHANGELOG
- [x] Add a CI job that checks the code compiles against the MSRV (`rust-version` in the workspace manifest) with `cargo check --workspace`, so an accidental use of a newer language feature fails before release
- [x] Validate the step graph in the registry: unknown prerequisite/conflict ids and dependency cycles fail with a clear message, covered by tests
- [x] Enforce declared step conflicts when building a run: when two conflicting steps are available on the platform, skip the later one and report it as skipped with the reason, covered by tests
- [x] Detect duplicate step ids in the registry and fail with a clear message instead of letting one shadow the other during dependency resolution, covered by tests
- [x] Detect WSL and container environments in the OS abstraction layer (for example `/proc/version`, `/.dockerenv`, `WSL_DISTRO_NAME`) so Linux steps can adapt or skip, covered by tests with injected probe facts
- [x] Fall back to walking the parent process chain when no environment marker identifies the terminal application, so the host terminal is still known on Linux and Windows; cover it with a fake process table
- [x] Extend the stub harness so each stub can return canned stdout, stderr and exit status per invocation, so ported steps can be tested against recorded command transcripts
- [x] Extend the stub harness with scripted stdin so the interactive quit prompt (yes / skip quitting / skip casks) is covered end to end
- [x] Remove the deprecated `update-all.sh` compatibility shim and replace the zsh syntax step in `.github/workflows/ci.yml` with a check for the shim (requires a human to apply the workflow change, since agents cannot edit `.github/workflows/`)
- [x] Make the Rust test targets compile on the MSRV and extend the MSRV CI job to `cargo check --workspace --all-targets`; the runner tests currently compare `ExitCode` values, which only implements `PartialEq` after 1.81
- [x] Show conflict skips in `--list-steps` so the listing names the same reason the run summary reports when two selected steps conflict, covered by tests

## Milestone 3: Safety and Reliability

The "deep detail" that sets the tool apart: updates should be predictable, reversible and never leave the machine half-changed.

- [x] `--check` mode: query every detected step read-only and print what is outdated (names, current -> new version) without changing anything; exit code 0 when up to date, 2 when updates are pending
- [x] Before/after version diff in the summary for every step (what actually changed), with `--json` and `--markdown` report output
- [x] Pre-upgrade snapshot: record installed versions per step (brew list --versions, brew bundle dump, uv tool list, mise ls, package lists) to a timestamped file so a rollback is possible
- [x] Rollback hints in the summary: print the exact commands to return to the previous versions recorded in the snapshot
- [ ] Preflight checks: network reachable, enough free disk space, not on battery below a threshold, not on a metered/hotspot connection (configurable, each skippable with a flag)
- [ ] Add a lock file so two freshen runs cannot overlap, with a clear message naming the PID that holds it
- [ ] Detect when another package manager process is running (brew, apt/dpkg lock, winget) and wait or abort with a clear message
- [ ] Graceful handling of SIGINT/SIGTERM on every OS: stop the running step, restore everything that was stopped (apps, containers, VMs), print a partial summary
- [ ] Idempotency guarantee: running the tool twice in a row must be a no-op the second time; add a test asserting it
- [ ] Per-step and global timeouts with a clear "timed out" status distinct from "failed"
- [ ] Network failure handling: retry transient download errors with exponential backoff, skip the step if the registry is unreachable
- [ ] Detect processes running from a tool version that is about to be removed (old Node, Python, Java, Go toolchains) and report them with the parent application that started them
- [ ] Detect long-running interactive sessions that an upgrade would break (zellij, tmux server, SSH sessions, running docker builds) and warn or defer
- [ ] Never run destructive cleanup (volume prune, cache wipe) unless explicitly enabled; log everything a cleanup removed and the space reclaimed
- [ ] Exit codes documented and stable: 0 ok, 1 step failure, 2 updates pending (check mode), 3 preflight failed, 130 interrupted
- [ ] Kill the whole process tree on timeout or interrupt (process group on Unix, job object on Windows) so grandchildren that keep output pipes open cannot outlive the step, covered by a test using a shell that spawns a child
- [ ] Prune old pre-upgrade snapshots with a configurable retention count (default keeps a small recent window) so automatic snapshots do not grow without bound, covered by tests
- [ ] Record cask versions in the pre-upgrade snapshot (`brew list --cask --versions`) so rollback hints can cover upgraded casks, covered by tests

## Milestone 4: macOS Depth

- [ ] Handle `version :latest` casks and `auto_updates true` casks correctly: decide per cask whether `--greedy` is needed, and show the reason in the check output
- [ ] Respect pinned formulae (`brew pin`) and report them as "held back" instead of failing
- [ ] Restart Homebrew services (`brew services`) whose formulae were upgraded, and report which ones were restarted
- [ ] Support casks whose artifacts are not `.app` (pkg installers, menu-bar helpers, `uninstall quit:` bundle ids) by reading the cask's `uninstall` stanza for the bundle ids to quit
- [ ] Detect apps that update themselves (Sparkle, Electron auto-update) and are mid-update, and wait instead of quitting them
- [ ] Preserve window/session state where possible: reopen apps in the same Space/display, hidden state preserved (`open -g -j`)
- [ ] Handle apps that were running as login items or menu-bar-only agents without a Dock icon
- [ ] Add Mac App Store updates via `mas outdated` / `mas upgrade`, with a skip when not signed in
- [ ] Add macOS system updates: list via `softwareupdate --list` (report-only by default), opt-in `--install-macos-updates`, and detect pending reboot
- [ ] Update Xcode Command Line Tools when `softwareupdate` lists them, and handle the common "CLT out of date after macOS update" brew error
- [ ] Add `brew doctor` and `brew bundle cleanup` (when a Brewfile exists) as report-only checks
- [ ] Handle Rosetta / dual-arch Homebrew installs (`/opt/homebrew` and `/usr/local`)
- [ ] Add Docker Desktop and OrbStack support alongside Colima (detect the active runtime, stop/start it the same way)
- [ ] Add Lima, Podman machine and Rancher Desktop VM handling using the same container-restore logic

## Milestone 5: Linux

- [ ] Add apt support (update, upgrade, autoremove, autoclean) with dpkg lock handling and `needrestart`/`/var/run/reboot-required` detection
- [ ] Add dnf/yum support with `needs-restarting` reboot detection
- [ ] Add pacman support (with optional AUR helpers `yay`/`paru`) including `pacnew` file reporting
- [ ] Add zypper, apk and emerge support as separate opt-in steps
- [ ] Add Flatpak (`flatpak update`, `uninstall --unused`) and Snap (`snap refresh`) steps
- [ ] Add Nix (`nix-channel --update`, `nix profile upgrade`, `home-manager switch`) and `nix-collect-garbage` as opt-in
- [ ] Add firmware updates through `fwupdmgr` (report-only by default)
- [ ] Privilege handling: use `sudo` only for steps that need it, validate credentials once up front, never run user-level tools as root
- [ ] Detect services that need a restart after upgrades (`systemctl`, `needrestart`) and offer to restart them
- [ ] Quit/restart desktop applications on Linux via `.desktop` entries and `gtk-launch`/`kill -TERM`, with the same graceful-then-force policy
- [ ] Add a Linux container integration test matrix (Ubuntu, Fedora, Arch) in CI

## Milestone 6: Windows

- [ ] Add winget support (`winget upgrade --all`, source update, pinned packages) with exit-code mapping
- [ ] Add Scoop (`scoop update *`, `scoop cleanup *`, `scoop cache rm *`) and Chocolatey (`choco upgrade all`) steps
- [ ] Add Windows Update reporting via the PSWindowsUpdate module or `usoclient`, report-only by default, with pending-reboot detection
- [ ] Add Microsoft Store app updates where an API or CLI is available
- [ ] Add WSL support: update `wsl --update`, and run the Linux steps inside each selected distro
- [ ] Quit/restart Windows applications gracefully (`CloseMainWindow`) with timeout and optional `taskkill /F`, never closing the host terminal
- [ ] Handle elevation: detect admin rights, request elevation only for steps that need it, and support running un-elevated with those steps skipped
- [ ] Support PowerShell 7 and Windows PowerShell 5.1 as launching shells, and `cmd.exe` shims
- [ ] Add a Windows CI job running the unit tests and a smoke test against stub package managers
- [ ] Resolve Windows executable names through `PATHEXT` during step detection so `uv` and `mise` are found as `uv.exe`/`mise.exe`, covered by tests with an injected lookup

## Milestone 7: Language and Developer Tooling

- [ ] mise: report when a newer Node LTS major exists than the active one (`mise ls-remote node@lts`) without switching automatically
- [ ] mise: add opt-in `--bump` flag with a warning that it replaces `lts`/`latest` with a fixed version
- [ ] mise: reinstall global npm packages listed in `~/.default-npm-packages` after the Node version changed, and the same for Python/Ruby default packages
- [ ] uv: run `uv self update` when uv is not installed via a system package manager, and `uv python upgrade` for managed interpreters
- [ ] Add pipx (`pipx upgrade-all`) and `pip` user-site outdated reporting (report-only)
- [ ] Add Rust: `rustup update`, `cargo install-update -a` (when cargo-update is installed)
- [ ] Add Go: report outdated binaries installed with `go install` and a safe update path
- [ ] Add global npm, pnpm, yarn and bun package updates, each as its own step with a skip when absent
- [ ] Add other version managers as steps: asdf, nvm, fnm, volta, pyenv, rbenv, sdkman, jenv, tfenv
- [ ] Add Ruby gems, Composer, Conda/Mamba, .NET tools and Elixir/Hex steps (all optional, skipped when absent)
- [ ] Add editor tooling: VS Code extensions (`code --update-extensions`), Neovim plugin managers (lazy.nvim, packer) and Mason
- [ ] Add shell tooling: oh-my-zsh, zinit, antidote, tmux plugin manager (tpm), fisher
- [ ] Add cloud/DevOps CLIs: `gh extension upgrade --all`, `kubectl krew upgrade`, `helm repo update`, AWS CLI v2 (non-brew installs), `gcloud components update`, `az upgrade`
- [ ] Add git-related upkeep: `git lfs` self-check, global pre-commit hook repos (`pre-commit autoupdate` for opted-in repo paths)
- [ ] Implement read-only `check()` for the uv and mise steps (`uv tool list --outdated`, `mise outdated`) so `--check` reports pending tool updates instead of always reporting them up to date, covered by recorded transcripts

## Milestone 8: Containers and Services

- [ ] Per-project opt-out for compose pull/up using a label or a config list of excluded projects
- [ ] Health-check aware recreation: after `up -d`, wait for health checks and report containers that did not become healthy, with the previous image digest for rollback
- [ ] Pin-aware pulls: skip images referenced by digest or a fixed non-`latest` tag unless `--docker-bump` is given
- [ ] Add Podman and `podman-compose` support with the same pull/recreate/prune flow
- [ ] Add local Kubernetes tooling handling: k3d, kind and minikube clusters are stopped/started around runtime upgrades
- [ ] Image cleanup policy options: prune dangling only (default), prune unused older than N days, report-only mode that shows reclaimable space
- [ ] Build cache and volume reporting with `docker system df`, and opt-in `docker builder prune`
- [ ] Support Docker contexts and remote engines safely (never touch a non-local context unless selected)
- [ ] Restart containers that were running before a runtime restart even if they were started with `docker run --restart no`, using the recorded snapshot

## Milestone 9: Configuration and UX

- [ ] Add a config file (TOML) at the OS-appropriate location (`~/.config/<name>/config.toml`, `%APPDATA%`) with a documented schema and validation errors that point at the offending line
- [ ] Support per-step enable/disable, include/exclude lists (casks, packages, containers, projects) and `NEVER_QUIT` / `ALWAYS_QUIT` app lists
- [ ] Support named profiles (e.g. `work`, `minimal`) selectable with `--profile`
- [ ] Add pre/post hooks per step and global hooks (`pre_run`, `post_run`) with environment variables describing the run
- [ ] Add user-defined custom steps (a name, a command, platforms, order) so anything not built in can be added without code changes
- [ ] Environment variable overrides for every config key with a documented naming scheme
- [ ] Add `--doctor` that verifies the environment (tools found, permissions, config validity) and prints actionable fixes
- [ ] Rich terminal output: step progress, spinners when interactive, plain output when piped or in CI, `NO_COLOR` support
- [ ] Add desktop notifications at the end of a run (macOS Notification Center, `notify-send`, Windows toast) with the result summary
- [ ] Log files with rotation in the OS-appropriate location, `--verbose` / `--quiet` levels and a `logs` subcommand to show the last run
- [ ] Add a `history` subcommand listing previous runs and what each one changed (from the snapshots)
- [ ] Add shell completions for zsh, bash, fish and PowerShell, and a man page
- [ ] Add a `self-update` subcommand and an update-available hint that never blocks the run
- [ ] Cap captured subprocess output (for example the last N bytes per stream) so a runaway command cannot exhaust memory, while still streaming every line live

## Milestone 10: Automation

- [ ] Add a `schedule install` / `schedule remove` subcommand: launchd agent (macOS), systemd user timer (Linux), Task Scheduler task (Windows)
- [ ] Scheduled runs default to check-only mode and notify when updates are pending; a separate opt-in enables unattended upgrades
- [ ] Skip scheduled runs when on battery, on a metered connection, in a meeting/presentation (frontmost full-screen app) or when the user was active in the last N minutes
- [ ] Catch-up behaviour: run once after wake/login when the scheduled time was missed
- [ ] Add a `--non-interactive` mode that never prompts and picks the safe default for every decision

## Milestone 11: Distribution and Docs

- [ ] GitHub Releases with prebuilt binaries for macOS (arm64, x86_64), Linux (x86_64, aarch64) and Windows (x86_64), with checksums (cargo-dist or an equivalent cross-compilation workflow)
- [ ] Publish to crates.io (`cargo install freshen`) and support `cargo binstall`
- [ ] Sign and notarize the macOS binary, sign the Windows binary, and publish SBOM / provenance attestations
- [ ] Homebrew tap formula, Scoop bucket manifest and winget manifest
- [ ] Linux packages: `.deb`, `.rpm`, an AUR PKGBUILD and a Nix flake
- [ ] One-line install script with checksum verification
- [ ] Docs site (mkdocs) with: quick start, config reference, step reference (one page per step), platform notes, FAQ and troubleshooting
- [ ] "Writing a step" contributor guide with a worked example and a step template
- [ ] Add a terminal recording (asciinema/GIF) to the README
- [ ] Add issue templates (bug report with `--doctor` output, new step request) and a security policy

## Milestone 12: Quality

- [ ] CI matrix: macOS, Ubuntu and Windows runners for lint, unit tests and the stub-based integration tests
- [ ] Recorded-transcript tests for parsers of third-party output (brew JSON, docker compose ls, winget, apt) with fixtures for several tool versions
- [ ] Property tests (`proptest`) and `cargo fuzz` targets for the parsers so unexpected output never crashes the run
- [ ] Enforce a minimum test coverage threshold in CI with `cargo llvm-cov`
- [ ] Container-based integration tests on Linux that exercise real package managers against throwaway images
- [ ] Add a scheduled CI job that runs against the latest Homebrew/winget/apt metadata to catch upstream output changes early
- [ ] Add performance budget: `--check` on a typical machine completes within a documented time

## Discovered

- [ ] Add the Cargo workspace checks promised by `tests/README.md` to `tests/run-tests.sh`: the workspace sets an MSRV (`rust-version`), `rust-toolchain.toml` pins an exact channel with `rustfmt`/`clippy`, and every crate under `crates/` inherits the workspace MSRV and lints
- [ ] Retry stub execution when the kernel returns `ETXTBSY` (text file busy), so parallel test threads that write and execute a stub cannot fail spuriously, with a test for the retry helper

<!-- The agent appends newly found work here. -->
