# aasvg

[![Crates.io](https://img.shields.io/crates/v/aasvg.svg)](https://crates.io/crates/aasvg)
[![Documentation](https://docs.rs/aasvg/badge.svg)](https://docs.rs/aasvg)
[![License](https://img.shields.io/crates/l/aasvg.svg)](LICENSE)

A Rust port of [aasvg](https://github.com/martinthomson/aasvg), which converts ASCII art diagrams into SVG.

![example](./example.svg)

## Usage

```rust
use aasvg::render;

let diagram = r#"
    +-----+     +-----+
    |     |---->|     |
    +-----+     +-----+
"#;

let svg = render(diagram);
```

The output SVG automatically supports light and dark mode via CSS `prefers-color-scheme`.

## Attribution

This project is a Rust port of Martin Thomson's [aasvg](https://github.com/martinthomson/aasvg), which itself uses the diagram rendering code from Morgan McGuire's [Markdeep](https://casual-effects.com/markdeep/).

Related projects:
- [goat](https://github.com/blampe/goat) - A Go implementation that reimplements the Markdeep algorithm

## License

BSD-2-Clause, matching the original aasvg license.
