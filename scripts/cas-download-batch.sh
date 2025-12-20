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

# Check if we got any files
POINTER_COUNT=$(find "$STAGING_DIR/pointers" -type f | wc -l)
if [[ "$POINTER_COUNT" -eq 0 ]]; then
    echo "Warning: No pointer files found under $POINTER_PREFIX"
    exit 0
fi

# Step 2: Read all hashes locally and collect unique ones
echo "Found $POINTER_COUNT pointer files, reading hashes..."
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
echo "Need $UNIQUE_HASHES unique CAS objects..."

# Step 3: Download unique CAS files in parallel
for hash in "${!HASH_TO_FILES[@]}"; do
    aws s3 cp "s3://$S3_BUCKET/cas/$hash" "$STAGING_DIR/cas/$hash" \
        --endpoint-url "$S3_ENDPOINT" &
done
wait

# Step 4: Copy/hardlink to destination with correct names, make executable
for hash in "${!HASH_TO_FILES[@]}"; do
    for filename in ${HASH_TO_FILES[$hash]}; do
        cp "$STAGING_DIR/cas/$hash" "$LOCAL_DIR/$filename"
        # Make binaries executable
        if [[ "$filename" == "ddc" || "$filename" == ddc-cell-* ]]; then
            chmod +x "$LOCAL_DIR/$filename"
        fi
        echo "$filename: downloaded (${hash:0:12}...)"
    done
done

echo "Done: $POINTER_COUNT files downloaded"
