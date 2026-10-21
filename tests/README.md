# Tests

`bash tests/run-tests.sh` runs the repository checks used by CI:

- the legacy zsh implementation and its deprecated `update-all.sh`
  compatibility shim are gone
- `README.md` documents the `update-all.sh` → `freshen` migration and
  `CHANGELOG.md` records the removal
- the CI workflow proposal replaces the old zsh syntax step with a guard that
  the `update-all.sh` shim stays removed
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
  `advisories` section, and the `cargo-deny` job in the rust workflow
  (`ci-proposals/rust.yml`, or `.github/workflows/rust.yml` once a human
  applies it) runs both `cargo deny check licenses` and
  `cargo deny check advisories`
- the `msrv` job in the rust workflow reads the workspace `rust-version` and
  runs `cargo check --workspace --all-targets` with that toolchain, so a newer
  language feature fails before release
- the rust workflow's lint, unit test and stub-based integration test steps run
  in a matrix on `ubuntu-latest`, `macos-latest` and `windows-latest`, so the
  platform-specific process, signal and lock behaviour is exercised on every
  supported operating system
- the `real-tools.yml` workflow runs `freshen --check` and `freshen --dry-run`
  read-only against the real Homebrew on `macos-latest`, validates both JSON
  reports with `tests/real-tools-check.py`, and fails when a Homebrew parser
  reports an error, skips a step or turns an unrecognised line into a bogus
  package name; `tests/real-tools-workflow.sh` proves the validator accepts
  clean reports and rejects each of those failures
- every CI workflow is accepted either as a proposal in `ci-proposals/` or
  after being applied to `.github/workflows/`, including `real-tools.yml`;
  `tests/workflow-layouts.sh` runs the repository checks against a fixture in
  both layouts to prove that applying a proposal cannot break them

Unit and integration tests for the Rust implementation live in the Cargo
workspace (`cargo test --all-features`); this script keeps only the
repository-level checks. The stub-based integration harness lives in
`crates/freshen/tests/`: it runs the built binary with temporary stub `brew`,
`uv`, `mise`, `docker` and `colima` executables on `PATH` and asserts the order
they were called. The binary is built with the `test-harness` feature, so run
the Rust tests with `--all-features`. Tests must never invoke the real tools.
