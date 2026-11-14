#!/bin/sh
# install.sh — One-line installer for freshen with SHA-256 checksum verification.
#
# One-liner:
#   curl -fsSL https://raw.githubusercontent.com/yildirimarda/freshen/main/install.sh | sh
#
# Detects the operating system and architecture, downloads the matching
# prebuilt freshen binary from the GitHub release, verifies its SHA-256
# checksum against the release's checksums.txt, and installs it to a directory
# on PATH (default /usr/local/bin).
#
# The release artifacts and checksums.txt are produced by
# ci-proposals/release-binaries.yml (assets named
# freshen-<version>-<target>.tar.gz on Unix and .zip on Windows).
#
# Environment overrides (for testing and custom installs):
#   FRESHEN_REPO     GitHub repository (default yildirimarda/freshen)
#   FRESHEN_VERSION  Release tag or "latest" (default latest)
#   FRESHEN_BASE_URL Base download URL (default derived from repo/version)
#   FRESHEN_BIN_DIR  Install directory (default /usr/local/bin)
#   FRESHEN_TMP_DIR  Working directory (default a fresh mktemp dir)
set -eu

VERSION="${FRESHEN_VERSION:-latest}"
REPO="${FRESHEN_REPO:-yildirimarda/freshen}"
BIN_DIR="${FRESHEN_BIN_DIR:-/usr/local/bin}"
TMP_DIR="${FRESHEN_TMP_DIR:-$(mktemp -d)}"

if [ -n "${FRESHEN_BASE_URL:-}" ]; then
  BASE_URL="$FRESHEN_BASE_URL"
else
  tag="$VERSION"
  [ "$VERSION" = "latest" ] && tag="latest"
  BASE_URL="https://github.com/${REPO}/releases/download/${tag}"
fi

sha256_of() {
  file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  elif command -v sha256 >/dev/null 2>&1; then
    sha256 -q "$file"
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$file" | awk '{print $NF}'
  else
    echo "freshen: no SHA-256 tool available (install coreutils or openssl)" >&2
    exit 1
  fi
}

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# --- detect operating system and architecture --------------------------------
os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m | tr '[:upper:]' '[:lower:]')"

case "$os" in
  darwin) dist_os="apple-darwin"; archive="tar.gz" ;;
  linux)  dist_os="unknown-linux-musl"; archive="tar.gz" ;;
  *)
    echo "freshen: unsupported operating system: $os" >&2
    exit 1
    ;;
esac

case "$arch" in
  x86_64|amd64) dist_arch="x86_64" ;;
  aarch64|arm64) dist_arch="aarch64" ;;
  *)
    echo "freshen: unsupported architecture: $arch" >&2
    exit 1
    ;;
esac

dist="${dist_arch}-${dist_os}"
asset="freshen-${VERSION}-${dist}.${archive}"

# --- download the binary and the release checksums file ----------------------
echo "freshen: downloading ${asset}"
curl -fsSL --retry 3 --connect-timeout 10 "${BASE_URL}/${asset}" -o "${TMP_DIR}/${asset}"
curl -fsSL --retry 3 --connect-timeout 10 "${BASE_URL}/checksums.txt" -o "${TMP_DIR}/checksums.txt"

# --- verify the SHA-256 checksum before touching the system ------------------
expected="$(grep -F "${asset}" "${TMP_DIR}/checksums.txt" | head -n 1 | awk '{print $1}')"
if [ -z "$expected" ]; then
  echo "freshen: no checksum for ${asset} in checksums.txt" >&2
  exit 1
fi

actual="$(sha256_of "${TMP_DIR}/${asset}")"
if [ "$actual" != "$expected" ]; then
  echo "freshen: SHA-256 checksum mismatch for ${asset}" >&2
  echo "  expected: $expected" >&2
  echo "  got:      $actual" >&2
  exit 1
fi

# --- extract and install -----------------------------------------------------
case "$archive" in
  tar.gz)
    tar -xzf "${TMP_DIR}/${asset}" -C "$TMP_DIR"
    binary="${TMP_DIR}/freshen"
    ;;
  *)
    echo "freshen: unsupported archive type: $archive" >&2
    exit 1
    ;;
esac

mkdir -p "$BIN_DIR"
install -m 0755 "$binary" "${BIN_DIR}/freshen"

echo "freshen: installed ${VERSION} to ${BIN_DIR}/freshen"
echo "Run: freshen --help"
