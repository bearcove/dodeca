# Pikru Parity Refactor Plan

A roadmap for achieving C-pikchr compatibility using Rust-first, zero-cost abstractions.

## Design Principles

1. **Explicit units** - Lengths (inches), pixels, scalars, and angles as distinct newtypes prevent unit confusion at compile time.
2. **Geometry/serialization separation** - Layout in inches; convert to pixels once at emit time.
3. **Centralized state** - Single context for variables, directions, current position, object lookup.
4. **Output variants** - SVG for diagrams, HTML for print/assert diagnostics.
5. **C layout semantics** - Match the reference implementation; rScale applied only at emission.

## Type System

```
                    ┌─────────────────────────────────────────┐
    Primitives      │  Length(f64)   - inches (canonical)     │
                    │  Px(f64)       - pixels (after scaling) │
                    │  Scalar(f64)   - unitless               │
                    │  Angle(f64)    - degrees                │
                    │  Color         - Named|Rgb|Rgba|Raw     │
                    └─────────────────────────────────────────┘
                                        │
                    ┌───────────────────▼───────────────────┐
    Geometry        │  Point<T>    { x: T, y: T }           │
    (generic)       │  Size<T>     { w: T, h: T }           │
                    │  BBox<T>     { min: Point, max: Point }│
                    └───────────────────────────────────────┘
                                        │
                    ┌───────────────────▼───────────────────┐
    Aliases         │  PtIn  = Point<Length>                │
                    │  PtPx  = Point<Px>                    │
                    │  BoxIn = BBox<Length>                 │
                    └───────────────────────────────────────┘
                                        │
                    ┌───────────────────▼───────────────────┐
    Conversion      │  Scaler { r_scale: f64 }              │
                    │    .len(Length) -> Px                 │
                    │    .point(Point<Length>) -> Point<Px> │
                    └───────────────────────────────────────┘
```

### Planned (not yet implemented)

| Type | Purpose |
|------|---------|
| `Stroke` | `{ color, width: Length, dash: DashPattern, invisible: bool }` |
| `Fill` | `{ color: Color }` |
| `DashPattern` | `Solid \| Dotted(Length, Length) \| Dashed(Length, Length)` |
| `Shape<T>` | `Box \| Circle \| Ellipse \| Line { pts: Vec<Point<T>> } \| ...` |
| `Drawable<T>` | `{ shape, stroke, fill, text: Vec<TextSpan> }` |
| `TextSpan` | `{ pos: Point<T>, value: String, font: FontSpec }` |
| `EvalValue` | `Length \| Scalar \| Color \| Angle` for type-checked expr eval |
| `LayoutContext` | dir, pos, typed vars, last/prev objects, bbox accumulator |

## Pipeline

```
  ┌───────┐    ┌────────┐    ┌──────────────────┐    ┌─────────────────┐
  │ Parse │───▶│ Expand │───▶│ Evaluate (inches)│───▶│ Emit (pixels)   │
  │  AST  │    │ Macros │    │  LayoutContext   │    │ Scaler → SVG    │
  └───────┘    └────────┘    └──────────────────┘    └─────────────────┘
                                      │
                              ┌───────▼───────┐
                              │ Drawable<Len> │
                              │ with styles   │
                              └───────────────┘
```

## Progress Tracker

### Done

- [x] **Primitives**: `Length`, `Px`, `Scalar`, `Angle`, `Color` newtypes
- [x] **Geometry**: Generic `Point<T>`, `Size<T>`, `BBox<T>`
- [x] **Scaler**: `len()`, `point()` for Length→Px conversion
- [x] **Operator traits**: `Add`, `Sub`, `Mul<f64>`, `Div<f64>`, `AddAssign`, `SubAssign` on `Length`
- [x] **Tests**: `even_with` and `until_even_with` parity tests against C

### In Progress

- [ ] Wire typed geometry into `render.rs` (replace `Point { x: Length, y: Length }` with `PtIn`)
- [ ] Refactor emitter to use `Scaler` for all coord/stroke/dash conversions

### Pending

| Step | Description |
|------|-------------|
| 3 | Layout semantics: advance, centering, chop/arrowheads, sublist local coords, bbox in inches |
| 4 | Evaluator returns `EvalValue`; unit-safe arithmetic; math error diagnostics |
| 5 | `Drawable<Length>` model populated from AST; styles from vars |
| 6 | Emit: dash arrays from `dashwid`, arrow sizes, stroke widths via Scaler |
| 7 | Print/assert: HTML output with `<br>` and C-like error lines |
| 8 | Style parity: hex/rgb/rgba colors, fg/bg vars, font-size, `data-pikchr-date`, `class="pikchr"` |
| 9 | Margins: margin + thickness + side margins in inches before viewBox |

## Notes

- Public API stays `pikchr(&str) -> Result<String>` returning SVG.
- Zero-cost: newtypes wrap `f64`; conversions inline; no runtime overhead.
- All layout math happens in inch space; pixel conversion is a one-time final pass.
