# Hotmeal Fuzzing

This directory contains fuzz targets for hotmeal.

## Browser Fuzzing

The `browser2` target tests hotmeal's diff/patch system against a real browser's DOM.

### How it works

1. The fuzzer generates random HTML pairs (old, new)
2. Both are normalized through the browser's DOMParser
3. hotmeal computes patches to transform old → new
4. Patches are applied via the browser's DOM APIs
5. The result is compared against the expected output

### Building the WASM bundle

The browser fuzzer requires a WASM build of the `browser-wasm` crate. To rebuild after making changes:

```bash
cd hotmeal/fuzz/browser-wasm
wasm-pack build --target web --out-dir ../browser-bundle/dist
```

### Running the browser fuzzer

```bash
cd hotmeal/fuzz
cargo fuzz run browser2
```

### Known limitations

Browsers are lenient when parsing HTML but strict when creating elements via DOM APIs. For example:

- `<s<>` parses as an element with tag name `s<`
- But `document.createElement("s<")` throws an error

The fuzzer skips test cases that produce:
- Invalid tag names (containing `<`, `>`, `=`, whitespace, etc.)
- Invalid attribute names (same characters)

These are fundamental browser quirks, not hotmeal bugs.
