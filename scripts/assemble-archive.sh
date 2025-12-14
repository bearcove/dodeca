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

# Auto-discover cdylib cells (crates with cdylib in Cargo.toml)
CDYLIB_CELLS=()
for dir in crates/dodeca-*/; do
    if [[ -f "$dir/Cargo.toml" ]] && grep -q 'cdylib' "$dir/Cargo.toml"; then
        cell=$(basename "$dir")
        # Convert crate name to lib name (dodeca-foo -> dodeca_foo)
        lib_name="${cell//-/_}"
        CDYLIB_CELLS+=("$lib_name")
    fi
done

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

echo "Discovered cdylib cells: ${CDYLIB_CELLS[*]:-none}"
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

# Copy and strip binary
cp "${RELEASE_DIR}/${BINARY_NAME}" staging/
if [[ -n "$STRIP_CMD" ]]; then
    echo "Stripping: ${BINARY_NAME}"
    $STRIP_CMD "staging/${BINARY_NAME}"
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
        if [[ -n "$STRIP_CMD" ]]; then
            echo "Stripping: ${BIN_NAME}"
            $STRIP_CMD "staging/${BIN_NAME}"
        fi
        echo "Copied rapace cell: ${BIN_NAME}"
    else
        echo "Warning: Rapace cell not found: $SRC"
    fi
done

# Copy and strip cdylib cells (if any)
if [[ ${#CDYLIB_CELLS[@]} -gt 0 ]]; then
    mkdir -p staging/cells
    for cell in "${CDYLIB_CELLS[@]}"; do
        CELL_FILE="${LIB_PREFIX}${cell}.${LIB_EXT}"
        SRC="${RELEASE_DIR}/${CELL_FILE}"
        if [[ -f "$SRC" ]]; then
            cp "$SRC" staging/cells/
            if [[ -n "$STRIP_CMD" ]]; then
                echo "Stripping: cells/${CELL_FILE}"
                $STRIP_CMD "staging/cells/${CELL_FILE}"
            fi
            echo "Copied cdylib cell: ${CELL_FILE}"
        else
            echo "Warning: cdylib cell not found: $SRC"
        fi
    done
fi

# Create archive
echo "Creating archive: $ARCHIVE_NAME"
if [[ "$ARCHIVE_EXT" == "zip" ]]; then
    cd staging && 7z a -tzip "../${ARCHIVE_NAME}" .
else
    tar -cJf "${ARCHIVE_NAME}" -C staging .
fi

# Cleanup
rm -rf staging

echo "Archive created: $ARCHIVE_NAME"
