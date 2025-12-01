#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# Build in release mode for accurate benchmarks
cargo build --release 2>/dev/null

SAVAGE="./target/release/savage"
CORPUS_DIR="tests/corpus"

# Generate benchmark table rows
generate_benchmarks() {
    for svg in "$CORPUS_DIR"/*.svg; do
        [ -f "$svg" ] || continue

        name=$(basename "$svg" .svg)
        # Make name human-readable (capitalize words, replace hyphens)
        display_name=$(echo "$name" | sed 's/-/ /g' | awk '{for(i=1;i<=NF;i++) $i=toupper(substr($i,1,1)) tolower(substr($i,2))}1')

        original_size=$(wc -c < "$svg" | tr -d ' ')
        minified=$("$SAVAGE" < "$svg")
        minified_size=${#minified}

        if [ "$original_size" -gt 0 ]; then
            # Use awk for float math
            awk -v orig="$original_size" -v mini="$minified_size" -v name="$display_name" '
            BEGIN {
                savings = (1 - mini / orig) * 100

                # Format sizes
                if (orig >= 1024) {
                    orig_fmt = sprintf("%.1f KB", orig / 1024)
                } else {
                    orig_fmt = orig " B"
                }

                if (mini >= 1024) {
                    min_fmt = sprintf("%.1f KB", mini / 1024)
                } else {
                    min_fmt = mini " B"
                }

                # Bold if savings > 30%
                if (savings > 30) {
                    savings_fmt = sprintf("**%.1f%%**", savings)
                } else {
                    savings_fmt = sprintf("%.1f%%", savings)
                }

                printf "| %s | %s | %s | %s |\n", name, orig_fmt, min_fmt, savings_fmt
            }'
        fi
    done
}

# Generate benchmarks
benchmarks=$(generate_benchmarks)

# Replace the benchmarks section using awk
awk -v benchmarks="$benchmarks" '
    /<!-- BENCHMARKS_START -->/ {
        print
        print "| File | Original | Minified | Savings |"
        print "|------|----------|----------|---------|"
        printf "%s\n", benchmarks
        skip = 1
        next
    }
    /<!-- BENCHMARKS_END -->/ {
        skip = 0
    }
    !skip { print }
' README.tmpl.md > README.md

echo "Generated README.md with fresh benchmarks"
