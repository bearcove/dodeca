#!/bin/bash
# Build browser JS/WASM assets that are shipped next to ddc.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export CI="${CI:-true}"

rustup target add wasm32-unknown-unknown

wasm-pack build crates/dodeca-devtools --target web --target-dir target/wasm-pack
wasm-pack build crates/dodeca-search-wasm --target web --target-dir target/wasm-pack

(cd libs/hotmeal/hotmeal-wasm && wasm-pack build --target web --dev --target-dir target-wasm)

(cd crates/dodeca/devtools-ui && pnpm install --no-frozen-lockfile && pnpm run build)
