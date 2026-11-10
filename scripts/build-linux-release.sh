#!/usr/bin/env bash
# Build and package the Linux desktop release, then prove it can load.
#
# Run this inside the release container (ubuntu:22.04), never on the host: the
# binary inherits the glibc of whatever machine links it. Ubuntu 22.04 gives a
# glibc 2.35 floor, which is what Debian 12 (2.36) and Ubuntu 22.04+ accept.
# See DEVELOPMENT.md, "Release compatibility".
#
# Usage: scripts/build-linux-release.sh <version>
set -euo pipefail

version="${1:-${VERSION:-}}"
if [ -z "$version" ]; then
    echo "usage: $0 <version>   (e.g. 0.1.0)" >&2
    exit 2
fi

target="${TARGET:-x86_64-unknown-linux-gnu}"
glibc_baseline="${GLIBC_BASELINE:-2.35}"
outdir="${OUTDIR:-dist}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# Callers may have installed rustup after this shell started (the CI container
# and the docker wrapper both do), so resolve cargo before giving up on it.
# Not `$CARGO_HOME/env`: that file hardcodes `$HOME/.cargo/bin`, and the
# container points CARGO_HOME at /cargo.
if ! command -v cargo >/dev/null 2>&1; then
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi

cargo build --release --locked -p desktop --target "$target"

bin_dir="target/${target}/release"
binary=""
for candidate in desktop pos; do
    if [ -f "$bin_dir/$candidate" ]; then
        binary="$bin_dir/$candidate"
        break
    fi
done
if [ -z "$binary" ]; then
    echo "error: no release binary in $bin_dir" >&2
    exit 1
fi

chmod +x apps/desktop/package-release.sh
./apps/desktop/package-release.sh "$binary" "$version" "$outdir" "$target"

scripts/check-glibc-baseline.sh "$glibc_baseline" "$binary"

# Loading the binary here is the cheapest possible smoke test: the container's
# glibc is the shop's floor, so a missing library or symbol shows up now.
if ldd "$binary" | grep -q 'not found'; then
    echo "error: $binary cannot resolve a shared library:" >&2
    ldd "$binary" >&2
    exit 1
fi
if ! ldd "$binary" | grep -q 'libc\.so\.6'; then
    echo "unexpected: $binary is not dynamically linked against libc" >&2
    ldd "$binary" >&2
fi

ls -l "$outdir"/stride-pos-"$version"*
