# pikru

[![crates.io](https://img.shields.io/crates/v/pikru.svg)](https://crates.io/crates/pikru)
[![docs.rs](https://docs.rs/pikru/badge.svg)](https://docs.rs/pikru)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](./LICENSE-MIT)

A Rust implementation of [pikchr](https://pikchr.org/), a PIC-like diagram
markup language for creating technical diagrams that generates SVG.

## Usage

```rust
let svg = pikru::pikchr(r#"box "Hello" arrow box "World""#).unwrap();
assert!(svg.contains("<svg"));
```

## Light/Dark Mode

Enable CSS variables for automatic light/dark theming:

```rust
use pikru::{pikchr_with_options, RenderOptions};

let options = RenderOptions { css_variables: true };
let svg = pikchr_with_options(r#"box "Hello""#, &options).unwrap();
assert!(svg.contains("light-dark("));
```

The generated SVG includes a `<style>` block with CSS variables using
`light-dark()`, so colors automatically adapt to the user's color scheme.

## Development

### Testing

The test suite validates output against the original C implementation:

```bash
cargo test                     # Run all tests
cargo xtask compare-html       # Generate visual comparison HTML
cargo xtask generate-pngs      # Convert SVGs to PNGs for debugging
```

See `comparison.html` for a side-by-side visual comparison of C vs Rust output.

### Pre-commit Hooks

```bash
./hooks/install.sh
```

## Attribution

pikru is a direct Rust port of:

- **[pikchr](https://pikchr.org/)** by D. Richard Hipp - The original C
  implementation, released under the
  [0-clause BSD license](https://pikchr.org/home/doc/trunk/doc/license.md).
  The C source is vendored in `vendor/pikchr-c/` for reference.

Also referenced during development:

- **[gopikchr](https://github.com/gopikchr/gopikchr)** - A Go port of pikchr
  by [@zellyn](https://github.com/zellyn).

## Continuous Integration

Our CI runs on [Depot](https://depot.dev) hosted runners &mdash; huge thanks
to Depot for sponsoring the compute that keeps the pikru test suite flying.

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

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.
