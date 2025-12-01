# savage

[![MIT + Apache 2.0](https://img.shields.io/badge/license-MIT%20%2B%20Apache%202.0-blue)](./LICENSE-MIT)
[![crates.io](https://img.shields.io/crates/v/savage.svg)](https://crates.io/crates/savage)
[![docs.rs](https://docs.rs/savage/badge.svg)](https://docs.rs/savage)

A savage SVG minifier.

## Philosophy

**Aggressive but safe.** Savage applies every optimization that won't break your SVG.
It removes comments, metadata, editor cruft (Inkscape, Illustrator), unnecessary
whitespace, default attribute values—anything that doesn't affect rendering.

**Visual fidelity first.** Every optimization is tested against headless Chrome
rendering with SSIM comparison. If the output doesn't match the input at 99.9%
similarity, the test fails.

**Rust-native.** No Node.js, no WASM, no subprocess. Just `cargo add savage` and go.

## Features

- Remove XML declarations, DOCTYPE, comments
- Remove metadata, title, desc elements
- Remove Inkscape/Sodipodi namespaces and elements
- Remove unused namespace declarations
- Collapse unnecessary groups
- Remove hidden and empty elements
- Minify path data (reduce precision, implicit commands)
- Minify colors (`#ff0000` → `red`, `#ffffff` → `#fff`)
- Remove default attribute values
- Minify inline styles
- Sort attributes for better gzip

## Usage

### As a library

```rust
use savage::minify;

let svg = r#"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
    <!-- A red square -->
    <rect x="10" y="10" width="80" height="80" fill="#ff0000" fill-opacity="1"/>
</svg>"#;

let minified = minify(svg).unwrap();
// <svg xmlns="http://www.w3.org/2000/svg" height="100" width="100"><rect fill="red" height="80" width="80" x="10" y="10"/></svg>
```

### As a CLI

```bash
# From stdin
echo '<svg>...</svg>' | savage

# From file
savage input.svg -o output.svg

# With stats
savage input.svg --stats
# 1961 -> 602 bytes (69.3% smaller)
```

### With custom options

```rust
use savage::{minify_with_options, Options};

let options = Options {
    precision: 1,           // decimal places for coordinates
    remove_comments: true,
    minify_paths: true,
    minify_colors: true,
    ..Options::default()
};

let minified = minify_with_options(svg, &options).unwrap();
```

## Installation

```bash
cargo add savage
```

Or for the CLI:

```bash
cargo install savage
```

## Benchmarks

Test corpus: `tests/corpus/*.svg` ({{ file_count }} files, {{ total.original }} total)

### Size comparison

| File | Original | savage | svgo |
|------|----------|--------|------|
{% for b in benchmarks -%}
| {{ b.name }} | {{ b.original }} | {{ b.savage }} ({{ b.savage_pct }}) | {{ b.svgo }} ({{ b.svgo_pct }}) |
{% endfor %}
| **Total** | **{{ total.original }}** | **{{ total.savage }}** ({{ total.savage_pct }}) | **{{ total.svgo }}** ({{ total.svgo_pct }}) |

### Speed comparison

| Tool | Time | Throughput |
|------|------|------------|
| savage | {{ timing.savage_time }} | {{ timing.savage_throughput }} |
| svgo | {{ timing.svgo_time }} | {{ timing.svgo_throughput }} |

savage is **{{ timing.speedup }}** faster than svgo.

To regenerate these benchmarks:

```bash
npm install svgo  # for comparison benchmarks
cargo run --bin generate-readme --features minijinja
```

## Inspired by

- [svgo](https://github.com/svg/svgo) - The OG SVG optimizer
- [oxvg](https://github.com/nickshanks/oxvg) - Rust SVGO port

## Sponsors

CI hosted by:

<p><a href="https://depot.dev?utm_source=savage">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/bearcove/savage/raw/main/static/depot-dark.svg">
<img src="https://github.com/bearcove/savage/raw/main/static/depot-light.svg" height="40" alt="Depot">
</picture>
</a></p>

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
