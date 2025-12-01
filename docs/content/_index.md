+++
title = "dodeca"
description = "A fully incremental static site generator"
+++

<img src="/images/dodeca-logo.svg" alt="Dodeca logo" style="float: right; margin-left: 1em; width: 150px;">

A fully incremental static site generator built with Rust.

![Mountain landscape](/images/mountain.jpg)

*Photo by [Samuel Ferrara](https://unsplash.com/@samferrara) on Unsplash (CC0)*

## Features

- **Incremental builds** via [Salsa](https://salsa-rs.github.io/salsa/) - only rebuild what changed
- **Font subsetting** - only include glyphs actually used on your site
- **OG image generation** with Typst - beautiful social cards automatically
- **Live-reload dev server** - instant feedback while editing
- **Jinja-like template engine** - familiar syntax, zero serde
- **Sass/SCSS compilation** - modern CSS workflow built-in
- **Search indexing** via Pagefind - fast client-side search
- **Link checking** - catch broken internal and external links

## Installation

### macOS / Linux

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.sh | sh
```

### Windows

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.ps1 | iex"
```

### Homebrew

```bash
brew install bearcove/tap/dodeca
```

### From source

```bash
cargo install dodeca
```

## Quick Start

```bash
# Build your site
ddc build

# Serve with live reload
ddc serve
```

## Configuration

Create `.config/dodeca.kdl` in your project root:

```kdl
content "docs/content"
output "docs/public"
```

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
