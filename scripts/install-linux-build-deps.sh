#!/usr/bin/env bash
# Install everything a Linux release build needs, on a bare Ubuntu/Debian image.
#
# Used by .github/workflows/release.yml (inside the ubuntu:22.04 container) and
# by scripts/docker-build-linux-release.sh. Idempotent: rerunning is cheap.
set -euo pipefail

if [ "$(id -u)" -eq 0 ]; then
    sudo=""
else
    sudo="sudo"
fi

$sudo apt-get update -qq
DEBIAN_FRONTEND=noninteractive $sudo apt-get install -y -qq \
    build-essential \
    ca-certificates \
    curl \
    git \
    pkg-config \
    rpm \
    zip \
    zstd

# The container has no Rust of its own; let rustup install its current stable.
# The glibc gate run on the artifact is what guarantees the release, not the
# compiler version, so there is nothing to pin here.
if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal
fi

cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
export PATH="$cargo_bin:$PATH"
# A GitHub Actions step is its own process, so an export here would die with
# this script; publish the directory to the following steps instead.
if [ -n "${GITHUB_PATH:-}" ]; then
    echo "$cargo_bin" >> "$GITHUB_PATH"
fi

cargo --version
