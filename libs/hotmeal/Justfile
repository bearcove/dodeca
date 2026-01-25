# Hotmeal development tasks

# List all available targets
default:
    @just --list

# Build the browser-wasm bundle (required for browser fuzzing/testing)
build-wasm:
    cd hotmeal/fuzz/browser-wasm && wasm-pack build --target web --out-dir ../browser-bundle/dist

# Run browser-based tests via thrall (launches Chrome)
test-browser: build-wasm
    cd hotmeal/fuzz && cargo nextest run -E 'test(browser)' --no-capture -j1

# Run a fuzz target (e.g., just fuzz apply_structured)
fuzz TARGET *ARGS: build-wasm
    cd hotmeal/fuzz && cargo +nightly fuzz run {{ TARGET }} -- -dict=html.dict {{ ARGS }}

# List all available fuzz targets
fuzz-list:
    cd hotmeal/fuzz && cargo +nightly fuzz list

# Minimize a crash artifact (e.g., just fuzz-tmin apply_structured artifacts/...)
fuzz-tmin TARGET ARTIFACT: build-wasm
    cd hotmeal/fuzz && cargo +nightly fuzz tmin {{ TARGET }} {{ ARTIFACT }}

# Run fuzz target with coverage report
fuzz-cov TARGET *ARGS: build-wasm
    cd hotmeal/fuzz && cargo +nightly fuzz coverage {{ TARGET }} -- -dict=html.dict {{ ARGS }}

# Run all tests in the main workspace
test:
    cargo nextest run

# Run clippy on all targets
clippy:
    cargo clippy --workspace --all-features --all-targets -- -D warnings
