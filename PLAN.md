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
- [x] Preflight checks: network reachable, enough free disk space, not on battery below a threshold, not on a metered/hotspot connection (configurable, each skippable with a flag)
- [x] Add a lock file so two freshen runs cannot overlap, with a clear message naming the PID that holds it
- [x] Detect when another package manager process is running (brew, apt/dpkg lock, winget) and wait or abort with a clear message
- [x] Graceful handling of SIGINT/SIGTERM on every OS: stop the running step, restore everything that was stopped (apps, containers, VMs), print a partial summary
- [x] Idempotency guarantee: running the tool twice in a row must be a no-op the second time; add a test asserting it
- [x] Per-step and global timeouts with a clear "timed out" status distinct from "failed"
- [x] Network failure handling: retry transient download errors with exponential backoff, skip the step if the registry is unreachable
- [x] Detect processes running from a tool version that is about to be removed (old Node, Python, Java, Go toolchains) and report them with the parent application that started them
- [x] Detect long-running interactive sessions that an upgrade would break (zellij, tmux server, SSH sessions, running docker builds) and warn or defer
- [x] Never run destructive cleanup (volume prune, cache wipe) unless explicitly enabled; log everything a cleanup removed and the space reclaimed
- [x] Exit codes documented and stable: 0 ok, 1 step failure, 2 updates pending (check mode), 3 preflight failed, 130 interrupted
- [x] Kill the whole process tree on timeout or interrupt (process group on Unix, job object on Windows) so grandchildren that keep output pipes open cannot outlive the step, covered by a test using a shell that spawns a child
- [x] Prune old pre-upgrade snapshots with a configurable retention count (default keeps a small recent window) so automatic snapshots do not grow without bound, covered by tests
- [x] Record cask versions in the pre-upgrade snapshot (`brew list --cask --versions`) so rollback hints can cover upgraded casks, covered by tests
- [x] Guard `brew autoremove` and `brew cleanup` with their dry-run listings so a run with nothing to remove does not invoke the mutating command, covered by tests
- [x] Extend the idempotency test to the uv, mise, Docker and Colima steps with stateful stubs, so every mutating step proves a second run changes nothing
- [x] Apply per-step and global timeouts to `--check` mode, with a "timed out" check status distinct from "failed", so a hung read-only query cannot hang the report
- [x] Make retry backoff sleeps respect the step and global deadlines and SIGINT, so a long retry wait cannot overshoot a timeout or delay interrupt handling, covered by tests
- [x] Probe the process table for stale toolchain processes on Windows (parent PID and command line) instead of silently skipping when `ps` is unavailable, covered by tests with injected process facts
- [x] Warn only about toolchain versions `mise prune` will actually remove, using a dry-run or install listing so a process on a newer non-active version is not reported as old, covered by tests
- [x] Include destructive cleanup results (removed items and reclaimed space) in the `--json` and `--markdown` reports, covered by tests
- [x] Assign a Windows child to its job object before it starts running (spawn suspended, then resume after assignment) so a grandchild spawned immediately cannot escape the process tree on timeout, covered by a Windows test
- [x] Make the package-manager wait loop check for SIGINT instead of polling until the wait expires, so an interrupt during the pre-run wait is honored promptly, covered by a test
- [x] Send human progress and cleanup log lines to stderr when `--json` or `--markdown` is selected, so the structured report is the only thing on stdout and can be parsed directly, covered by tests
- [x] Honour `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY` and custom CA bundles in the network preflight check and document how every step inherits them, covered by tests with injected environment
- [x] Report the specific network preflight failure (invalid proxy URL, unreadable CA bundle, TLS handshake failure) instead of the generic "network is unreachable", covered by tests with injected environment
- [x] Support CIDR ranges such as `10.0.0.0/8` in `NO_PROXY` matching so private networks are bypassed without listing every address, covered by tests with injected environment
- [x] Turn the zellij, tmux and SSH session warnings into one table-driven session guard (entry per tool: package names, server process pattern, in-session environment variable such as `TMUX` or `ZELLIJ`), keeping today's behaviour so that adding a tool is one table row, covered by the existing tests
- [x] Add GNU screen (`STY`, `screen -ls`), byobu, dtach/abduco and mosh-server to the session guard so an upgrade warns about every terminal session it would break, covered by tests with injected process tables
- [x] Extend terminal emulator detection with iTerm2, Apple Terminal, Rio, Termius, Zed, Cursor, Windsurf and JetBrains IDE terminals (bundle ids and `TERM_PROGRAM` values) so freshen never quits the terminal that runs it, covered by tests
- [x] Generalise the README wording: replace the zellij-only bullet and the Ghostty/iTerm2 examples with the list of supported terminals and session managers, generated from the same tables where possible
- [x] Implement read-only `check()` for the uv and mise steps (`uv tool list --outdated`, `mise outdated`) so `--check` reports pending tool updates instead of always reporting them up to date, covered by recorded transcripts
- [x] Run independent read-only `check()` queries concurrently with bounded concurrency (`--jobs N`, default 4) while keeping the printed order and the summary deterministic, covered by tests with slow stubs
- [x] Fall back to the detected terminal app's bundle id when `__CFBundleIdentifier` is absent, so the cask quit protection also covers hosts identified only through `TERM_PROGRAM` or environment markers, covered by tests

## Milestone 4: CI, Configuration and macOS Depth

- [x] Make `tests/run-tests.sh` accept each proposed workflow in either `ci-proposals/<name>.yml` or `.github/workflows/<name>.yml` (rust.yml, ci.yml, real-tools.yml), so that a human applying a proposal with `git mv` does not break the repository checks, covered by a test that runs the checks in both layouts
- [x] Retry stub execution when the kernel returns `ETXTBSY` (text file busy), so parallel test threads that write and execute a stub cannot fail spuriously, with a test for the retry helper
- [x] Make `preflight::tests::tcp_reachability_detects_open_and_closed_ports` robust against another process reusing the just-released port, so the suite does not fail spuriously on shared CI runners
- [x] CI matrix: macOS, Ubuntu and Windows runners for lint, unit tests and the stub-based integration tests
- [x] Propose a CI job (as `ci-proposals/real-tools.yml`) that runs `freshen --check` and `freshen --dry-run` read-only against the real Homebrew on `macos-latest` and fails on parser errors, so the recorded transcripts are verified against real output
- [x] Extend the proposed real-tool job to `windows-latest` (real Chocolatey and winget, read-only) and `ubuntu-latest` (real apt, read-only), with the same failure rule
- [x] Port the stub-based integration tests that do not need Unix-only facilities (signals, shell scripts, POSIX paths) off `#![cfg(unix)]` and run them against the Windows `.cmd` stubs, so the Windows matrix leg executes the package-manager transcripts instead of compiling empty test crates
- [x] Make the Windows unit tests pass on `windows-latest` (run `gh run view --log-failed` style output is in the CI log): (1) apps.rs `command_transcripts_match_the_zsh_script` joins with `\\`, compare normalized paths; (2) executable.rs `a_windows_candidate_is_found_through_its_extension` returns `tool.CMD` from PATHEXT, compare case-insensitively; (3) lock.rs `a_stale_lock_file_from_a_dead_run_does_not_block` reads a file that another handle has byte-range locked (os error 33), read through the locking handle or open with sharing; (4) network.rs CA bundle tests (os error 123, invalid file name syntax) build a path that is invalid on Windows, fix the name and `file://`-style path handling; (5) process.rs `a_windows_child_joins_its_job_before_it_can_start_a_grandchild` never sees the marker, investigate the suspended-child/job-object ordering. Then remove `continue-on-error` from `.github/workflows/rust.yml` via a `ci-proposals/` note in HUMAN-TASKS.md
- [x] Replace the unmaintained `rustls-pemfile` (used in network.rs) with `rustls_pki_types::pem::PemObject`, bump `rustls-native-certs` to 0.8, then remove the RUSTSEC-2025-0134 ignore from deny.toml
- [x] Add a config file (TOML) at the OS-appropriate location (`~/.config/<name>/config.toml`, `%APPDATA%`) with a documented schema and validation errors that point at the offending line
- [x] Support per-step enable/disable, include/exclude lists (casks, packages, containers, projects) and `NEVER_QUIT` / `ALWAYS_QUIT` app lists
- [x] Run every command whose output is parsed with a neutral locale (`LC_ALL=C`, `LANG=C`; UTF-8 console and `--disable-interactivity` on Windows) so localized output such as Turkish and progress spinners cannot break parsers, covered by tests that run the parsers on non-English fixtures
- [x] Detect an Automation (TCC) denial when quitting apps through AppleScript (error -1743) and print an actionable message naming the System Settings pane instead of treating the app as unquittable, covered by tests with injected osascript output
- [x] Detect casks whose installer needs an administrator password (pkg installers, `sudo` prompts) and skip them with a clear reason in non-interactive runs instead of hanging, covered by tests
- [x] Handle `version :latest` casks and `auto_updates true` casks correctly: decide per cask whether `--greedy` is needed, and show the reason in the check output
- [x] Respect pinned formulae (`brew pin`) and report them as "held back" instead of failing
- [x] Restart Homebrew services (`brew services`) whose formulae were upgraded, and report which ones were restarted
- [x] Support casks whose artifacts are not `.app` (pkg installers, menu-bar helpers, `uninstall quit:` bundle ids) by reading the cask's `uninstall` stanza for the bundle ids to quit
- [x] Detect apps that update themselves (Sparkle, Electron auto-update) and are mid-update, and wait instead of quitting them
- [x] Preserve window/session state where possible: reopen apps in the same Space/display, hidden state preserved (`open -g -j`)
- [x] Handle apps that were running as login items or menu-bar-only agents without a Dock icon
- [x] Add Mac App Store updates via `mas outdated` / `mas upgrade`, with a skip when not signed in
- [x] Add macOS system updates: list via `softwareupdate --list` (report-only by default), opt-in `--install-macos-updates`, and detect pending reboot
- [x] Update Xcode Command Line Tools when `softwareupdate` lists them, and handle the common "CLT out of date after macOS update" brew error
- [x] Add `brew doctor` and `brew bundle cleanup` (when a Brewfile exists) as report-only checks
- [x] Handle Rosetta / dual-arch Homebrew installs (`/opt/homebrew` and `/usr/local`)
- [x] Introduce a `ContainerRuntime` abstraction (detect, is_running, stop, start, and which formulae or casks trigger a restart) and move the existing Colima/lima logic behind it as the first implementation without changing behaviour, covered by the existing parity and idempotency tests
- [x] Detect the active container runtime from `DOCKER_HOST`, `docker context show` and the well-known socket paths (Colima, OrbStack, Docker Desktop, Rancher Desktop, Podman, native `/var/run/docker.sock`), show it in `--list-steps` and the run summary, and allow an override with a `--container-runtime` flag, covered by tests with injected environment and socket facts
- [x] Add Docker Desktop on macOS: detect it, quit it with `docker desktop stop` (falling back to the AppleScript quit) before its cask is upgraded, start it again afterwards and bring compose projects and standalone containers back with the same restore logic as Colima, covered by stub-based tests
- [x] Add OrbStack: detect it with `orbctl status`, stop and start it with `orbctl stop` and `orbctl start` around an upgrade of its cask, and restore containers, covered by stub-based tests
- [x] Add Rancher Desktop: stop and start it with `rdctl shutdown` and `rdctl start` around its cask upgrade and restore containers, covered by stub-based tests
- [x] Add Podman machine: stop and start it with `podman machine stop` and `podman machine start` around a Podman upgrade and restore containers, covered by stub-based tests
- [x] Add standalone Lima instances (`limactl list`, `stop`, `start`) that are not managed by Colima, covered by stub-based tests
- [x] Detect the compose command in use (`docker compose`, `docker-compose`, `podman compose`) and use it consistently for pull, up and down, covered by tests with injected command lookups
- [x] Add native Docker Engine on Linux to the runtime abstraction (restart through `systemctl restart docker` with a warning about running containers when `live-restore` is off), covered by tests with injected facts
- [x] Update the README and docs to describe the supported container runtimes and replace the Colima-only wording, once the runtime abstraction exists
- [x] Add a read-only Homebrew cask metadata probe to `--check` (or a dedicated command) so the real-tools job verifies the recorded `brew info --cask --json=v2` transcript and fails on cask JSON parser drift, covered by tests
- [x] Apply the package include/exclude list to the mise step, so an excluded mise tool is not upgraded (mise upgrades every plugin in one command, so the list needs a per-tool upgrade path), covered by tests
- [x] Skip the steps whose prerequisite is disabled in the configuration file, so disabling `brew` also skips the Homebrew steps that need it, covered by tests
- [x] Name the host terminal application in the Automation denial message (for example `Terminal`), so the user knows which entry to switch on in the System Settings Automation pane, covered by tests
- [x] Report why an AppleScript quit failed when the failure is not a denied Automation request (for example error -600 "application isn't running"), instead of the generic "did not quit" message, covered by tests with injected osascript output
- [x] Report the casks that were skipped because their installer needs an administrator password in the run summary and in the `--json`/`--markdown` reports, so a structured report names them instead of only printing a human-readable warning
- [x] Refactor `crates/freshen/tests/homebrew.rs` (and any other stub integration test that repeats a full `step_calls()` list) so the expected brew call transcript is built from small shared helpers (a base transcript plus named insertions) instead of being repeated in every test, so that adding one brew call touches one place; keep every existing assertion's meaning unchanged, no behavior changes
- [x] Refactor the container runtime stub tests (`crates/freshen/tests/containers.rs`, `lima.rs`, `orbstack.rs`, `podman.rs`, `rancher_desktop.rs`, `docker_desktop.rs`, `native_docker.rs`) so the expected call transcripts are built from small shared helpers (a base transcript plus named insertions, for example in `crates/freshen/tests/support/`) instead of repeating the full `step_calls()` list in every test, so that adding one external command touches one place; keep every assertion's meaning unchanged, make no behavior changes, and keep the change to these files only
- [x] Refactor the tooling and listing stub tests (`crates/freshen/tests/tooling.rs`, `lists.rs`, `idempotency.rs`) so the expected call transcripts are built from small shared helpers (a base transcript plus named insertions, for example in `crates/freshen/tests/support/`) instead of repeating the full `step_calls()` list in every test, so that adding one external command touches one place; keep every assertion's meaning unchanged, make no behavior changes, and keep the change to these files only
- [x] Refactor the remaining stub tests that repeat a full `step_calls()` or `calls()` list (`appstore.rs`, `cleanup.rs`, `macos_updates.rs`, `network.rs`, `report.rs`, `locale.rs`, `stub_harness.rs`) so the expected call transcripts are built from small shared helpers (a base transcript plus named insertions, for example in `crates/freshen/tests/support/`) instead of repeating the full `step_calls()` list in every test, so that adding one external command touches one place; keep every assertion's meaning unchanged, make no behavior changes, and keep the change to these files only
- [x] Respect pinned casks (`brew pin --cask`) as held back too, so a pinned cask is skipped and reported instead of being shown as upgradable, covered by tests
- [x] Report the Homebrew services that were restarted in the run summary and in the `--json`/`--markdown` reports, so a structured report names them instead of only printing a human-readable line, covered by tests
- [x] Stop apps for casks whose `uninstall` stanza lists only `signal` entries (for example Zoom's `signal: ["KILL", "us.zoom.xos"]`) instead of a `quit` bundle id, so a pkg-installer cask with no `quit` entry still has its running app stopped before the upgrade, covered by tests with a recorded transcript
- [ ] Skip the cask upgrade of an app that is still updating itself after the wait, so the Homebrew install does not race the app's own updater, covered by tests
- [ ] Report the apps that were left running because they were updating themselves in the run summary and in the `--json`/`--markdown` reports, so a structured report names them instead of only printing a human-readable line, covered by tests
- [ ] Detect an Electron (Squirrel) update that was downloaded but not applied yet (a `pending` directory in the app's cache) and treat that app as updating itself, covered by tests
- [ ] Explain that the hidden state and the window place of an app could not be read when macOS denies the permission to control System Events, so the user knows why a hidden app was reopened in the background, covered by tests with injected osascript output
- [ ] Put every window of a reopened app back where it was, not only the first one, so an app with several windows comes back the way it was, covered by tests
- [ ] Report the apps that were reopened hidden and the apps whose window was put back in the run summary and in the `--json`/`--markdown` reports, so a structured report names them instead of only printing a human-readable line, covered by tests
- [ ] Detect an app that sets `LSBackgroundOnly` in its `Info.plist` and treat it like a menu-bar app, so an app without any icon at all is ended instead of quit through an Apple event, covered by tests
- [ ] Find the app bundle of a bundle id that is only known by its id (a cask `uninstall` stanza), so an app without a Dock icon is detected, ended and reopened like any other app, covered by tests
- [ ] Handle a cask that installs a LaunchAgent (a login item without an app bundle): stop the agent before the upgrade and start it again afterwards with `launchctl kickstart -k`, covered by stub-based tests
- [ ] Apply the include and exclude lists to the Mac App Store apps, upgrading only the selected apps by their App Store id (`mas upgrade <id>`), so a listed app is neither upgraded nor reported as outdated, covered by tests
- [ ] Detect a missing App Store account on `mas` 5 and newer, which have no `account` command, so the Mac App Store step is skipped instead of failing, covered by tests with recorded transcripts
- [ ] Decide what the Mac App Store step does when `mas upgrade` needs a password: skip the upgrade in a non-interactive run or hand the terminal to `sudo`, so a scheduled run does not fail on a missing credential, covered by tests
- [ ] Detect a macOS restart that is still pending from an update that was installed earlier, not only in the run that installed it, so `freshen --check` tells the user that the machine has to restart before it is up to date, covered by tests with the recorded output of a machine with a staged update
- [ ] Update the Xcode Command Line Tools before the Homebrew steps run, so that a run which installs them also upgrades the packages in the same run instead of failing on the outdated tools, covered by tests
- [ ] Report the `brew doctor` warnings and the `brew bundle cleanup` packages in the run summary and in the `--json`/`--markdown` reports, so a structured report names them instead of only printing a human-readable line
- [ ] Make the Colima/lima and container steps discover both Homebrew installations (`/opt/homebrew` and `/usr/local`) instead of only the `brew` on `PATH`, so an outdated `colima`/`lima` formula in either architecture stops the containers before the upgrade, covered by tests with injected prefixes
- [ ] Complete the Docker Desktop AppleScript fallback: wait for the app to exit after the quit and start it again by its app bundle when `docker desktop start` is unavailable, so older versions finish the stop/start cycle around their cask upgrade, covered by stub-based tests
- [ ] Stop standalone Lima instances only when the `lima` formula itself is being upgraded, not on a Colima-only upgrade, so an unrelated VM keeps running, covered by stub-based tests
- [ ] Prefer `podman compose` when the active runtime is Podman even if a Docker compose implementation is also installed, so a forced `--container-runtime podman` run does not fall back to the Docker CLI, covered by tests with injected command lookups
- [ ] Use the active runtime's container CLI (`podman ps`, `podman image prune`) for standalone container listing and image cleanup when Podman is active, so a Podman-only host without the Docker CLI still restores containers, covered by stub-based tests

## Milestone 5: Linux

- [ ] Add apt support (update, upgrade, autoremove, autoclean) with dpkg lock handling and `needrestart`/`/var/run/reboot-required` detection
- [ ] Add dnf/yum support with `needs-restarting` reboot detection
- [ ] Add pacman support (with optional AUR helpers `yay`/`paru`) including `pacnew` file reporting
- [ ] Add zypper support (`zypper refresh`, `zypper update`, and `zypper ps -s` to list services that need a restart) as an opt-in step, covered by recorded transcripts
- [ ] Add apk support (`apk update`, `apk upgrade`) for Alpine as an opt-in step, covered by recorded transcripts
- [ ] Add emerge support (`emerge --sync`, `emerge -uDN @world`) for Gentoo as an opt-in, report-first step, covered by recorded transcripts
- [ ] Add Flatpak (`flatpak update -y`, and `flatpak uninstall --unused` only with `--cleanup`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add Snap (`snap refresh`, mentioning a held refresh window when snapd reports one), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add Nix upgrades (`nix-channel --update`, `nix profile upgrade`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add `home-manager switch` as an opt-in step that runs only when a home-manager configuration is detected, covered by tests with injected paths
- [ ] Add `nix-collect-garbage` as a cleanup step that runs only with `--cleanup` and reports the reclaimed space, covered by recorded transcripts
- [ ] Add firmware updates through `fwupdmgr` (report-only by default)
- [ ] Privilege handling: use `sudo` only for steps that need it, validate credentials once up front, never run user-level tools as root
- [ ] Detect services that need a restart after upgrades (`systemctl`, `needrestart`) and offer to restart them
- [ ] Quit/restart desktop applications on Linux via `.desktop` entries and `gtk-launch`/`kill -TERM`, with the same graceful-then-force policy
- [ ] Add a Linux container integration test matrix (Ubuntu, Fedora, Arch) in CI
- [ ] Make the Homebrew steps work with Homebrew on Linux (`/home/linuxbrew/.linuxbrew`, no casks), covered by tests with injected paths
- [ ] Add image-based distributions: `rpm-ostree upgrade` (Fedora Silverblue/Kinoite) and `bootc upgrade` as staged upgrades that announce the pending reboot, covered by recorded transcripts
- [ ] (Optional, low priority) Detect Termux on Android (`TERMUX_VERSION`, `PREFIX` under `/data/data/com.termux`) and treat it as a Linux variant that upgrades with `pkg update` and `pkg upgrade` without root, covered by tests with injected environment
- [ ] Trigger the native Docker Engine stop/restart cycle when apt, dnf, pacman or zypper upgrades the Docker package (`docker.io`/`docker-ce`), not only when the Homebrew `docker` formula is outdated, covered by stub-based tests

## Milestone 6: Windows

- [ ] Add winget support (`winget upgrade --all`, source update, pinned packages) with exit-code mapping
- [ ] Add Scoop steps (`scoop update`, `scoop update *`, `scoop cleanup *`, `scoop cache rm *`), skipped when scoop is absent, covered by recorded transcripts
- [ ] Add Windows Update reporting via the PSWindowsUpdate module or `usoclient`, report-only by default, with pending-reboot detection
- [ ] Add Microsoft Store app updates where an API or CLI is available
- [ ] Add WSL support: update `wsl --update`, and run the Linux steps inside each selected distro
- [ ] Quit/restart Windows applications gracefully (`CloseMainWindow`) with timeout and optional `taskkill /F`, never closing the host terminal
- [ ] Handle elevation: detect admin rights, request elevation only for steps that need it, and support running un-elevated with those steps skipped
- [ ] Support PowerShell 7 and Windows PowerShell 5.1 as launching shells, and `cmd.exe` shims
- [ ] Add a Windows CI job running the unit tests and a smoke test against stub package managers
- [ ] Resolve Windows executable names through `PATHEXT` during step detection so `uv` and `mise` are found as `uv.exe`/`mise.exe`, covered by tests with an injected lookup
- [ ] Add Chocolatey detection (`choco.exe` through `PATHEXT`, `choco --version`) and mark the step as requiring administrator rights, so an un-elevated run skips it with the reason "needs administrator" instead of failing, covered by tests with an injected elevation state
- [ ] Implement read-only `check()` for Chocolatey with `choco outdated --limit-output` (parse `name|current|available|pinned`), reporting pinned packages as held back, covered by recorded transcripts
- [ ] Add the Chocolatey upgrade step: first `choco upgrade chocolatey -y --no-progress`, then `choco upgrade all -y --no-progress`, streaming output and recording per-package failures in the summary, covered by stub-based tests
- [ ] Map Chocolatey exit codes: 0 is success, 1641 and 3010 are success with a "reboot pending" flag in the summary, any other non-zero code is a failure, covered by tests
- [ ] Honour `choco pin` and add an exclusion list for Chocolatey packages that is passed as `--except`, with the excluded packages shown in `--list-steps` and the run summary, covered by tests
- [ ] Record `choco list --local-only -r` in the pre-upgrade snapshot and print rollback hints (`choco upgrade <name> --version <old>`) for upgraded packages, covered by tests
- [ ] Report packages that are installed by more than one Windows package manager (winget, Chocolatey, Scoop) so the same application is not upgraded twice unnoticed, as a report-only check covered by tests with injected package lists
- [ ] Add a stub `choco` executable to the stub harness (canned `outdated` output, exit codes 0, 1641, 3010 and 1) and cover the Chocolatey step end to end on every platform
- [ ] Warn before `wsl --update` because it shuts down every running WSL distribution and anything running inside it (including containers): list the running distributions and require confirmation or `--yes`, covered by tests with injected `wsl --list --running` output
- [ ] Detect running processes whose executables live under the install location of an outdated winget, Chocolatey or Scoop package and apply the quit/force-kill policy before upgrading it, covered by tests with injected process and package facts
- [ ] Run PowerShell-based steps with a process-scoped execution policy (`-NoProfile -ExecutionPolicy Bypass` only for freshen's own invocation) and never change the machine policy, covered by tests of the generated command line
- [ ] Add Docker Desktop on Windows (WSL2 backend) and Podman Desktop to the container runtime abstraction, including the interaction with `wsl --update`, covered by stub-based tests

## Milestone 7: Language and Developer Tooling

- [ ] mise: report when a newer Node LTS major exists than the active one (`mise ls-remote node@lts`) without switching automatically
- [ ] mise: add opt-in `--bump` flag with a warning that it replaces `lts`/`latest` with a fixed version
- [ ] mise: reinstall global npm packages listed in `~/.default-npm-packages` after the Node version changed, and the same for Python/Ruby default packages
- [ ] uv: run `uv self update` when uv is not installed via a system package manager, and `uv python upgrade` for managed interpreters
- [ ] Add pipx (`pipx upgrade-all`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add a report-only `pip list --user --outdated` check that never upgrades anything, covered by recorded transcripts
- [ ] Add rustup (`rustup update`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add `cargo install-update -a` when cargo-update is installed, skipped when the tool is absent, covered by recorded transcripts
- [ ] Add Go: report outdated binaries installed with `go install` and a safe update path
- [ ] Add global npm packages (`npm outdated -g --json`, `npm update -g`), skipped when npm is absent or managed by mise, covered by recorded transcripts
- [ ] Add global pnpm packages (`pnpm update -g`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add global yarn packages (`yarn global upgrade`, classic yarn only), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add global bun packages (`bun update -g`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add asdf (`asdf plugin update --all` and a report of tools with newer versions) without changing the versions pinned in `.tool-versions`, skipped when the tool is absent, covered by recorded transcripts
- [ ] Add nvm, fnm and volta: report when a newer Node LTS exists than the one in use and update the manager itself only when it is not managed by brew or mise, covered by recorded transcripts
- [ ] Add pyenv and rbenv: update the managers and their plugins (`pyenv update`, `git pull` of rbenv and ruby-build) and report newer Python and Ruby versions without installing them, covered by recorded transcripts
- [ ] Add SDKMAN (`sdk selfupdate`, `sdk upgrade`) run in a non-interactive shell, skipped when the tool is absent, covered by recorded transcripts
- [ ] Add jenv and tfenv as report-only steps that show newer Java and Terraform versions without switching, covered by recorded transcripts
- [ ] Add Ruby gems (`gem update --system`, `gem update` for user-installed gems), skipped when Ruby is managed by a system package manager, covered by recorded transcripts
- [ ] Add Composer (`composer self-update`, `composer global update`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add Conda and Mamba (`conda update -n base conda`, `micromamba self-update`) and report-only checks for other environments, covered by recorded transcripts
- [ ] Add .NET global tools (`dotnet tool update --global` for each installed tool), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add Elixir tooling (`mix local.hex --force`, `mix local.rebar --force`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add VS Code extensions (`code --update-extensions`), and the same for Cursor and VSCodium when their CLIs exist, covered by recorded transcripts
- [ ] Add Neovim plugin managers (`lazy.nvim` through `nvim --headless "+Lazy! sync" +qa`, packer) as opt-in steps because they change plugins, covered by recorded transcripts
- [ ] Add Mason and Treesitter parser updates (`nvim --headless "+MasonUpdate" +qa`) as an opt-in step, covered by recorded transcripts
- [ ] Add oh-my-zsh (`omz update --unattended`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add zinit (`zinit self-update`, `zinit update --all`) and antidote (`antidote update`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add the tmux plugin manager (`~/.tmux/plugins/tpm/bin/update_plugins all`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add fisher (`fisher update`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add GitHub CLI extensions (`gh extension upgrade --all`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add kubectl plugins (`kubectl krew upgrade`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add Helm (`helm repo update`) and a report-only list of releases with newer chart versions, covered by recorded transcripts
- [ ] Add AWS CLI v2 for non-brew installs: report a newer version and update through the official installer only with an explicit flag, covered by recorded transcripts
- [ ] Add the Google Cloud CLI (`gcloud components update --quiet`) when installed outside a system package manager, covered by recorded transcripts
- [ ] Add the Azure CLI (`az upgrade --yes`) when installed outside a system package manager, covered by recorded transcripts
- [ ] Add a `git lfs` self-check (`git lfs version`, `git lfs install --skip-repo`), skipped when the tool is absent, covered by recorded transcripts
- [ ] Add `pre-commit autoupdate` as an opt-in step for the repositories listed in the config, never scanning the disk for repositories, covered by tests

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
- [ ] Detect and control a rootless Docker Engine (`$XDG_RUNTIME_DIR/docker.sock`, `systemctl --user restart docker`), so it is not mistaken for an unreachable system daemon, covered by tests with injected facts

## Milestone 9: Configuration and UX

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
- [ ] Refuse to load the config file, hooks and custom steps from world-writable files or directories and print the offending path, covered by tests on Unix and with injected ACL facts on Windows
- [ ] Print the exact command of every hook and custom step before running it (unless `--quiet`) and require `--trust-config`, or an interactive confirmation that stores a hash, the first time a new or changed hook or custom step is seen, covered by tests
- [ ] Add a `FRESHEN_CONFIG` environment variable that points at an alternative configuration file, so a scheduled run or a wrapper script can choose the file without changing the command line, covered by tests
- [ ] Print the configuration file freshen read and the values it set (in `--doctor` and in the run summary), so a user can see which settings were in effect, covered by tests

## Milestone 10: Automation

- [ ] Add a `schedule install` / `schedule remove` subcommand: launchd agent (macOS), systemd user timer (Linux), Task Scheduler task (Windows)
- [ ] Scheduled runs default to check-only mode and notify when updates are pending; a separate opt-in enables unattended upgrades
- [ ] Skip scheduled runs when on battery, on a metered connection, in a meeting/presentation (frontmost full-screen app) or when the user was active in the last N minutes
- [ ] Catch-up behaviour: run once after wake/login when the scheduled time was missed
- [ ] Add a `--non-interactive` mode that never prompts and picks the safe default for every decision
- [ ] Make the `--non-interactive` mode skip the casks whose installer needs an administrator password, so a scheduled run never waits for a password even when a terminal is attached

## Milestone 11: Distribution and Docs

- [ ] GitHub Releases with prebuilt binaries for macOS (arm64, x86_64), Linux (x86_64, aarch64) and Windows (x86_64), with checksums (cargo-dist or an equivalent cross-compilation workflow)
- [ ] Add Windows aarch64 and fully static Linux (musl) targets to the release workflow, with checksums for every artifact
- [ ] Publish to crates.io (`cargo install freshen`) and support `cargo binstall`
- [ ] Sign and notarize the macOS binary, sign the Windows binary, and publish SBOM / provenance attestations
- [ ] Homebrew tap formula, Scoop bucket manifest and winget manifest
- [ ] Linux packages: `.deb`, `.rpm`, an AUR PKGBUILD and a Nix flake
- [ ] One-line install script with checksum verification
- [ ] Docs site (mkdocs) with: quick start, config reference, step reference (one page per step), platform notes, FAQ and troubleshooting
- [ ] "Writing a step" contributor guide with a worked example and a step template
- [ ] Add a terminal recording (asciinema/GIF) to the README
- [ ] Add issue templates (bug report with `--doctor` output, new step request) and a security policy
- [ ] State in the README and docs that freshen collects no telemetry and sends no data itself (only the tools it runs and the optional self-update check use the network), and make the self-update check easy to disable

## Milestone 12: Quality

- [ ] Recorded-transcript tests for parsers of third-party output (brew JSON, docker compose ls, winget, apt) with fixtures for several tool versions
- [ ] Property tests (`proptest`) and `cargo fuzz` targets for the parsers so unexpected output never crashes the run
- [ ] Enforce a minimum test coverage threshold in CI with `cargo llvm-cov`
- [ ] Container-based integration tests on Linux that exercise real package managers against throwaway images
- [ ] Add a scheduled CI job that runs against the latest Homebrew/winget/apt metadata to catch upstream output changes early
- [ ] Add performance budget: `--check` on a typical machine completes within a documented time
- [ ] Add `docs/TESTING.md` with a manual checklist per operating system (a real run on macOS, Linux and Windows with the expected output and a rollback check) to run before every minor release

## Discovered

- [ ] Add the Cargo workspace checks promised by `tests/README.md` to `tests/run-tests.sh`: the workspace sets an MSRV (`rust-version`), `rust-toolchain.toml` pins an exact channel with `rustfmt`/`clippy`, and every crate under `crates/` inherits the workspace MSRV and lints
- [ ] Probe both IPv4 and IPv6 (and allow overriding the endpoint) in the network preflight check so IPv6-only or filtered networks are not reported as unreachable, covered by tests with injected endpoints
- [ ] Detect user-marked metered networks on macOS instead of only hotspot SSIDs, so the metered preflight check matches the system setting, covered by tests with injected command output
- [ ] Make `StubTools` append each invocation to the stub log atomically, so integration tests can assert the calls of concurrently executed `--check` steps without interleaved lines, covered by a test that runs two stubs at once
- [ ] Run every Windows command that needs a shell through `cmd.exe /C` with `cmd`-compatible quoting: the C runtime escapes a quote inside an argument with a backslash, which `cmd.exe` does not understand, so a quoted path arrives mangled
- [ ] Verify the stub-based integration tests on `windows-latest`: while the Windows unit tests failed, the job stopped before that step, so the `.cmd` stubs have never been exercised in CI
- [ ] Split `SSL_CERT_DIR` on the platform path delimiter when collecting CA bundle paths, the way OpenSSL and `rustls-native-certs` do, so a list of certificate directories is not treated as one bogus path that fails the network check, covered by tests
- [ ] Report the native trust store load errors from the network preflight even when only some certificates were loaded, so a partly broken system trust store is not silently ignored, covered by tests with an injected trust store result
- [ ] Parse the winget output as JSON (`winget --output json`), because winget translates its table with the Windows UI language and no locale variable can change that
- [ ] Add the `brew-doctor` and `brew-bundle-cleanup` steps to the real-tools workflow, so their parsers are verified against the real Homebrew output, and write a `Brewfile` in the job so the bundle cleanup step is not skipped

<!-- The agent appends newly found work here. -->
