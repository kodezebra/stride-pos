#!/usr/bin/env bash
# Build the Linux release packages locally in the same image CI uses.
#
# The host distro does not matter (Fedora, Arch, anything): the container
# supplies glibc 2.35, so the .deb/.rpm/.tar.gz run on Debian 12 and Ubuntu
# 22.04+. Needs docker or podman.
#
# Usage: scripts/docker-build-linux-release.sh <version>
set -euo pipefail

version="${1:-${VERSION:-}}"
if [ -z "$version" ]; then
    echo "usage: $0 <version>   (e.g. 0.1.0)" >&2
    exit 2
fi

image="${IMAGE:-ubuntu:22.04}"

if command -v docker >/dev/null 2>&1; then
    engine=docker
elif command -v podman >/dev/null 2>&1; then
    engine=podman
else
    echo "error: docker or podman is required" >&2
    exit 1
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Cargo's registry and target dir live in named volumes so the build never
# leaves root-owned files in the checkout. Only dist/ is written back, and it
# is handed to the caller's uid before the container exits.
"$engine" run --rm \
    -e VERSION="$version" \
    -e HOST_UID="$(id -u)" \
    -e HOST_GID="$(id -g)" \
    -e CARGO_HOME=/cargo \
    -e RUSTUP_HOME=/rustup \
    -v "$root:/src" \
    -v stride-pos-cargo:/cargo \
    -v stride-pos-rustup:/rustup \
    -v stride-pos-target:/src/target \
    -w /src \
    "$image" \
    bash -lc '
        set -euo pipefail
        git config --global --add safe.directory /src
        ./scripts/install-linux-build-deps.sh
        ./scripts/build-linux-release.sh "$VERSION"
        chown -R "${HOST_UID}:${HOST_GID}" /src/dist
    '

echo "packages in $root/dist"
