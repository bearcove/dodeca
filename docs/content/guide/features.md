+++
title = "Features"
description = "What dodeca offers"
weight = 40
+++

## Build

- **Incremental builds** via [Salsa](https://salsa-rs.github.io/salsa/) - only rebuild what changed
- **Sass/SCSS compilation** - modern CSS workflow built-in
- **Search indexing** via Pagefind - fast client-side search
- **Link checking** - catch broken internal and external links

## Runtime

- **DOM patching** - no full reload, just surgical DOM updates via WASM
- **Live-reload dev server** - instant feedback while editing

## Assets

- **Font subsetting** - only include glyphs actually used on your site (saves TONS of bandwidth!)
- **Responsive images** - automatic JXL/WebP variants at multiple sizes

## Templating

- **Jinja-like template engine** - familiar syntax, zero serde

## Code Sample Execution

- **Automatic code sample validation** - ensures your documentation examples actually work
- **Rust code execution** with proper dependency management
- **Cached execution** - fast rebuilds with content hashing via Blake3

Here's an example that will be automatically validated:

```rust
use std::collections::HashMap;

fn main() {
    let mut map = HashMap::new();
    map.insert("hello", "world");
    println!("Map contains {} entries", map.len());
}
```

This code sample will be compiled and executed during the build process to ensure it works correctly.
