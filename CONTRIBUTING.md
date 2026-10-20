# Contributing to freshen

Thanks for your interest! The project is small and plan-driven.

## Workflow

1. Pick an unchecked item from [PLAN.md](PLAN.md) or open an issue describing what you want to add.
2. Fork, create a branch (`feat/short-name`) and make your change.
3. Every step must be skipped gracefully when its tool is missing, and must declare the platforms it supports. The implementation is the Rust workspace under `crates/` (`freshen-lib` holds all logic, `freshen` is a thin CLI wrapper); the deprecated `update-all.sh` shim has been removed, so use `freshen` directly.
4. Run the checks below.
5. Commit with [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `chore:`, `test:`) and open a pull request.

## Checks

```bash
cargo fmt --all -- --check                             # Rust phase
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
bash tests/run-tests.sh                                # repository checks
```

## Style

- All comments and output messages in English.
- Rust: no `unwrap()`/`expect()` outside tests, OS-specific code behind `cfg(target_os)`.
- Never add steps that delete user data without an explicit opt-in flag.

## Reporting bugs

Please include macOS version, `brew --version`, your OS and version, which optional tools you use (colima, uv, mise, ...) and the output of the failing run.
