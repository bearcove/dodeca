#!/bin/bash
# Build ddc for a target
# Usage: scripts/build-target.sh <target-triple>
# Example: scripts/build-target.sh x86_64-unknown-linux-gnu
set -euo pipefail

TARGET="${1:?Usage: $0 <target-triple>}"

echo "Building for target: $TARGET"

# Set up cross-compilation environment for ARM Linux
if [[ "$TARGET" == "aarch64-unknown-linux-gnu" ]]; then
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
fi

# Build ddc
echo "Building ddc..."
cargo build --release --target "$TARGET" --bin ddc

echo "Build complete for $TARGET"
