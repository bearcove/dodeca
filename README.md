# dodeca

[![MIT + Apache 2.0](https://img.shields.io/badge/license-MIT%20%2B%20Apache%202.0-blue)](./LICENSE-MIT)
[![salsa | yes please](https://img.shields.io/badge/salsa-yes%20please-green)](https://crates.io/crates/salsa)

A fully incremental static site generator.

## Philosophy

**Dev mode = Production mode.** Unlike other static site generators that take shortcuts
in development (skipping minification, using original asset paths, etc.), dodeca serves
exactly what you'll get in production: cache-busted URLs, minified HTML, subsetted fonts,
responsive images—the works. Even font subsetting runs in dev!

This is possible because dodeca uses [Salsa](https://salsa-rs.github.io/salsa/) for
incremental computation. Every transformation is a cached query. Change a file and only
the affected queries re-run. First page load builds what's needed; subsequent requests
are instant.

**Custom template engine.** Dodeca includes its own Jinja-like template engine. This gives
us precise error messages with source locations, lets us track template dependencies for
accurate incremental rebuilds, and avoids a serde dependency by working directly with native types.

**Cache-busting everywhere.** All assets get content-hashed URLs (`style.a1b2c3d4.css`), so
you can serve them with aggressive cache headers. This happens in dev too, so you catch
any URL rewriting issues before production.

**Minification by default.** HTML is minified with [minify-html](https://crates.io/crates/minify-html),
SVGs are optimized with [oxvg](https://crates.io/crates/oxvg). Both run in dev mode because
the performance cost is negligible and you want to catch issues early.

**Responsive image pipeline.** Images are automatically converted to JPEG-XL and WebP at
multiple breakpoints (320px to 1920px). Each image also gets a [thumbhash](https://evanw.github.io/thumbhash/)—a
tiny ~28 byte placeholder that displays instantly while the real image loads.

## Features

- incremental builds via [Salsa](https://salsa-rs.github.io/salsa/)
- font subsetting (only include glyphs actually used)
- OG image generation with Typst
- live-reload dev server
- Jinja-like template engine with macros and imports
- Sass/SCSS compilation
- search indexing via Pagefind
- internal and external link checking
- responsive images (JPEG-XL + WebP with thumbhash placeholders)
- cache-busted asset URLs (in dev and prod)
- HTML minification
- SVG optimization
- Zola-style `@/` internal links
- markdown extensions (tables, footnotes, strikethrough)
- automatic table of contents generation

## Installation

### Homebrew

```bash
brew install bearcove/tap/dodeca
```

### macOS / Linux

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.sh | sh
```

### Windows

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.ps1 | iex"
```

### With cargo-binstall

```bash
cargo binstall dodeca
```

See [cargo-binstall](https://crates.io/crates/cargo-binstall) for installation.

### From source

```bash
cargo install dodeca
```

## Usage

### Development

```bash
ddc serve
```

Starts a live-reload dev server with a TUI showing real-time build progress. Pages are built on-demand—only what you request gets built, making startup instant even for large sites.

Press `p` to expose your local server to a public address (via bore.pub), handy for testing on other devices or sharing previews.

### Production build

```bash
ddc build
```

Builds your entire site to the output directory. Thanks to Salsa's incremental computation and a content-addressed storage (CAS) system, subsequent builds only recompute what actually changed.

## Configuration

Create `.config/dodeca.kdl`:

```kdl
content "docs/content"
output "docs/public"
```

## Sponsors

Thanks to all individual sponsors:

<p> <a href="https://github.com/sponsors/fasterthanlime">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/github-dark.svg">
<img src="./static/sponsors-v3/github-light.svg" height="40" alt="GitHub Sponsors">
</picture>
</a> <a href="https://patreon.com/fasterthanlime">
    <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/patreon-dark.svg">
    <img src="./static/sponsors-v3/patreon-light.svg" height="40" alt="Patreon">
    </picture>
</a> </p>

...along with corporate sponsors:

<p> <a href="https://aws.amazon.com">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/aws-dark.svg">
<img src="./static/sponsors-v3/aws-light.svg" height="40" alt="AWS">
</picture>
</a> <a href="https://zed.dev">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/zed-dark.svg">
<img src="./static/sponsors-v3/zed-light.svg" height="40" alt="Zed">
</picture>
</a> <a href="https://depot.dev?utm_source=facet">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/depot-dark.svg">
<img src="./static/sponsors-v3/depot-light.svg" height="40" alt="Depot">
</picture>
</a> </p>

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
