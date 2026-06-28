#!/bin/bash
# Compatibility entrypoint for the browser JS/WASM asset build.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$ROOT/scripts/build-browser-assets.sh" "$@"
