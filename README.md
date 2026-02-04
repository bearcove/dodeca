# dodeca

[![MIT + Apache 2.0](https://img.shields.io/badge/license-MIT%20%2B%20Apache%202.0-blue)](./LICENSE-MIT)
[![salsa | yes please](https://img.shields.io/badge/salsa-yes%20please-green)](https://crates.io/crates/salsa)

A fully incremental static site generator.

## Philosophy

**Dev mode = Production mode.** Unlike other static site generators that take shortcuts
in development (skipping minification, using original asset paths, etc.), dodeca serves
exactly what you'll get in production: cache-busted URLs, minified HTML, subsetted fonts,
responsive images—the works. Even font subsetting runs in dev!

This is possible because dodeca uses picante for incremental computation. Every
transformation is a cached query. Change a file and only the affected queries re-run.
First page load builds what's needed; subsequent requests are instant.

**Custom template engine.** Dodeca includes its own Jinja-like template engine. This gives
you the power of a real language (conditionals, loops, variable interpolation) without
the complexity of learning a new syntax. The engine includes rich diagnostics with
line/column information for template errors.

**Plugin architecture.** Dodeca's plugin system (via the `plugcard` crate) allows extending
the build pipeline without modifying the core. Image optimization, CSS processing, syntax
highlighting, and more can be plugged in independently.

## Getting Started

Install dodeca:

```bash
cargo install dodeca
```

Create a new site:

```bash
ddc init my-site
cd my-site
ddc serve
```

This starts a development server at `http://localhost:8080` with live reload.

## Key Features

- **Incremental builds**: Only affected files are re-processed on changes
- **Cache-busted URLs**: Assets get unique names based on content hash
- **Image optimization**: Built-in image processing with responsive variants
- **Font subsetting**: Automatically subset fonts to used characters
- **HTML minification**: Minify HTML in production builds
- **Template engine**: Powerful, easy-to-learn Jinja-like templates
- **Code execution**: Execute code samples and capture output
- **Link checking**: Verify all links in your site
- **Extensible**: Plugin system for custom transformations

## Structure

The dodeca workspace includes:

- **dodeca**: Main SSG binary (`ddc` CLI)
- **gingembre**: Template engine with rich diagnostics
- **plugcard**: Plugin system for extending the build pipeline
- **dodeca-\***: Individual plugins for various transformations
- **livereload-client**: WASM client for live reload in development

## Development

Build everything:

```bash
cargo xtask build
```

Run tests:

```bash
cargo test --workspace
```

Build the documentation:

```bash
cargo doc --workspace --no-deps --open
```

## Contributing

Contributions are welcome! Please open issues and pull requests on GitHub.


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

...without whom this work could not exist.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
