#!/usr/bin/env bash
# Refuse a binary that needs glibc symbols newer than the release baseline.
#
# A dynamically linked Linux binary keeps the glibc version of the machine that
# built it. Build on a newer distro than the shops run and the loader fails with
# "version `GLIBC_2.39' not found" the moment the shop starts the till. This
# check turns that runtime surprise into a red build.
#
# Usage: check-glibc-baseline.sh <baseline> <binary> [binary...]
#   baseline  highest glibc version the release is allowed to need, e.g. 2.35
set -euo pipefail

if [ "$#" -lt 2 ]; then
    echo "usage: $0 <baseline> <binary> [binary...]" >&2
    exit 2
fi

baseline="${1#GLIBC_}"
shift

status=0
for bin in "$@"; do
    if [ ! -f "$bin" ]; then
        echo "error: no such binary: $bin" >&2
        exit 2
    fi

    # `objdump -T` lists one line per referenced symbol; the highest version it
    # names is the floor the loader will enforce.
    needed="$(objdump -T "$bin" 2>/dev/null \
        | grep -o 'GLIBC_[0-9][0-9.]*' \
        | sed 's/^GLIBC_//' \
        | sort -Vu \
        | tail -n 1 || true)"

    if [ -z "$needed" ]; then
        echo "ok   $bin: no versioned glibc symbols (static or self-contained)"
        continue
    fi

    # `sort -V` picks the higher of the two; if that is not the baseline, the
    # binary needs something newer.
    highest="$(printf '%s\n%s\n' "$needed" "$baseline" | sort -V | tail -n 1)"
    if [ "$highest" = "$baseline" ]; then
        echo "ok   $bin: needs glibc $needed (baseline $baseline)"
    else
        echo "FAIL $bin: needs glibc $needed, but the release baseline is $baseline" >&2
        echo "     Build this artifact in scripts/build-linux-release.sh (or the" >&2
        echo "     matching CI container) instead of on the host distro." >&2
        status=1
    fi
done

exit "$status"
