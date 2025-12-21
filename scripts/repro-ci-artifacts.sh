#!/usr/bin/env bash
set -euo pipefail

run_id="${1:-}"
pattern="${2:-}"
dest="${3:-dist}"

if [[ -z "$run_id" ]]; then
  echo "usage: $0 <run_id> [artifact-pattern] [dest-dir]" >&2
  exit 1
fi

rm -rf "$dest"
mkdir -p "$dest"

if [[ -n "$pattern" ]]; then
  gh run download "$run_id" -p "$pattern" -D "$dest"
else
  gh run download "$run_id" -D "$dest"
fi

chmod +x "$dest"/ddc "$dest"/ddc-cell-* || true

export DODECA_BIN="$PWD/$dest/ddc"
export DODECA_CELL_PATH="$PWD/$dest"
export DDC_LOG_TIME="${DDC_LOG_TIME:-utc}"
export DODECA_SHOW_LOGS="${DODECA_SHOW_LOGS:-1}"
export RUST_LOG="${RUST_LOG:-debug,hyper_util=debug,reqwest=debug,ddc=debug,rapace_core=debug}"

cargo xtask integration --no-build
