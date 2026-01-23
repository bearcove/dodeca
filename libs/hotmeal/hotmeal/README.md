# hotmeal

[![crates.io](https://img.shields.io/crates/v/hotmeal.svg)](https://crates.io/crates/hotmeal)
[![documentation](https://docs.rs/hotmeal/badge.svg)](https://docs.rs/hotmeal)
[![MIT licensed](https://img.shields.io/crates/l/hotmeal.svg)](./LICENSE)

# hotmeal

HTML toolkit with arena-based DOM, html5ever parsing, tree diffing, and serialization.

## Features

- **HTML5 Parsing**: Full HTML5 tree construction via html5ever with error recovery
- **Arena-based DOM**: Efficient arena-allocated tree with zero-copy parsing
- **Tree Diffing**: GumTree/Chawathe algorithm via cinereus for efficient DOM patches
- **Serialization**: HTML output with proper escaping and formatting

## Usage

```rust
use hotmeal::{parse_body, diff::{diff, apply_patches}};

// Parse HTML
let old = parse_body("<div><p>Hello</p></div>");
let new = parse_body("<div><p>World</p></div>");

// Compute patches
let patches = hotmeal::diff::diff(&old, &new).expect("diffing should succeed");

// Apply patches to transform old into new
let mut old_diff: hotmeal::diff::Element = (&old).into();
hotmeal::diff::apply_patches(&mut old_diff, &patches).expect("patches should apply");
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
