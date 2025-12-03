# fontcull

Font subsetting library. Subset fonts to only include glyphs that are actually used.

## Features

- **No Python** - No fonttools/pyftsubset dependency, just Rust + C++ for WOFF2
- **Multiple formats** - Supports TTF, OTF, and WOFF2 input
- **WOFF2 output** - Compress subsetted fonts to WOFF2 for web delivery
- **Static analysis** (optional) - Parse HTML/CSS to detect font usage

## Usage

```rust
use fontcull::{subset_font_to_woff2, decompress_font};
use std::collections::HashSet;

// Load a font file
let font_data = std::fs::read("MyFont.ttf").unwrap();

// Define which characters to keep
let chars: HashSet<char> = "Hello World".chars().collect();

// Subset and compress to WOFF2
let woff2 = subset_font_to_woff2(&font_data, &chars).unwrap();
std::fs::write("MyFont-subset.woff2", woff2).unwrap();
```

### With WOFF2 input

```rust
use fontcull::{decompress_font, subset_font_data, compress_to_woff2};
use std::collections::HashSet;

let woff2_input = std::fs::read("MyFont.woff2").unwrap();

// Decompress → Subset → Recompress
let decompressed = decompress_font(&woff2_input).unwrap();
let chars: HashSet<char> = "Hello".chars().collect();
let subsetted = subset_font_data(&decompressed, &chars).unwrap();
let woff2_output = compress_to_woff2(&subsetted).unwrap();
```

### Static HTML/CSS analysis

Enable the `static-analysis` feature to parse HTML and CSS for font usage:

```toml
[dependencies]
fontcull = { version = "2", features = ["static-analysis"] }
```

```rust
use fontcull::{analyze_fonts, extract_css_from_html, subset_font_to_woff2};

let html = r#"<html>
  <head><style>body { font-family: "MyFont"; }</style></head>
  <body><p>Hello World</p></body>
</html>"#;

let css = extract_css_from_html(html);
let analysis = analyze_fonts(html, &css);

if let Some(chars) = analysis.chars_per_font.get("MyFont") {
    let font_data = std::fs::read("MyFont.ttf").unwrap();
    let woff2 = subset_font_to_woff2(&font_data, chars).unwrap();
    std::fs::write("MyFont-subset.woff2", woff2).unwrap();
}
```

## API

### Core functions

- `subset_font_data(font_data, chars)` - Subset font to TTF bytes
- `subset_font_to_woff2(font_data, chars)` - Subset and compress to WOFF2
- `decompress_font(font_data)` - Decompress WOFF2 to TTF/OTF
- `compress_to_woff2(font_data)` - Compress TTF/OTF to WOFF2

### Unicode codepoint variants

- `subset_font_data_unicode(font_data, unicodes)` - Subset using `u32` codepoints
- `subset_font_to_woff2_unicode(font_data, unicodes)` - Subset to WOFF2 using codepoints

### Format detection

- `FontFormat::detect(data)` - Detect font format from magic bytes

## License

MIT
