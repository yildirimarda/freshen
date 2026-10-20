# 0001. Implement freshen in Rust

- Status: accepted
- Date: 2026-10-20

## Context

freshen keeps a developer machine up to date (package managers, language
toolchains, containers) on macOS, Linux and Windows. Until now the
implementation was the zsh script `update-all.sh`, which cannot run on
Windows and has no story for a packaged, versioned release. We need an
implementation language for the port (PLAN.md Milestone 2) that ships as a
single artifact with no runtime to install, works on all three desktop
platforms, handles subprocesses and signals robustly, and can be released
prebuilt for many targets.

## Decision

Implement freshen in Rust, shipped as a single static binary.

## Alternatives considered

- **Go** — a single static binary and cross-compilation are just as easy, but
  process spawning, signal handling and stream management are lower level and
  more error-prone than Rust's `std::process`/`tokio` ecosystem; GC pauses and
  runtime are irrelevant here but add nothing either. Cross was not chosen
  because Rust's ecosystem for subprocess/signal work (and this project's
  stated preference for it) is a better fit.
- **Python via `uv tool install`** — installable with a single `uv tool
  install freshen`, but requires the Python runtime and uv on the user's
  machine, is slower to start, and does not give a single static binary;
  Windows support is possible but weaker than a native binary. Not chosen
  because "no runtime to install" is a core requirement.

## Consequences

- Single static binary, no runtime to install on the target machine.
- First-class support for macOS, Linux and Windows from one codebase.
- Strong subprocess and signal handling (streaming output, timeouts,
  SIGINT/SIGTERM with restoration of stopped resources).
- Easy cross-compilation to prebuilt release targets.
- The zsh script `update-all.sh` stays as-is until Rust reaches parity, then
  is removed (PLAN.md Milestone 2).