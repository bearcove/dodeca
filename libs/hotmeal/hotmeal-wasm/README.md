# hotmeal-wasm

[![crates.io](https://img.shields.io/crates/v/hotmeal-wasm.svg)](https://crates.io/crates/hotmeal-wasm)
[![documentation](https://docs.rs/hotmeal-wasm/badge.svg)](https://docs.rs/hotmeal-wasm)
[![MIT licensed](https://img.shields.io/crates/l/hotmeal-wasm.svg)](./LICENSE)

# hotmeal-wasm

WebAssembly bindings for hotmeal HTML toolkit.

Provides browser-side HTML parsing, DOM diffing, and patch application for use in JavaScript/TypeScript applications.

## Features

- Parse HTML strings into DOM trees
- Compute minimal patches between two HTML documents
- Apply patches to update the DOM efficiently
- Works with any JavaScript framework or vanilla JS

## Usage

```javascript
import init, { parse_html, diff_html, apply_patches } from 'hotmeal-wasm';

await init();

const old_html = '<div><p>Hello</p></div>';
const new_html = '<div><p>World</p></div>';

const patches = diff_html(old_html, new_html);
// Apply patches to your DOM...
```

## Sponsors

Thanks to:

<p> <a href="https://zed.dev">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/bearcove/hotmeal/raw/main/static/sponsors-v3/zed-dark.svg">
<img src="https://github.com/bearcove/hotmeal/raw/main/static/sponsors-v3/zed-light.svg" height="40" alt="Zed">
</picture>
</a> <a href="https://depot.dev?utm_source=hotmeal">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/bearcove/hotmeal/raw/main/static/sponsors-v3/depot-dark.svg">
<img src="https://github.com/bearcove/hotmeal/raw/main/static/sponsors-v3/depot-light.svg" height="40" alt="Depot">
</picture>
</a> </p>

## License

Licensed under the MIT license ([LICENSE](https://github.com/bearcove/hotmeal/blob/main/LICENSE) or <http://opensource.org/licenses/MIT>).
