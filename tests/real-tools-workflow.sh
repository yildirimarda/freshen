#!/usr/bin/env bash
# Verifies the real-tools CI proposal: it runs both read-only modes against the
# real Homebrew on macOS and fails on parser drift, not just on command errors.
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root" || exit 1
fail=0
ok()  { echo "ok    $1"; }
bad() { echo "FAIL  $1"; fail=1; }

workflow=""
for path in ci-proposals/real-tools.yml .github/workflows/real-tools.yml; do
  if [[ -f "$path" ]]; then
    workflow="$path"
    break
  fi
done

if [[ -z "$workflow" ]]; then
  bad "no real-tools workflow in ci-proposals/ or .github/workflows/"
  exit "$fail"
fi

static_ok=1
grep -qF 'runs-on: macos-latest' "$workflow" || { echo "  does not run on macos-latest"; static_ok=0; }
grep -qF -- '--check' "$workflow" || { echo "  does not run freshen --check"; static_ok=0; }
grep -qF -- '--dry-run' "$workflow" || { echo "  does not run freshen --dry-run"; static_ok=0; }
grep -qF -- '--json' "$workflow" || { echo "  does not request JSON reports"; static_ok=0; }
grep -qF 'brew,brew-formulae,brew-casks' "$workflow" || { echo "  does not select the Homebrew steps"; static_ok=0; }
grep -qF 'tests/real-tools-check.py check' "$workflow" || { echo "  does not validate the check report"; static_ok=0; }
grep -qF 'tests/real-tools-check.py dry-run' "$workflow" || { echo "  does not validate the dry-run report"; static_ok=0; }
if grep -v '^[[:space:]]*#' "$workflow" | grep -Eq 'brew[[:space:]]+(update|upgrade|install|uninstall|reinstall|autoremove|cleanup|prune)'; then
  echo "  runs a mutating brew command"
  static_ok=0
fi
if (( static_ok )); then
  ok "$workflow runs both read-only modes against real Homebrew on macOS"
else
  bad "$workflow is not a read-only real-Homebrew check"
fi

validator="tests/real-tools-check.py"
if [[ ! -f "$validator" ]]; then
  bad "tests/real-tools-check.py is missing"
  exit "$fail"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/check.json" <<'JSON'
{
  "failed": 0,
  "timed_out": 0,
  "steps": [
    {"id": "brew", "name": "Homebrew", "status": "up to date", "items": [], "error": null},
    {"id": "brew-formulae", "name": "Homebrew formulae", "status": "outdated",
     "items": [{"name": "jq", "current": "1.7.1", "latest": "1.8.0"}], "error": null},
    {"id": "brew-casks", "name": "Homebrew casks", "status": "outdated",
     "items": [{"name": "firefox", "current": "121.0", "latest": "122.0"}], "error": null},
    {"id": "brew-autoremove", "name": "Homebrew autoremove", "status": "up to date", "items": [], "error": null},
    {"id": "brew-cleanup", "name": "Homebrew cleanup", "status": "up to date", "items": [], "error": null},
    {"id": "brew-reopen", "name": "Homebrew reopen", "status": "up to date", "items": [], "error": null}
  ]
}
JSON

cat > "$tmp/dry-run.json" <<'JSON'
{
  "failed": 0,
  "timed_out": 0,
  "interrupted": 0,
  "was_interrupted": false,
  "steps": [
    {"id": "brew", "name": "Homebrew", "status": "planned", "error": null},
    {"id": "brew-formulae", "name": "Homebrew formulae", "status": "planned", "error": null},
    {"id": "brew-casks", "name": "Homebrew casks", "status": "planned", "error": null},
    {"id": "brew-autoremove", "name": "Homebrew autoremove", "status": "planned", "error": null},
    {"id": "brew-cleanup", "name": "Homebrew cleanup", "status": "planned", "error": null},
    {"id": "brew-reopen", "name": "Homebrew reopen", "status": "planned", "error": null}
  ]
}
JSON

expect_pass() {
  local mode="$1" file="$2" label="$3"
  if python3 "$validator" "$mode" "$file" >/dev/null 2>&1; then
    ok "$label"
  else
    bad "$label (validator rejected a valid report)"
  fi
}

expect_fail() {
  local mode="$1" file="$2" label="$3"
  if python3 "$validator" "$mode" "$file" >/dev/null 2>&1; then
    bad "$label (validator accepted an invalid report)"
  else
    ok "$label"
  fi
}

expect_pass check "$tmp/check.json" "validator accepts a clean check report"
expect_pass dry-run "$tmp/dry-run.json" "validator accepts a clean dry-run report"

sed 's/"failed": 0/"failed": 1/' "$tmp/check.json" > "$tmp/check-failed.json"
expect_fail check "$tmp/check-failed.json" "validator rejects a failed check report"

sed 's/"name": "jq"/"name": "jq (1.7.1 < 1.8.0"/' "$tmp/check.json" > "$tmp/check-drift.json"
expect_fail check "$tmp/check-drift.json" "validator rejects a line that did not parse"

sed 's/"status": "up to date"/"status": "skipped"/' "$tmp/check.json" > "$tmp/check-skipped.json"
expect_fail check "$tmp/check-skipped.json" "validator rejects a skipped Homebrew check"

cat > "$tmp/check-missing.json" <<'JSON'
{"failed": 0, "timed_out": 0, "steps": [
  {"id": "brew", "status": "up to date", "items": [], "error": null}
]}
JSON
expect_fail check "$tmp/check-missing.json" "validator rejects a missing Homebrew step"

sed 's/"status": "planned"/"status": "failed"/' "$tmp/dry-run.json" > "$tmp/dry-run-failed.json"
expect_fail dry-run "$tmp/dry-run-failed.json" "validator rejects a failed dry-run step"

sed 's/"status": "planned"/"status": "skipped"/' "$tmp/dry-run.json" > "$tmp/dry-run-skipped.json"
expect_fail dry-run "$tmp/dry-run-skipped.json" "validator rejects a skipped dry-run step"

exit "$fail"
