#!/bin/bash
# Assemble release archive for a target
# Usage: scripts/assemble-archive.sh <target-triple>
# Example: scripts/assemble-archive.sh x86_64-unknown-linux-gnu
set -euo pipefail

TARGET="${1:?Usage: $0 <target-triple>}"

echo "Assembling archive for: $TARGET"

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

# Auto-discover cdylib plugins (crates with cdylib in Cargo.toml)
CDYLIB_PLUGINS=()
for dir in crates/dodeca-*/; do
    if [[ -f "$dir/Cargo.toml" ]] && grep -q 'cdylib' "$dir/Cargo.toml"; then
        plugin=$(basename "$dir")
        # Convert crate name to lib name (dodeca-foo -> dodeca_foo)
        lib_name="${plugin//-/_}"
        CDYLIB_PLUGINS+=("$lib_name")
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

# Create staging directory
rm -rf staging
mkdir -p staging/plugins

# Copy binary
cp "target/${TARGET}/release/${BINARY_NAME}" staging/

# Copy rapace plugin binaries
for plugin in "${RAPACE_PLUGINS[@]}"; do
    if [[ "$TARGET" == *windows* ]]; then
        BIN_NAME="dodeca-${plugin}.exe"
    else
        BIN_NAME="dodeca-${plugin}"
    fi
    SRC="target/${TARGET}/release/${BIN_NAME}"
    if [[ -f "$SRC" ]]; then
        cp "$SRC" staging/
        echo "Copied rapace plugin: ${BIN_NAME}"
    else
        echo "Warning: Rapace plugin not found: $SRC"
    fi
done

# Copy cdylib plugins
for plugin in "${CDYLIB_PLUGINS[@]}"; do
    PLUGIN_FILE="${LIB_PREFIX}${plugin}.${LIB_EXT}"
    SRC="target/${TARGET}/release/${PLUGIN_FILE}"
    if [[ -f "$SRC" ]]; then
        cp "$SRC" staging/plugins/
        echo "Copied cdylib plugin: ${PLUGIN_FILE}"
    else
        echo "Warning: cdylib plugin not found: $SRC"
    fi
done

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
