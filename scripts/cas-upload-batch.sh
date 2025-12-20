#!/usr/bin/env bash
# Content-Addressed Storage batch upload for CI artifacts.
#
# Usage: cas-upload-batch.sh <pointer-prefix> <file1> [file2] ...
#
# Example: cas-upload-batch.sh "ci/1234/cells-linux-x64" target/release/ddc-cell-*
#
# This script:
# 1. Computes SHA256 hashes in parallel using sha256sum
# 2. Creates a staging directory with hardlinks named by hash
# 3. Uses `aws s3 sync` for efficient parallel upload to CAS
# 4. Writes pointer files in parallel

set -euo pipefail

POINTER_PREFIX="$1"
shift
FILES=("$@")

if [[ ${#FILES[@]} -eq 0 ]]; then
    echo "Usage: cas-upload-batch.sh <pointer-prefix> <file1> [file2] ..."
    exit 1
fi

S3_ENDPOINT="${S3_ENDPOINT:?S3_ENDPOINT must be set}"
S3_BUCKET="${S3_BUCKET:?S3_BUCKET must be set}"

STAGING_DIR=$(mktemp -d)
trap "rm -rf '$STAGING_DIR'" EXIT

# Compute all hashes in parallel
echo "Computing hashes for ${#FILES[@]} files..."
declare -A FILE_HASHES
while IFS= read -r line; do
    hash="${line%% *}"
    file="${line#* }"
    # Remove leading ./ or * that sha256sum might add
    file="${file#\*}"
    file="${file#./}"
    FILE_HASHES["$file"]="$hash"
done < <(sha256sum "${FILES[@]}" 2>/dev/null || shasum -a 256 "${FILES[@]}")

# Create staging directory with hardlinks named by hash
mkdir -p "$STAGING_DIR/cas"
for file in "${FILES[@]}"; do
    # Normalize path for lookup
    normalized="${file#./}"
    hash="${FILE_HASHES[$normalized]}"
    if [[ -z "$hash" ]]; then
        echo "Error: No hash found for $file"
        exit 1
    fi
    # Use hardlink if possible (same filesystem), else copy
    ln "$file" "$STAGING_DIR/cas/$hash" 2>/dev/null || cp "$file" "$STAGING_DIR/cas/$hash"
done

# Sync to S3 CAS (only uploads files that don't exist)
echo "Syncing ${#FILES[@]} files to CAS..."
aws s3 sync "$STAGING_DIR/cas/" "s3://$S3_BUCKET/cas/" \
    --endpoint-url "$S3_ENDPOINT" \
    --size-only \
    --no-progress

# Write pointer files in parallel
echo "Writing pointer files..."
for file in "${FILES[@]}"; do
    normalized="${file#./}"
    hash="${FILE_HASHES[$normalized]}"
    basename="${file##*/}"
    pointer_key="$POINTER_PREFIX/$basename"
    echo "$hash" | aws s3 cp - "s3://$S3_BUCKET/$pointer_key" --endpoint-url "$S3_ENDPOINT" &
done
wait

echo "Done: ${#FILES[@]} files processed"
