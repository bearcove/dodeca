# fontcull

A Rust port of the parts of [glyphhanger](https://github.com/zachleat/glyphhanger) I use regularly.

## Why?

glyphhanger is amazing, but it's 5+ years old and requires a specific old version of Chrome/Puppeteer that's become painful to maintain. With the help of Claude, I ported the core functionality to Rust.

## What it does

1. Opens URLs in a headless browser (via chromiumoxide)
2. Extracts all glyphs/characters used on the page (including `::before`/`::after` pseudo-elements)
3. Optionally spiders the site to find more pages
4. Subsets font files to only include the characters actually used

## Usage

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

## Subsetting backends

- **klippa** (default): Pure Rust subsetting via [fontations](https://github.com/googlefonts/fontations), woff2 compression via vendored Google woff2
- **pyftsubset** (`--pyftsubset`): Falls back to Python's fonttools (requires `pip install fonttools brotli`)

## Install

```bash
cargo install --git https://github.com/bearcove/fontcull
```

Requires Chrome/Chromium installed (uses your system browser, no specific version needed).

## Library usage

fontcull can also be used as a library for font subsetting:

```rust
use fontcull::{subset_font_to_woff2, analyze_fonts, extract_css_from_html};
use std::collections::HashSet;

// Direct subsetting with known characters
let font_data = std::fs::read("font.ttf").unwrap();
let chars: HashSet<char> = "Hello World".chars().collect();
let woff2 = subset_font_to_woff2(&font_data, &chars).unwrap();

// Or analyze HTML/CSS to find which characters are used (requires `static-analysis` feature)
let html = r#"<html><style>body { font-family: "MyFont"; }</style><body>Hello</body></html>"#;
let css = extract_css_from_html(html);
let analysis = analyze_fonts(html, &css);
```

### Features

- `klippa` (default): Pure Rust font subsetting
- `static-analysis`: Static HTML/CSS parsing for font usage detection
- `browser` (default): Browser-based glyph extraction for CLI

## Sponsors

<p> <a href="https://depot.dev?utm_source=fontcull">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/depot-dark.svg">
<img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/depot-light.svg" height="40" alt="Depot">
</picture>
</a> </p>

...without whom this work could not exist.
