# fontcull

[![Crates.io](https://img.shields.io/crates/v/fontcull.svg)](https://crates.io/crates/fontcull)
[![Documentation](https://docs.rs/fontcull/badge.svg)](https://docs.rs/fontcull)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Pure Rust font subsetting library powered by [klippa](https://github.com/googlefonts/fontations).

## Library usage

```rust
use fontcull::{subset_font_to_woff2, decompress_font};
use std::collections::HashSet;

// Subset a font to only include specific characters
let font_data = std::fs::read("font.ttf").unwrap();
let chars: HashSet<char> = "Hello World".chars().collect();
let woff2 = subset_font_to_woff2(&font_data, &chars).unwrap();
std::fs::write("font-subset.woff2", woff2).unwrap();

// Can also decompress WOFF/WOFF2 to TTF
let woff2_data = std::fs::read("font.woff2").unwrap();
let ttf_data = decompress_font(&woff2_data).unwrap();
```

### Features

- `static-analysis`: Static HTML/CSS parsing for font usage detection

## CLI

The `fontcull-cli` crate provides a command-line tool that:

1. Opens URLs in a headless browser (via chromiumoxide)
2. Extracts all glyphs/characters used on the page (including `::before`/`::after` pseudo-elements)
3. Optionally spiders the site to find more pages
4. Subsets font files to only include the characters actually used

### Install

```bash
cargo install fontcull-cli
```

Requires Chrome/Chromium installed (uses your system browser, no specific version needed).

### Usage

```bash
# Just get the unicode range of characters used
fontcull https://example.com

# Subset fonts based on page content
fontcull https://example.com --subset=fonts/*.ttf

# Spider multiple pages
fontcull https://example.com --spider-limit=10 --subset=fonts/*.ttf

# Filter by font family
fontcull https://example.com --subset=fonts/*.ttf --family="My Font"

# Add extra characters to always include
fontcull https://example.com --subset=fonts/*.ttf --whitelist="0123456789"
```

## Sponsors

Thanks to all individual sponsors:

<p>
<a href="https://github.com/sponsors/fasterthanlime">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/github-dark.svg">
<img src="./static/sponsors-v3/github-light.svg" height="40" alt="GitHub Sponsors">
</picture>
</a>
<a href="https://patreon.com/fasterthanlime">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/patreon-dark.svg">
<img src="./static/sponsors-v3/patreon-light.svg" height="40" alt="Patreon">
</picture>
</a>
</p>

...along with corporate sponsors:

<p>
<a href="https://zed.dev">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/zed-dark.svg">
<img src="./static/sponsors-v3/zed-light.svg" height="40" alt="Zed">
</picture>
</a>
<a href="https://depot.dev?utm_source=fontcull">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/depot-dark.svg">
<img src="./static/sponsors-v3/depot-light.svg" height="40" alt="Depot">
</picture>
</a>
</p>

...without whom this work could not exist.
