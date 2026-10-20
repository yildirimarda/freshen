#!/bin/sh
# Deprecated compatibility wrapper for the Rust `freshen` binary.
#
# The zsh implementation was removed once the Rust port reached parity. This
# wrapper keeps existing aliases and scripts working for one release: it maps
# the old environment variables to flags and forwards every argument.
#
# Use `freshen` directly; see README.md ("Migrating from update-all.sh").

set -eu

if [ "${ASSUME_YES:-0}" = "1" ]; then
    set -- --yes "$@"
fi
if [ "${FORCE_KILL:-0}" = "1" ]; then
    set -- --force-kill "$@"
fi

if ! command -v freshen >/dev/null 2>&1; then
    echo "update-all.sh: 'freshen' is not on PATH; install it with 'cargo install --path crates/freshen' or use a release binary" >&2
    exit 127
fi

exec freshen "$@"
