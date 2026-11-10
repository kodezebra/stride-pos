#!/usr/bin/env bash
# Installs the compiler the released binary is built with.
#
# Deliberately not `rust-toolchain.toml`. That file is also read by a
# developer's own `cargo`, and rustup resolves it by *name*: someone whose
# `stable` already is the pinned version would still be made to download a
# second copy of it. Only CI needs this exact compiler, so only CI pays.
#
# Moving the version is a release decision: a newer compiler can raise the
# glibc floor on its own (1.99 added weak pidfd references that want 2.39).
# Bump it, then re-run scripts/check-glibc-baseline.sh on the artifact.
set -euo pipefail

channel="${1:-1.99.0}"

rustup toolchain install "$channel" \
    --profile minimal \
    --component rustfmt \
    --component clippy
rustup default "$channel"

rustc --version
cargo --version
