#!/bin/sh
# Runs a command in the Linux OCCT dev container with the repository mounted at /src.
#   tools/docker/dev.sh cargo test --workspace --all-features
set -e
root="$(cd "$(dirname "$0")/../.." && pwd)"
MSYS_NO_PATHCONV=1 exec docker run --rm -v "$root:/src" -v flatbello-target:/target \
    -v flatbello-cargo:/root/.cargo/registry flatbello-dev "$@"
