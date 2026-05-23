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

# Auto-discover cell cdylibs (cells with crate-type = ["cdylib"], excluding -proto)
CELL_LIBS=()
for dir in cells/cell-*/; do
    dirname=$(basename "$dir")
    # Skip proto crates
    if [[ "$dirname" == *-proto ]]; then
        continue
    fi
    if [[ -f "$dir/Cargo.toml" ]] && grep -q '"cdylib"' "$dir/Cargo.toml"; then
        lib_name=$(
            awk '
                /^\[/ { in_lib = ($0 == "[lib]"); next }
                in_lib && $1 == "name" {
                    gsub(/"/, "", $3)
                    print $3
                    exit
                }
            ' "$dir/Cargo.toml"
        )
        if [[ -n "$lib_name" ]]; then
            CELL_LIBS+=("$lib_name")
        fi
    fi
done

echo "Discovered cell cdylibs: ${CELL_LIBS[*]:-none}"

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

# Copy and strip cell cdylibs
for lib in "${CELL_LIBS[@]}"; do
    LIB_FILE="${LIB_PREFIX}${lib}.${LIB_EXT}"
    SRC="${RELEASE_DIR}/${LIB_FILE}"
    if [[ -f "$SRC" ]]; then
        cp "$SRC" staging/
        BIN_FILES+=("${LIB_FILE}")
        echo "Copied cell cdylib: ${LIB_FILE}"
    else
        echo "Warning: cell cdylib not found: $SRC"
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
    tar -c -f - -C staging . | xz -T0 -1 > "${ARCHIVE_NAME}"
fi

# Cleanup
rm -rf staging

echo "Archive created: $ARCHIVE_NAME"
