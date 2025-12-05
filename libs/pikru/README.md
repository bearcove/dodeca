# pikru

[![MIT + Apache 2.0](https://img.shields.io/badge/license-MIT%20%2B%20Apache%202.0-blue)](./LICENSE-MIT)
[![experimental](https://img.shields.io/badge/status-experimental-orange)]()

A Rust implementation of [pikchr](https://pikchr.org/), a PIC-like diagram markup language that generates SVG.

## About

Pikchr (pronounced "picture") is a diagram description language designed for embedding in Markdown fenced code blocks. This crate provides a pure Rust implementation.

## Status

🚧 **Work in Progress** - Early-stage port of the C implementation.

## Development

### Testing

The test suite compares Rust output against the original C implementation:

```bash
# Run specific tests (recommended - full suite can hang)
cargo test test01 -- --nocapture
cargo test test12 -- --nocapture

# Run unit tests (these also update visual comparison)
cargo test
```

### Visual Comparison

A visual comparison HTML file is automatically generated showing side-by-side comparisons of C vs Rust SVG output:

```bash
# Manually generate comparison HTML
cargo xtask compare-html
```

Individual SVG files are written to `debug-svg/` for inspection:
- `debug-svg/test01-c.svg` - C implementation output
- `debug-svg/test01-rust.svg` - Rust implementation output
- Files are named as `{test-name}-{c|rust}.svg`

The comparison is automatically updated:
- When running `cargo test` (via `z_update_visual_comparison` test)
- In pre-commit hooks (if installed)

### Pre-commit Hooks

Install git hooks to automatically update comparisons before commits:

```bash
./scripts/install-hooks.sh
```

This ensures `comparison.html` is always up-to-date with the latest code changes.

## Reference Source

The original C implementation from upstream `pikchr` is vendored under `vendor/pikchr-c/` for easy access during parity work. It is not part of the Rust crate build.

## Continuous Integration

Our CI runs on [Depot](https://depot.dev) hosted runners &mdash; huge thanks to Depot for sponsoring the compute that keeps the pikru test suite flying.

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
<a href="https://aws.amazon.com">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/aws-dark.svg">
<img src="./static/sponsors-v3/aws-light.svg" height="40" alt="AWS">
</picture>
</a>
<a href="https://zed.dev">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/zed-dark.svg">
<img src="./static/sponsors-v3/zed-light.svg" height="40" alt="Zed">
</picture>
</a>
<a href="https://depot.dev?utm_source=facet">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/depot-dark.svg">
<img src="./static/sponsors-v3/depot-light.svg" height="40" alt="Depot">
</picture>
</a>
</p>

## License

MIT OR Apache-2.0
