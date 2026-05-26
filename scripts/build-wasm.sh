#!/bin/bash
# Build WASM crates for dodeca
# Usage: scripts/build-wasm.sh
set -euo pipefail

# Get wasm-bindgen version from Cargo.lock
WASM_BINDGEN_VERSION=$(cargo metadata --format-version 1 | jq -r '.packages[] | select(.name == "wasm-bindgen") | .version' | head -1)

# Install matching wasm-bindgen-cli
cargo install wasm-bindgen-cli --version "$WASM_BINDGEN_VERSION"

# Add wasm32 target
rustup target add wasm32-unknown-unknown

# Build WASM crates
cargo build --package livereload-client --package dodeca-devtools --target wasm32-unknown-unknown --release

# Run wasm-bindgen
# On Windows, wasm-bindgen might not be in PATH, so use full path if available
WASM_BINDGEN="wasm-bindgen"
if [[ -f "$HOME/.cargo/bin/wasm-bindgen" ]]; then
    WASM_BINDGEN="$HOME/.cargo/bin/wasm-bindgen"
fi

$WASM_BINDGEN --target web --out-dir crates/livereload-client/pkg target/wasm32-unknown-unknown/release/livereload_client.wasm
$WASM_BINDGEN --target web --out-dir crates/dodeca-devtools/pkg target/wasm32-unknown-unknown/release/dodeca_devtools.wasm

echo "WASM build complete"
