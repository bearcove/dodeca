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
        LIB_PREFIX=""
        LIB_EXT="dll"
        ARCHIVE_EXT="zip"
        ;;
    *apple*)
        BINARY_NAME="ddc"
        LIB_PREFIX="lib"
        LIB_EXT="dylib"
        ARCHIVE_EXT="tar.xz"
        ;;
    *)
        BINARY_NAME="ddc"
        LIB_PREFIX="lib"
        LIB_EXT="so"
        ARCHIVE_EXT="tar.xz"
        ;;
esac

ARCHIVE_NAME="dodeca-${TARGET}.${ARCHIVE_EXT}"

# Auto-discover rapace cells (cells with [[bin]] in Cargo.toml, excluding -proto)
RAPACE_CELLS=()
for dir in cells/cell-*/; do
    dirname=$(basename "$dir")
    # Skip proto crates
    if [[ "$dirname" == *-proto ]]; then
        continue
    fi
    if [[ -f "$dir/Cargo.toml" ]] && grep -q '\[\[bin\]\]' "$dir/Cargo.toml"; then
        RAPACE_CELLS+=("$dirname")
    fi
done

echo "Discovered rapace cells: ${RAPACE_CELLS[*]:-none}"

# Create staging directory
rm -rf staging
mkdir -p staging

# Determine strip command
case "$TARGET" in
    *windows*)
        # Windows: skip stripping (could use llvm-strip if needed)
        STRIP_CMD=""
        ;;
    aarch64-unknown-linux-gnu)
        # Cross-compiled ARM: use aarch64 strip
        STRIP_CMD="aarch64-linux-gnu-strip"
        ;;
    *)
        STRIP_CMD="strip"
        ;;
esac

# Copy binaries
BIN_FILES=()
cp "${RELEASE_DIR}/${BINARY_NAME}" staging/
BIN_FILES+=("${BINARY_NAME}")

# Copy acceptor binary
if [[ "$TARGET" == *windows* ]]; then
    ACCEPTOR_NAME="ddc-acceptor.exe"
else
    ACCEPTOR_NAME="ddc-acceptor"
fi
ACCEPTOR_SRC="${RELEASE_DIR}/${ACCEPTOR_NAME}"
if [[ -f "$ACCEPTOR_SRC" ]]; then
    cp "$ACCEPTOR_SRC" staging/
    BIN_FILES+=("${ACCEPTOR_NAME}")
    echo "Copied acceptor: ${ACCEPTOR_NAME}"
else
    echo "Warning: acceptor not found: $ACCEPTOR_SRC"
fi

# Copy and strip rapace cell binaries
for cell in "${RAPACE_CELLS[@]}"; do
    if [[ "$TARGET" == *windows* ]]; then
        BIN_NAME="ddc-${cell}.exe"
    else
        BIN_NAME="ddc-${cell}"
    fi
    SRC="${RELEASE_DIR}/${BIN_NAME}"
    if [[ -f "$SRC" ]]; then
        cp "$SRC" staging/
        BIN_FILES+=("${BIN_NAME}")
        echo "Copied rapace cell: ${BIN_NAME}"
    else
        echo "Warning: Rapace cell not found: $SRC"
    fi
done

# Copy devtools WASM/JS bundle if present
DEVTOOLS_DIR="crates/dodeca-devtools/pkg"
if [[ -d "$DEVTOOLS_DIR" ]]; then
    mkdir -p staging/devtools
    cp -R "$DEVTOOLS_DIR"/. staging/devtools/
    echo "Copied devtools bundle: ${DEVTOOLS_DIR}"
else
    echo "Warning: devtools bundle not found at ${DEVTOOLS_DIR}"
fi

# Strip binaries in parallel (if applicable)
if [[ -n "$STRIP_CMD" ]]; then
    echo "Stripping binaries (${#BIN_FILES[@]} files) with: ${STRIP_CMD}"
    pids=()
    for bin in "${BIN_FILES[@]}"; do
        $STRIP_CMD "staging/${bin}" &
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
    tar -c -I 'xz -T0 -1' -f "${ARCHIVE_NAME}" -C staging .
fi

# Cleanup
rm -rf staging

echo "Archive created: $ARCHIVE_NAME"
