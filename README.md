# dodeca

[![MIT/Apache-2.0 licensed](https://img.shields.io/crates/l/dodeca.svg)](./LICENSE)
[![Discord](https://img.shields.io/discord/1379550208551026748?logo=discord&label=discord)](https://discord.gg/JhD7CwCJ8F)

A fully incremental static site generator. Features:

- Incremental builds via [Salsa](https://salsa-rs.github.io/salsa/)
- Font subsetting (only include glyphs actually used)
- OG image generation with Typst
- Live-reload dev server
- Jinja-like template engine
- Sass/SCSS compilation
- Search indexing via Pagefind
- Internal and external link checking

## Installation

```bash
# With cargo-binstall (recommended)
cargo binstall dodeca

# Or from source
cargo install dodeca
```

## Usage

```bash
# Build site
ddc build

# Serve with live reload
ddc serve
```

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
