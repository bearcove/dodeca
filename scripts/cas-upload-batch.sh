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

# Timing helper
now_ms() { python3 -c 'import time; print(int(time.time() * 1000))'; }
START_TIME=$(now_ms)
last_step=$START_TIME

log_step() {
    local now=$(now_ms)
    local elapsed=$((now - last_step))
    echo "  ⏱ $1: ${elapsed}ms"
    last_step=$now
}

STAGING_DIR=$(mktemp -d)
trap "rm -rf '$STAGING_DIR'" EXIT

# Show file sizes
echo "Files to upload (${#FILES[@]}):"
total_size=0
for file in "${FILES[@]}"; do
    size=$(stat -f%z "$file" 2>/dev/null || stat -c%s "$file" 2>/dev/null)
    size_mb=$(echo "scale=1; $size / 1048576" | bc)
    echo "  $(basename "$file"): ${size_mb}MB"
    total_size=$((total_size + size))
done
total_mb=$(echo "scale=1; $total_size / 1048576" | bc)
echo "  Total: ${total_mb}MB"

# Compute all hashes in parallel
echo "Computing SHA256 hashes..."
declare -A FILE_HASHES
while IFS= read -r line; do
    hash="${line%% *}"
    file="${line#* }"
    # Remove leading ./ or * that sha256sum might add
    file="${file#\*}"
    file="${file#./}"
    FILE_HASHES["$file"]="$hash"
done < <(sha256sum "${FILES[@]}" 2>/dev/null || shasum -a 256 "${FILES[@]}")
log_step "hashing ${total_mb}MB"

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
log_step "staging (hardlinks)"

# Sync to S3 CAS (only uploads files that don't exist)
echo "Syncing to CAS..."
aws s3 sync "$STAGING_DIR/cas/" "s3://$S3_BUCKET/cas/" \
    --endpoint-url "$S3_ENDPOINT" \
    --size-only \
    --no-progress
log_step "s3 sync to CAS"

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
log_step "pointer files (parallel)"

END_TIME=$(now_ms)
TOTAL=$((END_TIME - START_TIME))
echo "Done: ${#FILES[@]} files (${total_mb}MB) in ${TOTAL}ms"
