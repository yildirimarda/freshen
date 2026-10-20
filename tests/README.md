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
- `deny.toml` (cargo-deny) is valid TOML with a `licenses` allow-list and an
  `advisories` section, and the `cargo-deny` job in `ci-proposals/rust.yml`
  runs both `cargo deny check licenses` and `cargo deny check advisories`

Unit and integration tests for the Rust implementation live in the Cargo
workspace (`cargo test --all-features`); this script keeps only the
repository-level checks. The stub-based integration harness lives in
`crates/freshen/tests/`: it runs the built binary with temporary stub `brew`,
`uv`, `mise`, `docker` and `colima` executables on `PATH` and asserts the order
they were called. The binary is built with the `test-harness` feature, so run
the Rust tests with `--all-features`. Tests must never invoke the real tools.
