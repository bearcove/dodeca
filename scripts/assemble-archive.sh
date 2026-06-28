#!/bin/bash
# Assemble release archive for a target
# Usage: scripts/assemble-archive.sh <target-triple>
# Example: scripts/assemble-archive.sh x86_64-unknown-linux-gnu
set -euo pipefail

TARGET="${1:?Usage: $0 <target-triple>}"

echo "Assembling archive for: $TARGET"

# Determine release directory (cross-compiled vs native)
if [[ -d "target/${TARGET}/release" ]]; then
    RELEASE_DIR="target/${TARGET}/release"
else
    RELEASE_DIR="target/release"
fi
echo "Using release directory: $RELEASE_DIR"

# Determine binary name and archive format
case "$TARGET" in
    *windows*)
        BINARY_NAME="ddc.exe"
        ARCHIVE_EXT="zip"
        ;;
    *apple*)
        BINARY_NAME="ddc"
        ARCHIVE_EXT="tar.xz"
        ;;
    *)
        BINARY_NAME="ddc"
        ARCHIVE_EXT="tar.xz"
        ;;
esac

ARCHIVE_NAME="dodeca-${TARGET}.${ARCHIVE_EXT}"

# Create staging directory
rm -rf staging
mkdir -p staging

# Determine strip command
STRIP_CMD=()
case "$TARGET" in
    *windows*)
        # Windows: skip stripping (could use llvm-strip if needed)
        ;;
    *apple*)
        # Mach-O dylibs keep imported symbols in the indirect symbol table.
        STRIP_CMD=(strip -Sx)
        ;;
    aarch64-unknown-linux-gnu)
        # Cross-compiled ARM: use aarch64 strip
        STRIP_CMD=(aarch64-linux-gnu-strip)
        ;;
    *)
        STRIP_CMD=(strip)
        ;;
esac

# Copy binaries
BIN_FILES=()
cp "${RELEASE_DIR}/${BINARY_NAME}" staging/
chmod +x "staging/${BINARY_NAME}"
BIN_FILES+=("${BINARY_NAME}")

# Copy browser JS/WASM assets. ddc reads these from `dodeca-assets/` at runtime
# instead of embedding them into the Rust binary.
bash scripts/stage-browser-assets.sh staging/dodeca-assets

# Strip binaries in parallel (if applicable)
if [[ ${#STRIP_CMD[@]} -gt 0 ]]; then
    echo "Stripping binaries (${#BIN_FILES[@]} files) with: ${STRIP_CMD[*]}"
    pids=()
    for bin in "${BIN_FILES[@]}"; do
        "${STRIP_CMD[@]}" "staging/${bin}" &
        pids+=("$!")
    done
    for pid in "${pids[@]}"; do
        wait "$pid"
    done
fi

# Create archive
echo "Creating archive: $ARCHIVE_NAME"
if [[ "$ARCHIVE_EXT" == "zip" ]]; then
    cd staging && 7z a -tzip "../${ARCHIVE_NAME}" .
else
    tar -c -f - -C staging . | xz -T0 -1 > "${ARCHIVE_NAME}"
fi

# Cleanup
rm -rf staging

echo "Archive created: $ARCHIVE_NAME"
