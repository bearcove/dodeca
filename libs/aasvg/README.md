# aasvg

Convert ASCII art diagrams to SVG with automatic light/dark mode support.

This is a Rust port of [aasvg](https://github.com/martinthomson/aasvg) by Martin Thomson,
which is itself derived from [Markdeep](https://casual-effects.com/markdeep/) by Morgan McGuire.

## Features

- **Light/Dark Mode**: SVG output uses CSS variables with `prefers-color-scheme` media query
- **Zero Dependencies**: Pure Rust implementation
- **Comprehensive**: Supports lines, arrows, boxes, curves, points, and text

## Usage

```rust
use aasvg::render;

let diagram = r#"
+--------+     +--------+
| Hello  |---->| World  |
+--------+     +--------+
"#;

let svg = render(diagram);
println!("{}", svg);
```

### With Options

```rust
use aasvg::{render_with_options, RenderOptions};

let options = RenderOptions::new()
    .with_backdrop(true)    // Add background rectangle
    .with_stretch(true);    // Stretch text to fit cells

let svg = render_with_options(diagram, &options);
```

## Supported Elements

| Element | Characters | Description |
|---------|-----------|-------------|
| Lines | `-` `│` `\|` `/` `\` | Horizontal, vertical, diagonal |
| Double lines | `=` `║` | Thick/emphasized lines |
| Squiggle | `~` | Wavy horizontal lines |
| Vertices | `+` `.` `'` `,` `` ` `` | Connection points |
| Arrows | `>` `<` `^` `v` `V` | Directional indicators |
| Points | `*` `o` `●` `○` | Markers on lines |
| Jumps | `(` `)` | Line crossings |

## Example

```
+--------+
| Input  |
+---+----+
    |
    v
+---+----+     .--------.
| Parse  |---->| Output |
+--------+     '--------'
```

## Light/Dark Mode

The generated SVG includes CSS variables that automatically adapt to the user's
color scheme preference:

```css
:root {
  --aasvg-stroke: #000;
  --aasvg-fill: #000;
  --aasvg-bg: #fff;
  --aasvg-text: #000;
}
@media (prefers-color-scheme: dark) {
  :root {
    --aasvg-stroke: #fff;
    --aasvg-fill: #fff;
    --aasvg-bg: #1a1a1a;
    --aasvg-text: #fff;
  }
}
```

## Attribution

This crate is a Rust port of:
- [aasvg](https://github.com/martinthomson/aasvg) by Martin Thomson (BSD-2-Clause)
- [Markdeep](https://casual-effects.com/markdeep/) by Morgan McGuire (BSD-2-Clause)

## License

BSD-2-Clause - See [LICENSE](LICENSE) for details.
