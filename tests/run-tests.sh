#!/usr/bin/env bash
# Repository-level checks. Must stay free of side effects: never calls the
# real brew/docker/uv/mise.
set -uo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail=0
ok()  { echo "ok    $1"; }
bad() { echo "FAIL  $1"; fail=1; }

# 1. zsh syntax of the legacy script
if [[ -f update-all.sh ]]; then
  if command -v zsh >/dev/null 2>&1; then
    zsh -n update-all.sh && ok "zsh -n update-all.sh" || bad "zsh -n update-all.sh"
  else
    echo "skip  zsh not installed, syntax check skipped"
  fi
fi

# 2. PLAN.md format: lines that look like tasks must match the exact format
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

# 3. ADR files: valid number, template sections, and the index table links them
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

# 4. cargo-deny: deny.toml is a valid TOML with a licenses allow-list and an
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
  if [[ -f ci-proposals/rust.yml ]]; then
    grep -q 'cargo-deny:' ci-proposals/rust.yml || { echo "  no cargo-deny job"; prop_ok=0; }
    grep -q 'cargo deny check licenses' ci-proposals/rust.yml || { echo "  missing licenses check"; prop_ok=0; }
    grep -q 'cargo deny check advisories' ci-proposals/rust.yml || { echo "  missing advisories check"; prop_ok=0; }
    if (( prop_ok )); then ok "ci-proposals/rust.yml cargo-deny job (licenses + advisories)"; else bad "ci-proposals/rust.yml missing a cargo-deny job running both checks"; fi
  else
    bad "deny.toml present but ci-proposals/rust.yml missing"
  fi
fi

exit "$fail"
