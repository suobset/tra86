#!/usr/bin/env bash
#
# Runs Bind's bind-lldb tests inside a Linux container, exercising the *live*
# LLDB backend (launch/breakpoint/registers/backtrace/step) that macOS gates
# behind developer-tools authorization.
#
# Requires Docker. Grants SYS_PTRACE (LLDB needs ptrace) and mounts the repo
# read-only; build artifacts and the cargo registry live in the container / a
# named volume so the host tree and its target/ are untouched.
#
# Usage:
#   scripts/test-linux.sh                 # run the default (cargo test -p bind-lldb)
#   scripts/test-linux.sh <cargo args>    # e.g. scripts/test-linux.sh test --workspace
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE=bind-linux-test

echo "==> building $IMAGE"
docker build -f "$REPO_ROOT/docker/Dockerfile.linux-test" -t "$IMAGE" "$REPO_ROOT"

echo "==> running tests in container"
# A named volume caches the cargo registry across runs so repeats are fast.
# With no arguments the image's default CMD (cargo test -p bind-lldb) runs;
# any arguments override it (e.g. `scripts/test-linux.sh cargo test --workspace`).
docker_args=(
    --rm
    --memory=4g
    --cap-add=SYS_PTRACE
    --security-opt seccomp=unconfined
    -v "$REPO_ROOT":/src:ro
    -v bind-cargo-registry:/usr/local/cargo/registry
    "$IMAGE"
)
if [ "$#" -gt 0 ]; then
    exec docker run "${docker_args[@]}" "$@"
else
    exec docker run "${docker_args[@]}"
fi
