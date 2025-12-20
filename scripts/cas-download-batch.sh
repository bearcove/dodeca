#!/usr/bin/env bash
# Content-Addressed Storage batch download for CI artifacts.
#
# Usage: cas-download-batch.sh <pointer-prefix> <local-dir>
#
# Example: cas-download-batch.sh "ci/1234/cells-linux-x64" dist/
#
# This script:
# 1. Syncs all pointer files from prefix to temp dir (single s3 sync)
# 2. Reads hashes locally (fast, no network)
# 3. Downloads unique CAS files in parallel

set -euo pipefail

POINTER_PREFIX="$1"
LOCAL_DIR="$2"

if [[ -z "$POINTER_PREFIX" || -z "$LOCAL_DIR" ]]; then
    echo "Usage: cas-download-batch.sh <pointer-prefix> <local-dir>"
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

mkdir -p "$LOCAL_DIR"
mkdir -p "$STAGING_DIR/pointers"
mkdir -p "$STAGING_DIR/cas"

# Step 1: Sync all pointer files at once
echo "Fetching pointer files from $POINTER_PREFIX..."
aws s3 sync "s3://$S3_BUCKET/$POINTER_PREFIX/" "$STAGING_DIR/pointers/" \
    --endpoint-url "$S3_ENDPOINT" \
    --no-progress
log_step "sync pointer files"

# Check if we got any files
POINTER_COUNT=$(find "$STAGING_DIR/pointers" -type f | wc -l | tr -d ' ')
if [[ "$POINTER_COUNT" -eq 0 ]]; then
    echo "Warning: No pointer files found under $POINTER_PREFIX"
    exit 0
fi

# Step 2: Read all hashes locally and collect unique ones
declare -A HASH_TO_FILES  # hash -> space-separated list of filenames

for pointer_file in "$STAGING_DIR/pointers"/*; do
    [[ -f "$pointer_file" ]] || continue
    filename=$(basename "$pointer_file")
    hash=$(cat "$pointer_file")

    if [[ -n "${HASH_TO_FILES[$hash]:-}" ]]; then
        HASH_TO_FILES[$hash]+=" $filename"
    else
        HASH_TO_FILES[$hash]="$filename"
    fi
done

UNIQUE_HASHES=${#HASH_TO_FILES[@]}
echo "Found $POINTER_COUNT pointers → $UNIQUE_HASHES unique CAS objects"
log_step "parse pointers"

# Step 3: Download unique CAS files in parallel
echo "Downloading from CAS..."
for hash in "${!HASH_TO_FILES[@]}"; do
    aws s3 cp "s3://$S3_BUCKET/cas/$hash" "$STAGING_DIR/cas/$hash" \
        --endpoint-url "$S3_ENDPOINT" &
done
wait
log_step "download CAS objects (parallel)"

# Step 4: Copy to destination with correct names, make executable
total_size=0
for hash in "${!HASH_TO_FILES[@]}"; do
    for filename in ${HASH_TO_FILES[$hash]}; do
        cp "$STAGING_DIR/cas/$hash" "$LOCAL_DIR/$filename"
        size=$(stat -f%z "$LOCAL_DIR/$filename" 2>/dev/null || stat -c%s "$LOCAL_DIR/$filename" 2>/dev/null)
        total_size=$((total_size + size))
        # Make binaries executable
        if [[ "$filename" == "ddc" || "$filename" == ddc-cell-* ]]; then
            chmod +x "$LOCAL_DIR/$filename"
        fi
    done
done
log_step "copy to destination"

total_mb=$(echo "scale=1; $total_size / 1048576" | bc)
END_TIME=$(now_ms)
TOTAL=$((END_TIME - START_TIME))
echo "Done: $POINTER_COUNT files (${total_mb}MB) in ${TOTAL}ms"
