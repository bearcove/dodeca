#!/bin/bash
# Build ddc and cell cdylibs for a target
# Usage: scripts/build-target.sh <target-triple>
# Example: scripts/build-target.sh x86_64-unknown-linux-gnu
set -euo pipefail

TARGET="${1:?Usage: $0 <target-triple>}"

echo "Building for target: $TARGET"

# Auto-discover cell cdylibs (cells with crate-type = ["cdylib"], excluding -proto)
CELL_PACKAGES=()
for dir in cells/cell-*/; do
    dirname=$(basename "$dir")
    # Skip proto crates
    if [[ "$dirname" == *-proto ]]; then
        continue
    fi
    if [[ -f "$dir/Cargo.toml" ]] && grep -q '"cdylib"' "$dir/Cargo.toml"; then
        CELL_PACKAGES+=("$dirname")
    fi
done

echo "Discovered cell cdylib packages: ${CELL_PACKAGES[*]:-none}"

# Set up cross-compilation environment for ARM Linux
if [[ "$TARGET" == "aarch64-unknown-linux-gnu" ]]; then
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
fi

# Build ddc
echo "Building ddc..."
cargo build --release --target "$TARGET" --bin ddc

# Build cell cdylibs if any
if [[ ${#CELL_PACKAGES[@]} -gt 0 ]]; then
    echo "Building cell cdylibs..."
    CELL_ARGS=""
    for cell in "${CELL_PACKAGES[@]}"; do
        CELL_ARGS="$CELL_ARGS --package $cell"
    done
    cargo build --release --target "$TARGET" $CELL_ARGS
fi

echo "Build complete for $TARGET"
