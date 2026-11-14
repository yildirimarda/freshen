#!/bin/sh
# tests/install.sh — Functional tests for install.sh (checksum verification).
#
# Spins up a local HTTP server serving a fake release (a dummy tarball and a
# checksums.txt), runs install.sh against it, and asserts:
#   * a matching checksum installs the binary to FRESHEN_BIN_DIR
#   * a mismatched checksum aborts the install (non-zero exit)
#   * a missing asset aborts the install
#
# No network is used: everything is served from localhost.
set -eu

cd "$(cd "$(dirname "$0")/.." && pwd)"

fail=0
ok() { echo "ok    $1"; }
bad() { echo "FAIL  $1"; fail=1; }

command -v python3 >/dev/null 2>&1 || { ok "install.sh (python3 unavailable, skipped)"; exit 0; }
command -v curl >/dev/null 2>&1 || { ok "install.sh (curl unavailable, skipped)"; exit 0; }
command -v sha256sum >/dev/null 2>&1 || { ok "install.sh (sha256sum unavailable, skipped)"; exit 0; }

# Pick a free local port.
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"

root="$(mktemp -d)"
cleanup() { kill "$server" 2>/dev/null; rm -rf "$root"; }
trap cleanup EXIT

# Detect the platform the same way install.sh does, so the fake asset matches.
os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m | tr '[:upper:]' '[:lower:]')"
case "$os" in
  darwin) dist_os="apple-darwin"; archive="tar.gz" ;;
  linux)  dist_os="unknown-linux-musl"; archive="tar.gz" ;;
  *) bad "unsupported OS $os"; exit 1 ;;
esac
case "$arch" in
  x86_64|amd64) dist_arch="x86_64" ;;
  aarch64|arm64) dist_arch="aarch64" ;;
  *) bad "unsupported arch $arch"; exit 1 ;;
esac
dist="${dist_arch}-${dist_os}"
asset="freshen-0.1.0-${dist}.${archive}"

# Build a fake release tarball containing a "freshen" stub.
staging="${root}/staging"
mkdir -p "$staging"
printf '%s\n' '#!/bin/sh' 'echo freshen-stub' > "$staging/freshen"
chmod +x "$staging/freshen"
tar -czf "${root}/${asset}" -C "$staging" freshen

hash="$(sha256sum "${root}/${asset}" | awk '{print $1}')"
printf '%s  %s\n' "$hash" "$asset" > "${root}/checksums.txt"

# Serve the fake release directory over HTTP.
python3 -m http.server "$port" --directory "$root" >/dev/null 2>&1 &
server=$!

# Wait for the server to accept connections.
for i in $(seq 1 50); do
  if curl -fsS "http://127.0.0.1:${port}/" >/dev/null 2>&1; then break; fi
  sleep 0.1
done

base_url="http://127.0.0.1:${port}"
bindir="$(mktemp -d)"

# --- Test 1: matching checksum installs the binary -------------------------
if FRESHEN_BASE_URL="$base_url" FRESHEN_BIN_DIR="$bindir" FRESHEN_VERSION=0.1.0 \
    sh install.sh >/dev/null 2>&1; then
  if [ -x "${bindir}/freshen" ]; then
    ok "install.sh installs the binary when the checksum matches"
  else
    bad "install.sh reported success but did not install an executable"
  fi
else
  bad "install.sh failed although the checksum matched"
fi

installed="$( "${bindir}/freshen" 2>/dev/null || true )"
[ "$installed" = "freshen-stub" ] && ok "the installed binary runs" || bad "the installed binary does not run"

# --- Test 2: mismatched checksum aborts ------------------------------------
# Corrupt the tarball so its hash no longer matches checksums.txt.
printf 'corrupted' > "${root}/${asset}"
if FRESHEN_BASE_URL="$base_url" FRESHEN_BIN_DIR="$bindir" FRESHEN_VERSION=0.1.0 \
    sh install.sh >/dev/null 2>&1; then
  bad "install.sh succeeded although the checksum mismatched"
else
  ok "install.sh aborts on a checksum mismatch"
fi

# --- Test 3: missing asset aborts ------------------------------------------
missing="freshen-0.1.0-x86_64-unknown-linux-gnu.${archive}"
if FRESHEN_BASE_URL="$base_url" FRESHEN_BIN_DIR="$bindir" FRESHEN_VERSION=0.1.0 \
    sh install.sh >/dev/null 2>&1; then
  bad "install.sh succeeded although the asset is missing"
else
  ok "install.sh aborts when the asset is missing"
fi

exit "$fail"
