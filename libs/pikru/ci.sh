#!/bin/bash
set -euo pipefail

# CI test script - runs the same tests as CI to catch issues before pushing
# Usage: ./ci.sh [--full]
#
# By default, runs only what CI runs: build C pikchr + cargo test --all --locked
# With --full, also runs fmt check and clippy

FULL=false
if [[ "${1:-}" == "--full" ]]; then
    FULL=true
fi

echo "==> Running CI checks locally"
echo ""

if $FULL; then
    echo "==> Checking formatting..."
    cargo fmt --all -- --check

    echo ""
    echo "==> Running clippy..."
    cargo clippy --all --all-targets -- -D warnings

    echo ""
fi

echo "==> Building C pikchr (required for comparison tests)..."
make -C vendor/pikchr-c pikchr

echo ""
echo "==> Running tests (cargo test --all --locked)..."
cargo test --all --locked

echo ""
echo "==> All CI checks passed!"
