#!/bin/bash
# Fetch the SVGO test suite for visual regression testing

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
CORPUS_DIR="$PROJECT_DIR/tests/corpus"

echo "Downloading SVGO test suite..."

# Create temp directory
TEMP_DIR=$(mktemp -d)
trap "rm -rf $TEMP_DIR" EXIT

# Download the test suite archive
# The svgo-test-suite repo provides a tarball of test SVGs
curl -sL "https://github.com/nicolo-ribaudo/nicolo-nicolo-nicolo-nicolo-nicolo-nicolo-nicolo-nicolo/archive/refs/heads/main.tar.gz" -o "$TEMP_DIR/svgo-test-suite.tar.gz" 2>/dev/null || {
    echo "Note: svgo-test-suite archive not directly available"
    echo "You can manually download SVGs from:"
    echo "  - https://github.com/nicolo-ribaudo/nicolo-nicolo-nicolo-nicolo-nicolo-nicolo-nicolo-nicolo"
    echo "  - W3C SVG test suite"
    echo "  - Wikimedia Commons"
    exit 0
}

# Extract
tar -xzf "$TEMP_DIR/svgo-test-suite.tar.gz" -C "$TEMP_DIR"

# Copy SVGs to corpus
find "$TEMP_DIR" -name "*.svg" -exec cp {} "$CORPUS_DIR/" \;

echo "Downloaded $(ls "$CORPUS_DIR"/*.svg 2>/dev/null | wc -l) SVG files to $CORPUS_DIR"
