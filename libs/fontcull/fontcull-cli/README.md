# fontcull-cli

CLI tool to subset fonts based on actual glyph usage from web pages.

Uses a headless browser to render pages and extract exactly which glyphs are used by each font family, then subsets fonts to include only those glyphs.

## Installation

```bash
cargo install fontcull-cli
```

## Usage

### Extract unicode ranges

Scan a URL and print the unicode range of characters used:

```bash
fontcull https://example.com
# Output: U+20-7E,U+A0,U+2019
```

### Subset fonts

Subset font files based on glyph usage from a page:

```bash
fontcull https://example.com --subset "fonts/*.ttf" --output dist/
```

### Spider multiple pages

Crawl same-origin links to gather glyphs from multiple pages:

```bash
fontcull https://example.com --spider-limit 50 --subset "fonts/*.woff2"
```

### Filter by font family

Only include glyphs used by specific font families:

```bash
fontcull https://example.com --family "Inter,Roboto" --subset fonts/inter.ttf
```

### Add whitelist characters

Always include certain characters even if not detected:

```bash
fontcull https://example.com --whitelist "0123456789" --subset fonts/body.ttf
```

## Options

| Option | Short | Description |
|--------|-------|-------------|
| `--subset <PATTERN>` | `-s` | Font files to subset (glob patterns supported) |
| `--output <DIR>` | `-o` | Output directory for subset fonts |
| `--family <NAMES>` | `-f` | Only include glyphs from these font families (comma-separated) |
| `--spider-limit <N>` | | Maximum pages to crawl (0 = disabled) |
| `--whitelist <CHARS>` | `-w` | Characters to always include |

## Output

- Subset fonts are written as WOFF2 files with `-subset` suffix
- Without `--subset`, prints the unicode range to stdout

## Requirements

- A Chromium-based browser must be installed (Chrome, Chromium, Edge)
- The browser is launched headlessly via CDP

## License

MIT
