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
