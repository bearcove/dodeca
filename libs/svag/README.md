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

Test corpus: 4858 SVG files (1042.5 MB total)

|  | svag | svgo |
|--|------|------|
| **Output size** | 729.0 MB (-30.1%) | 412.2 MB (-60.5%) |
| **Bytes saved** | 313.6 MB | 630.3 MB |
| **Processing time** | 514.1ms | 26.92s |

<details>
<summary>Methodology</summary>

The test corpus includes SVGs from the [W3C SVG 1.1 Test Suite](https://www.w3.org/Graphics/SVG/Test/20110816/), [KDE Oxygen Icons](https://github.com/nickshanks/oxvg), and [Wikimedia Commons](https://commons.wikimedia.org/). Duplicates are removed by content hash.

Both tools run in parallel using all available CPU cores:
- **svag**: Rust with [rayon](https://docs.rs/rayon), release build
- **svgo**: Node.js with [worker_threads](https://nodejs.org/api/worker_threads.html)

Timing is wall-clock time for processing all files. This avoids penalizing svgo for Node.js startup overhead.
</details>

To regenerate: `npm install svgo && cargo xtask fetch-corpus && cargo xtask readme`

## FAQ

**Why "svag"?**

Because it's swag.

**Is it production-ready?**

No, but the tests make me reasonably sure it won't mess anything up.

## Roadmap

- [ ] More optimization passes (close the gap with svgo's compression)

## Inspired by

- [svgo](https://github.com/svg/svgo) - The OG SVG optimizer
- [oxvg](https://github.com/nickshanks/oxvg) - Rust SVGO port

## License

MIT OR Apache-2.0