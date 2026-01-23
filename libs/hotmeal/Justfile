list:
    just --list

test:
    cargo nextest run --no-fail-fast
    cd hotmeal-wasm && pnpm test

fuzz:
    cd hotmeal/fuzz && cargo +nightly fuzz run roundtrip
