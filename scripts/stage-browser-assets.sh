#!/bin/bash
# Copy built browser JS/WASM assets into a runtime asset directory.
set -euo pipefail

DEST="${1:?Usage: $0 <dest-dir>}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DEVTOOLS_RUNTIME_DIR="crates/dodeca-devtools/pkg"
DEVTOOLS_UI_DIR="crates/dodeca/devtools-ui/dist"
SEARCH_UI_DIR="crates/dodeca-search-wasm/ui"
SEARCH_RUNTIME_DIR="crates/dodeca-search-wasm/pkg"

for dir in "$DEVTOOLS_RUNTIME_DIR" "$DEVTOOLS_UI_DIR" "$SEARCH_UI_DIR" "$SEARCH_RUNTIME_DIR"; do
    if [[ ! -d "$dir" ]]; then
        echo "Missing browser asset directory: $dir" >&2
        exit 1
    fi
done

rm -rf "$DEST"
mkdir -p "$DEST/devtools-runtime" "$DEST/devtools-ui" "$DEST/search"

cp -R "$DEVTOOLS_RUNTIME_DIR"/. "$DEST/devtools-runtime/"
cp -R "$DEVTOOLS_UI_DIR"/. "$DEST/devtools-ui/"
cp "$SEARCH_UI_DIR/search.js" "$SEARCH_UI_DIR/search.css" "$DEST/search/"
cp "$SEARCH_RUNTIME_DIR/dodeca_search_wasm.js" \
   "$SEARCH_RUNTIME_DIR/dodeca_search_wasm_bg.wasm" \
   "$DEST/search/"

echo "Staged browser assets in $DEST"
