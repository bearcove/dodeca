# svag

[![MIT + Apache 2.0](https://img.shields.io/badge/license-MIT%20%2B%20Apache%202.0-blue)](./LICENSE-MIT)
[![crates.io](https://img.shields.io/crates/v/svag.svg)](https://crates.io/crates/svag)
[![docs.rs](https://docs.rs/svag/badge.svg)](https://docs.rs/svag)

An SVG minifier.

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
use svag::minify;

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
echo '<svg>...</svg>' | svag

# From file
svag input.svg -o output.svg

# With stats
svag input.svg --stats
# 1961 -> 602 bytes (69.3% smaller)
```

### With custom options

```rust
use svag::{minify_with_options, Options};

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
cargo add svag
```

Or for the CLI:

```bash
cargo install svag
```

## Benchmarks

Test corpus: `tests/corpus/*.svg` (3 files, 2.7 KB total)

| File | Original | svag | svgo |
|------|----------|------|------|
| Complex Path | 545 B | 335 B (-38.5%) | 336 B (-38.3%) |
| Inkscape Bloated | 1.9 KB | 602 B (-69.3%) | 521 B (-73.4%) |
| Simple | 215 B | 206 B (-4.2%) | 190 B (-11.6%) |

| **Total** | **2.7 KB** | **1.1 KB** (-58.0%) | **1.0 KB** (-61.5%) |

### Summary

|  | svag | svgo |
|--|------|------|
| **Bytes saved** | 1.5 KB | 1.6 KB |
| **Processing time** | 0.8ms | 23.2ms |

[svgo](https://github.com/svg/svgo) is a mature, battle-tested Node.js SVG optimizer. Timing measured with svgo loaded as a library (not CLI) for fair comparison.

To regenerate these benchmarks:

```bash
npm install svgo  # for comparison
cargo xtask readme
```

## FAQ

**Why "svag"?**

Because it's swag.

**Is it production-ready?**

No, but the tests make me reasonably sure it won't mess anything up.

## Roadmap

- [ ] More optimization passes (match svgo's output sizes)
- [ ] SVGO-compatible plugin system
- [ ] Streaming support for large files

## Inspired by

- [svgo](https://github.com/svg/svgo) - The OG SVG optimizer
- [oxvg](https://github.com/nickshanks/oxvg) - Rust SVGO port

## License

MIT OR Apache-2.0