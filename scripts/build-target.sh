#!/bin/bash
# Build ddc and plugins for a target
# Usage: scripts/build-target.sh <target-triple>
# Example: scripts/build-target.sh x86_64-unknown-linux-gnu
set -euo pipefail

TARGET="${1:?Usage: $0 <target-triple>}"

echo "Building for target: $TARGET"

# Auto-discover cdylib plugins (crates with cdylib in Cargo.toml)
CDYLIB_PLUGINS=()
for dir in crates/dodeca-*/; do
    if [[ -f "$dir/Cargo.toml" ]] && grep -q 'cdylib' "$dir/Cargo.toml"; then
        plugin=$(basename "$dir")
        CDYLIB_PLUGINS+=("$plugin")
    fi
done

# Auto-discover rapace plugins (mods with [[bin]] in Cargo.toml, excluding -proto)
RAPACE_PLUGINS=()
for dir in mods/mod-*/; do
    dirname=$(basename "$dir")
    # Skip proto crates
    if [[ "$dirname" == *-proto ]]; then
        continue
    fi
    if [[ -f "$dir/Cargo.toml" ]] && grep -q '\[\[bin\]\]' "$dir/Cargo.toml"; then
        RAPACE_PLUGINS+=("$dirname")
    fi
done

echo "Discovered cdylib plugins: ${CDYLIB_PLUGINS[*]:-none}"
echo "Discovered rapace plugins: ${RAPACE_PLUGINS[*]:-none}"

# Set up cross-compilation environment for ARM Linux
if [[ "$TARGET" == "aarch64-unknown-linux-gnu" ]]; then
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
fi

# Build ddc and all rapace plugins
echo "Building ddc and rapace plugins..."
RAPACE_ARGS="-p dodeca"
for plugin in "${RAPACE_PLUGINS[@]}"; do
    bin_name="dodeca-${plugin}"
    RAPACE_ARGS="$RAPACE_ARGS -p $plugin --bin $bin_name"
done
cargo build --release --target "$TARGET" $RAPACE_ARGS

# Build cdylib plugins if any
if [[ ${#CDYLIB_PLUGINS[@]} -gt 0 ]]; then
    echo "Building cdylib plugins..."
    CDYLIB_ARGS=""
    for plugin in "${CDYLIB_PLUGINS[@]}"; do
        CDYLIB_ARGS="$CDYLIB_ARGS -p $plugin"
    done
    cargo build --release --target "$TARGET" $CDYLIB_ARGS
fi

echo "Build complete for $TARGET"
