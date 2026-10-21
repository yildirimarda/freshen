#!/usr/bin/env bash
# Repository-level checks. Must stay free of side effects: never calls the
# real brew/docker/uv/mise.
set -uo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail=0
ok()  { echo "ok    $1"; }
bad() { echo "FAIL  $1"; fail=1; }

# Workflows are proposed under ci-proposals/ until a human applies them with
# `git mv` to .github/workflows/. Accept either location for every workflow so
# applying a proposal never breaks these checks.
workflow_path() {
  local name="$1" path
  for path in "ci-proposals/$name.yml" ".github/workflows/$name.yml"; do
    if [[ -f "$path" ]]; then
      printf '%s\n' "$path"
      return 0
    fi
  done
  return 1
}

# 1. The legacy zsh implementation and its deprecated sh compatibility shim
#    are gone.
if [[ ! -e update-all.sh ]]; then
  ok "update-all.sh compatibility shim removed"
else
  bad "update-all.sh compatibility shim is still present"
fi

# 2. The migration is documented where users look for it, and the docs no
#    longer promise that the removed shim is still available.
if grep -q '^## Migrating from update-all.sh' README.md && grep -q 'freshen' README.md; then
  if grep -q 'remains as a deprecated' README.md; then
    bad "README.md still advertises the removed update-all.sh shim"
  else
    ok "README.md documents the migration to freshen"
  fi
else
  bad "README.md is missing the update-all.sh migration section"
fi
if grep -q '^## \[Unreleased\]' CHANGELOG.md && grep -qi 'zsh implementation' CHANGELOG.md && grep -qi 'compatibility shim' CHANGELOG.md; then
  ok "CHANGELOG.md records the zsh and shim removal"
else
  bad "CHANGELOG.md is missing the zsh/shim removal entry"
fi

# 3. PLAN.md format: lines that look like tasks must match the exact format
if [[ -f PLAN.md ]]; then
  malformed="$(grep -nE '^[[:space:]]*-[[:space:]]*\[' PLAN.md | grep -vE '^[0-9]+:- \[( |x)\] .+' || true)"
  if [[ -z "$malformed" ]]; then ok "PLAN.md task lines"; else bad "PLAN.md malformed task lines:"; echo "$malformed"; fi

  n=0; seq_ok=1
  while IFS= read -r num; do
    if [[ "$num" != "$n" ]]; then seq_ok=0; fi
    n=$((n + 1))
  done < <(grep -oE '^## Milestone [0-9]+:' PLAN.md | grep -oE '[0-9]+')
  if (( seq_ok )); then ok "PLAN.md milestones numbered 0..$((n - 1))"; else bad "PLAN.md milestone numbering is not consecutive"; fi
fi

# 4. ADR files: valid number, template sections, and the index table links them
if [[ -d docs/adr ]]; then
  n=0; adr_ok=1
  for f in docs/adr/[0-9][0-9][0-9][0-9]-*.md; do
    [[ -e "$f" ]] || continue
    num="${f#docs/adr/}"; num="${num%%-*}"
    if [[ "$num" == "0000" ]]; then
      n=1; continue   # 0000 is the template placeholder; real ADRs start at 0001
    fi
    if [[ "$num" != "$(printf '%04d' "$n")" ]]; then adr_ok=0; echo "  $f is not numbered $n"; fi
    for section in '## Context' '## Decision' '## Alternatives considered' '## Consequences'; do
      grep -q "^${section}" "$f" || { adr_ok=0; echo "  $f missing section: $section"; }
    done
    grep -qE '^- Status: (proposed|accepted|superseded by [0-9]{4})' "$f" || { adr_ok=0; echo "  $f missing/invalid Status line"; }
    grep -qE '^- Date: [0-9]{4}-[0-9]{2}-[0-9]{2}' "$f" || { adr_ok=0; echo "  $f missing/invalid Date line"; }
    grep -q "$(basename "$f")" docs/adr/README.md || { adr_ok=0; echo "  $f not linked from docs/adr/README.md"; }
    n=$((n + 1))
  done
  if (( adr_ok )); then ok "docs/adr files (numbering, sections, index links)"; else bad "docs/adr file checks"; fi
fi

# 5. cargo-deny: deny.toml is a valid TOML with a licenses allow-list and an
#    advisories section, and the CI proposal actually runs both checks
if [[ -f deny.toml ]]; then
  deny_ok=1
  python3 - <<'PY' >/dev/null 2>&1 || deny_ok=0
import sys, tomllib
try:
    with open("deny.toml", "rb") as f:
        cfg = tomllib.load(f)
    assert "licenses" in cfg and "allow" in cfg["licenses"] and cfg["licenses"]["allow"]
    assert "advisories" in cfg and "ignore" in cfg["advisories"]
except Exception:
    sys.exit(1)
PY
  [[ "$deny_ok" == "1" ]] || { bad "deny.toml is not valid / missing licenses.allow or advisories"; }
  if (( deny_ok )); then ok "deny.toml (valid TOML, licenses allow-list, advisories)"; fi

  prop_ok=1
  rust_file="$(workflow_path rust)"
  if [[ -n "$rust_file" ]]; then
    grep -q 'cargo-deny:' "$rust_file" || { echo "  no cargo-deny job"; prop_ok=0; }
    grep -q 'cargo deny check licenses' "$rust_file" || { echo "  missing licenses check"; prop_ok=0; }
    grep -q 'cargo deny check advisories' "$rust_file" || { echo "  missing advisories check"; prop_ok=0; }
    if (( prop_ok )); then ok "$rust_file cargo-deny job (licenses + advisories)"; else bad "$rust_file missing a cargo-deny job running both checks"; fi
  else
    bad "deny.toml present but no rust workflow in ci-proposals/ or .github/workflows/"
  fi
fi

# 6. MSRV job: the CI proposal reads the workspace rust-version and checks the
#    whole workspace including all test targets with that exact toolchain, so a
#    newer language feature cannot slip in unnoticed.
rust_file="$(workflow_path rust)"
if [[ -f Cargo.toml && -n "$rust_file" ]]; then
  msrv_ok=1
  msrv="$(sed -n 's/^rust-version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' Cargo.toml | head -n1)"
  [[ -n "$msrv" ]] || { echo "  Cargo.toml has no rust-version"; msrv_ok=0; }
  grep -q '^  msrv:' "$rust_file" || { echo "  no msrv job"; msrv_ok=0; }
  grep -q 'rust-version' "$rust_file" || { echo "  msrv job does not read rust-version"; msrv_ok=0; }
  grep -q 'steps.msrv.outputs.version' "$rust_file" || { echo "  msrv job does not use the manifest value"; msrv_ok=0; }
  grep -q 'cargo +.*check --workspace --all-targets' "$rust_file" || { echo "  msrv job does not run cargo check --workspace --all-targets"; msrv_ok=0; }
  if (( msrv_ok )); then ok "$rust_file MSRV job (rust-version -> cargo check --workspace --all-targets)"; else bad "$rust_file missing an MSRV check job"; fi
fi

# 7. CI replaces the old zsh syntax step with a guard that the compatibility
#    shim stays removed. Check the proposal until a human applies it, then the
#    active workflow.
ci_file="$(workflow_path ci)"
if [[ -n "$ci_file" ]]; then
  ci_ok=1
  grep -q 'test ! -e update-all.sh' "$ci_file" || { echo "  no shim-removal guard"; ci_ok=0; }
  if grep -qE 'zsh -n|Zsh syntax' "$ci_file"; then echo "  still runs a zsh syntax check"; ci_ok=0; fi
  if (( ci_ok )); then ok "$ci_file checks that the update-all.sh shim is gone"; else bad "$ci_file does not replace the zsh syntax step with a shim check"; fi
else
  bad "no CI workflow proposal or active workflow found"
fi

# 8. The real-tools workflow runs freshen's read-only modes against the real
#    Homebrew on macOS and fails on parser drift, not just on command errors.
#    It is accepted either as a proposal or after a human applies it.
if real_tools_file="$(workflow_path real-tools)"; then
  ok "real-tools workflow found at $real_tools_file"
  if [[ -f tests/real-tools-workflow.sh ]]; then
    bash tests/real-tools-workflow.sh || fail=1
  else
    bad "tests/real-tools-workflow.sh is missing"
  fi
else
  bad "no real-tools workflow in ci-proposals/ or .github/workflows/"
fi

# 9. CI matrix: lint, unit tests and the stub-based integration tests must run
#    on macOS, Ubuntu and Windows, so platform-specific process, signal and
#    lock behaviour is exercised instead of only the Linux code path.
rust_file="$(workflow_path rust)"
if [[ -f Cargo.toml && -n "$rust_file" ]]; then
  matrix_ok=1
  grep -qF 'matrix:' "$rust_file" || { echo "  no CI matrix"; matrix_ok=0; }
  for runner in ubuntu-latest macos-latest windows-latest; do
    grep -qF "$runner" "$rust_file" || { echo "  matrix does not include $runner"; matrix_ok=0; }
  done
  grep -qF 'runs-on: ${{ matrix.os }}' "$rust_file" || { echo "  job does not run on the matrix OS"; matrix_ok=0; }
  grep -qF 'cargo fmt --all -- --check' "$rust_file" || { echo "  no fmt lint step"; matrix_ok=0; }
  grep -qF 'cargo clippy --all-targets --all-features -- -D warnings' "$rust_file" || { echo "  no clippy lint step"; matrix_ok=0; }
  grep -qF 'cargo test --all-features --lib --bins' "$rust_file" || { echo "  no unit test step"; matrix_ok=0; }
  grep -qF 'cargo test --all-features --tests' "$rust_file" || { echo "  no stub-based integration test step"; matrix_ok=0; }
  if (( matrix_ok )); then
    ok "$rust_file CI matrix (macOS, Ubuntu, Windows: lint, unit and integration tests)"
  else
    bad "$rust_file does not run lint, unit and stub-based integration tests on all three runners"
  fi
fi

# 10. Prove the layout rules above: run this script against a fixture with every
#     workflow in ci-proposals/ and again with them in .github/workflows/.
if [[ "${RUN_TESTS_SKIP_WORKFLOW_LAYOUTS:-}" != "1" && -f tests/workflow-layouts.sh ]]; then
  bash tests/workflow-layouts.sh || fail=1
fi

exit "$fail"
