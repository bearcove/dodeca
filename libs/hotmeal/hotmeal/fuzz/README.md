# Hotmeal Fuzzing

This directory contains fuzz targets for hotmeal.

## Fuzz Targets

### `apply`

Tests native diff and patch application with full trace capture. Generates random HTML pairs, computes patches, applies them, and verifies the result matches the expected output.

```bash
cargo fuzz run apply -- -dict=html.dict -max_len=65536
```

### `parse_parity`

Compares html5ever parsing against the browser's DOMParser to ensure both produce equivalent DOM trees.

```bash
cargo fuzz run parse_parity
```

### `apply_parity`

Tests that patches computed natively produce the same results when applied via browser DOM APIs. This catches any differences between hotmeal's native patch application and the browser's DOM manipulation.

```bash
cargo fuzz run apply_parity
```

### `apply_structured`

Uses a structured DOM generator to create complex, valid HTML documents. Tests the full diff/patch roundtrip on these structured inputs.

```bash
cargo fuzz run apply_structured
```

## Debugging with Tracing

When you find a crash or failure, you can enable tracing to see detailed logs of what's happening internally. The hotmeal crate has extensive tracing instrumentation.

### Running with tracing enabled

```bash
FACET_LOG=debug cargo fuzz run <target> <artifact_path> --features tracing
```

For more verbose output:

```bash
FACET_LOG=trace cargo fuzz run <target> <artifact_path> --features tracing
```

### Example: debugging a crash

```bash
# Run the failing case with debug-level tracing
FACET_LOG=debug cargo fuzz run apply_structured artifacts/apply_structured/crash-xxx --features tracing

# For even more detail (shows every tree operation)
FACET_LOG=trace cargo fuzz run apply_structured artifacts/apply_structured/crash-xxx --features tracing
```

The tracing output shows:
- DOM tree operations (append, insert, detach)
- Diff algorithm decisions (matching, edit operations)
- Shadow tree mutations during patch generation
- Path computations and slot management

## Browser Fuzzing

The `parse_parity` and `apply_parity` targets test hotmeal against a real browser's DOM.

### How it works

1. The fuzzer generates random HTML pairs (old, new)
2. Both are normalized through the browser's DOMParser
3. hotmeal computes patches to transform old → new
4. Patches are applied via the browser's DOM APIs (for `apply_parity`)
5. The result is compared against the expected output

### Building the WASM bundle

The browser fuzzer requires a WASM build of the `browser-wasm` crate. To rebuild after making changes:

```bash
cd hotmeal/fuzz/browser-wasm
wasm-pack build --target web --out-dir ../browser-bundle/dist
```

### Known limitations

Browsers are lenient when parsing HTML but strict when creating elements via DOM APIs. For example:

- `<s<>` parses as an element with tag name `s<`
- But `document.createElement("s<")` throws an error

The fuzzer skips test cases that produce:
- Invalid tag names (containing `<`, `>`, `=`, whitespace, etc.)
- Invalid attribute names (same characters)

These are fundamental browser quirks, not hotmeal bugs.
