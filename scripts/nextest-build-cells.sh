#!/usr/bin/env bash
set -euo pipefail

cargo build --workspace --exclude dodeca-devtools

if [[ -z "${NEXTEST_ENV:-}" ]]; then
    echo "NEXTEST_ENV is not set" >&2
    exit 1
fi

printf 'DODECA_CELL_PATH=%s/target/debug\n' "$PWD" >> "$NEXTEST_ENV"
