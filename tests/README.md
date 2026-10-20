# Tests

`bash tests/run-tests.sh` runs the repository checks used by CI:

- `update-all.sh` passes `zsh -n` (skipped locally when zsh is not installed)
- every task line in `PLAN.md` uses the exact `- [ ] ` / `- [x] ` format that `run.sh` parses
- every `## Milestone N: name` heading is numbered consecutively
- every ADR under `docs/adr/` is numbered consecutively from `0000`, carries a
  valid `Status`/`Date` and the four template sections
  (`Context`/`Decision`/`Alternatives considered`/`Consequences`), and is
  linked from `docs/adr/README.md`
- once the Cargo workspace exists (`Cargo.toml` at the root): the workspace
  sets an MSRV (`rust-version`), `rust-toolchain.toml` pins an exact channel
  with `rustfmt`/`clippy`, and every crate under `crates/` inherits the
  workspace MSRV and lints

Unit and integration tests for the Rust implementation live in the Cargo
workspace (`cargo test --all-features`); this script keeps only the
repository-level checks. PLAN.md Milestone 2 adds a stub-based harness (fake
`brew`, `uv`, `mise`, `docker`, `colima` on `PATH`) that asserts the order of
calls. Tests must never invoke the real tools.
