# Tests

`bash tests/run-tests.sh` runs the repository checks used by CI:

- `update-all.sh` passes `zsh -n` (skipped locally when zsh is not installed)
- every task line in `PLAN.md` uses the exact `- [ ] ` / `- [x] ` format that `run.sh` parses
- every `## Milestone N: name` heading is numbered consecutively

PLAN.md Milestone 2 adds a stub-based harness (fake `brew`, `uv`, `mise`, `docker`, `colima` on `PATH`) that asserts the order of calls. Tests must never invoke the real tools. Once the Rust workspace exists, unit and integration tests live in the Cargo workspace and this script keeps only the repository-level checks.
