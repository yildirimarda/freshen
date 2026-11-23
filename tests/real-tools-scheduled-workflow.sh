#!/usr/bin/env bash
# Verifies the scheduled real-tools CI proposal: it runs on a nightly schedule,
# refreshes real package-manager metadata (`brew update`, `apt-get update`,
# `winget source update`) before checking, runs both read-only modes against
# the real Homebrew on macOS, the real apt on Ubuntu and the real winget and
# Chocolatey on Windows, and fails on parser drift, not just on command errors.
# It is deliberately not a required check, so an upstream output change cannot
# block a pull request.
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root" || exit 1
fail=0
ok()  { echo "ok    $1"; }
bad() { echo "FAIL  $1"; fail=1; }

workflow=""
for path in ci-proposals/real-tools-scheduled.yml .github/workflows/real-tools-scheduled.yml; do
  if [[ -f "$path" ]]; then
    workflow="$path"
    break
  fi
done

if [[ -z "$workflow" ]]; then
  bad "no real-tools-scheduled workflow in ci-proposals/ or .github/workflows/"
  exit "$fail"
fi

static_ok=1
for runner in macos-latest ubuntu-latest windows-latest; do
  grep -qF "runs-on: $runner" "$workflow" || { echo "  does not run on $runner"; static_ok=0; }
done
grep -q 'schedule:' "$workflow" || { echo "  no schedule trigger"; static_ok=0; }
grep -qF 'cron: "0 4 * * *"' "$workflow" || { echo "  no nightly cron"; static_ok=0; }
grep -qF -- '--check' "$workflow" || { echo "  does not run freshen --check"; static_ok=0; }
grep -qF -- '--dry-run' "$workflow" || { echo "  does not run freshen --dry-run"; static_ok=0; }
grep -qF -- '--json' "$workflow" || { echo "  does not request JSON reports"; static_ok=0; }
grep -qF -- '--only' "$workflow" || { echo "  does not select a package manager step"; static_ok=0; }
grep -qF 'brew,brew-formulae,brew-casks' "$workflow" || { echo "  does not select the Homebrew steps"; static_ok=0; }
grep -qF 'APT_STEPS: apt' "$workflow" || { echo "  does not select the apt step"; static_ok=0; }
grep -qF 'WINDOWS_STEPS: winget,chocolatey' "$workflow" || { echo "  does not select the winget and Chocolatey steps"; static_ok=0; }
grep -qF 'brew update' "$workflow" || { echo "  does not refresh Homebrew metadata"; static_ok=0; }
grep -qF 'apt-get update' "$workflow" || { echo "  does not refresh apt metadata"; static_ok=0; }
grep -qF 'winget source update' "$workflow" || { echo "  does not refresh winget metadata"; static_ok=0; }
grep -qF 'tests/real-tools-check.py check' "$workflow" || { echo "  does not validate the check report"; static_ok=0; }
grep -qF 'tests/real-tools-check.py dry-run' "$workflow" || { echo "  does not validate the dry-run report"; static_ok=0; }
grep -qF -- '--profile apt' "$workflow" || { echo "  does not validate the apt report"; static_ok=0; }
grep -qF -- '--profile windows' "$workflow" || { echo "  does not validate the winget/Chocolatey report"; static_ok=0; }

# Metadata refresh is intentional here, so allow it. Reject only the mutating
# package-manager commands that `--check`/`--dry-run` must never run.
if grep -v '^[[:space:]]*#' "$workflow" | grep -Eq \
  'brew[[:space:]]+(upgrade|install|uninstall|reinstall|autoremove|cleanup|prune)|apt(-get)?[[:space:]]+(upgrade|dist-upgrade|full-upgrade|install|remove|purge|autoremog|autoclean|clean)|choco[[:space:]]+(upgrade|install|uninstall)|winget[[:space:]]+(upgrade|install|uninstall)'; then
  echo "  runs a mutating package-manager command"
  static_ok=0
fi

# Not a required check: a scheduled job must not carry `required` gating that
# would block merges, and must not be triggered only on pull_request.
if grep -qF 'required:' "$workflow"; then
  echo "  marks a job as required (a scheduled job must never block merges)"
  static_ok=0
fi
if grep -Eq '^on:[[:space:]]*pull_request[[:space:]]*$' "$workflow" && ! grep -q 'schedule:' "$workflow"; then
  echo "  runs only on pull_request, not on a schedule"
  static_ok=0
fi

if (( static_ok )); then
  ok "$workflow runs both read-only modes against real Homebrew, apt, winget and Chocolatey on a nightly schedule with refreshed metadata"
else
  bad "$workflow is not a scheduled read-only real-tools check"
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
    {"id": "brew-doctor", "name": "Homebrew doctor", "status": "up to date", "items": [], "error": null},
    {"id": "brew-bundle-cleanup", "name": "Homebrew bundle cleanup", "status": "up to date", "items": [], "error": null},
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
    {"id": "brew-doctor", "name": "Homebrew doctor", "status": "planned", "error": null},
    {"id": "brew-bundle-cleanup", "name": "Homebrew bundle cleanup", "status": "planned", "error": null},
    {"id": "brew-reopen", "name": "Homebrew reopen", "status": "planned", "error": null}
  ]
}
JSON

cat > "$tmp/apt-check.json" <<'JSON'
{
  "failed": 0,
  "timed_out": 0,
  "steps": [
    {"id": "apt", "name": "apt", "status": "outdated",
     "items": [{"name": "libc6:amd64", "current": "2.39-0ubuntu8.1", "latest": "2.39-0ubuntu8.2"}],
     "error": null}
  ]
}
JSON

cat > "$tmp/apt-dry-run.json" <<'JSON'
{
  "failed": 0,
  "timed_out": 0,
  "interrupted": 0,
  "was_interrupted": false,
  "steps": [
    {"id": "apt", "name": "apt", "status": "planned", "error": null}
  ]
}
JSON

cat > "$tmp/windows-check.json" <<'JSON'
{
  "failed": 0,
  "timed_out": 0,
  "steps": [
    {"id": "winget", "name": "winget", "status": "outdated",
     "items": [{"name": "Microsoft.VisualStudioCode", "current": "1.90.0", "latest": "1.91.0"}],
     "error": null},
    {"id": "chocolatey", "name": "Chocolatey", "status": "up to date", "items": [], "error": null}
  ]
}
JSON

cat > "$tmp/windows-dry-run.json" <<'JSON'
{
  "failed": 0,
  "timed_out": 0,
  "interrupted": 0,
  "was_interrupted": false,
  "steps": [
    {"id": "winget", "name": "winget", "status": "planned", "error": null},
    {"id": "chocolatey", "name": "Chocolatey", "status": "planned", "error": null}
  ]
}
JSON

expect_pass() {
  local mode="$1" file="$2" label="$3"
  shift 3
  if python3 "$validator" "$mode" "$file" "$@" >/dev/null 2>&1; then
    ok "$label"
  else
    bad "$label (validator rejected a valid report)"
  fi
}

expect_fail() {
  local mode="$1" file="$2" label="$3"
  shift 3
  if python3 "$validator" "$mode" "$file" "$@" >/dev/null 2>&1; then
    bad "$label (validator accepted an invalid report)"
  else
    ok "$label"
  fi
}

expect_pass check "$tmp/check.json" "validator accepts a clean check report"
expect_pass dry-run "$tmp/dry-run.json" "validator accepts a clean dry-run report"
expect_pass check "$tmp/apt-check.json" "validator accepts a clean apt check report" --profile apt
expect_pass dry-run "$tmp/apt-dry-run.json" "validator accepts a clean apt dry-run report" --profile apt
expect_pass check "$tmp/windows-check.json" "validator accepts a clean winget/Chocolatey check report" --profile windows
expect_pass dry-run "$tmp/windows-dry-run.json" "validator accepts a clean winget/Chocolatey dry-run report" --profile windows

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

sed 's/"status": "outdated"/"status": "skipped"/' "$tmp/apt-check.json" > "$tmp/apt-check-skipped.json"
expect_fail check "$tmp/apt-check-skipped.json" "validator rejects a skipped apt check" --profile apt

sed 's/"name": "libc6:amd64"/"name": "libc6:amd64 (2.39 < 2.40"/' "$tmp/apt-check.json" > "$tmp/apt-check-drift.json"
expect_fail check "$tmp/apt-check-drift.json" "validator rejects an apt line that did not parse" --profile apt

cat > "$tmp/apt-check-missing.json" <<'JSON'
{"failed": 0, "timed_out": 0, "steps": []}
JSON
expect_fail check "$tmp/apt-check-missing.json" "validator rejects a missing apt step" --profile apt

sed 's/"status": "outdated"/"status": "failed"/' "$tmp/windows-check.json" > "$tmp/windows-check-failed.json"
expect_fail check "$tmp/windows-check-failed.json" "validator rejects a failed winget/Chocolatey check" --profile windows

cat > "$tmp/windows-check-missing.json" <<'JSON'
{"failed": 0, "timed_out": 0, "steps": [
  {"id": "winget", "status": "up to date", "items": [], "error": null}
]}
JSON
expect_fail check "$tmp/windows-check-missing.json" "validator rejects a missing Chocolatey step" --profile windows

sed 's/"status": "planned"/"status": "skipped"/' "$tmp/windows-dry-run.json" > "$tmp/windows-dry-run-skipped.json"
expect_fail dry-run "$tmp/windows-dry-run-skipped.json" "validator rejects a skipped winget/Chocolatey dry-run step" --profile windows

exit "$fail"
