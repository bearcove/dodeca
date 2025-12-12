#!/bin/bash
# Build ddc and plugins for a target
# Usage: scripts/build-target.sh <target-triple>
# Example: scripts/build-target.sh x86_64-unknown-linux-gnu
set -euo pipefail

TARGET="${1:?Usage: $0 <target-triple>}"

echo "Building for target: $TARGET"

# Auto-discover plugins (crates with cdylib in Cargo.toml)
PLUGINS=()
for dir in crates/dodeca-*/; do
    if [[ -f "$dir/Cargo.toml" ]] && grep -q 'cdylib' "$dir/Cargo.toml"; then
        plugin=$(basename "$dir")
        PLUGINS+=("$plugin")
    fi
done

echo "Discovered plugins: ${PLUGINS[*]}"

# Set up cross-compilation environment for ARM Linux
if [[ "$TARGET" == "aarch64-unknown-linux-gnu" ]]; then
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
fi

# Build ddc and dodeca-mod-http (rapace plugin binary)
echo "Building ddc and dodeca-mod-http..."
cargo build --release --target "$TARGET" -p dodeca -p mod-http --bin dodeca-mod-http

# Build plugins
echo "Building plugins..."
PLUGIN_ARGS=""
for plugin in "${PLUGINS[@]}"; do
    PLUGIN_ARGS="$PLUGIN_ARGS -p $plugin"
done
cargo build --release --target "$TARGET" $PLUGIN_ARGS

echo "Build complete for $TARGET"
