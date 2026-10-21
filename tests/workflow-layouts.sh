#!/usr/bin/env bash
# Verifies that tests/run-tests.sh accepts CI workflows both as proposals in
# ci-proposals/ and after a human applies them to .github/workflows/ with
# `git mv`. The fixture keeps only the files the repository checks read.
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0

make_fixture() {
  local dir="$1" layout="$2" name src dest
  mkdir -p "$dir/tests" "$dir/ci-proposals" "$dir/.github/workflows"
  cp "$root/tests/run-tests.sh" "$dir/tests/run-tests.sh"
  cp "$root/Cargo.toml" "$dir/Cargo.toml"
  cp "$root/deny.toml" "$dir/deny.toml"
  cp "$root/PLAN.md" "$dir/PLAN.md"
  cat > "$dir/README.md" <<'MD'
# freshen

## Migrating from update-all.sh

Use freshen instead.
MD
  cat > "$dir/CHANGELOG.md" <<'MD'
## [Unreleased]

- Removed the zsh implementation and the compatibility shim.
MD

  for name in rust ci real-tools; do
    src="$root/ci-proposals/$name.yml"
    [[ -f "$src" ]] || src="$root/.github/workflows/$name.yml"
    if [[ "$layout" == proposals ]]; then
      dest="$dir/ci-proposals/$name.yml"
    else
      dest="$dir/.github/workflows/$name.yml"
    fi
    if [[ -f "$src" ]]; then
      cp "$src" "$dest"
    else
      # real-tools.yml is optional in the repository today; a minimal workflow
      # still proves that the layout resolver handles the name.
      printf 'name: %s\non: [push]\njobs: {}\n' "$name" > "$dest"
    fi
  done
}

for layout in proposals applied; do
  dir="$tmp/$layout"
  make_fixture "$dir" "$layout"
  if out="$(cd "$dir" && RUN_TESTS_SKIP_WORKFLOW_LAYOUTS=1 bash tests/run-tests.sh 2>&1)"; then
    echo "ok    run-tests.sh passes with workflows in $layout layout"
  else
    echo "FAIL  run-tests.sh fails with workflows in $layout layout"
    printf '%s\n' "$out" | sed 's/^/  /'
    fail=1
  fi
  if ! grep -q 'real-tools workflow found at' <<<"$out"; then
    echo "FAIL  run-tests.sh did not locate the real-tools workflow in $layout layout"
    fail=1
  fi
done

exit "$fail"
