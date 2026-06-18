# Pikru Design & Progress

A Rust implementation of pikchr aiming for C-pikchr compatibility using zero-cost abstractions.

## Design Principles

1. **Explicit units** - Lengths (inches), pixels, scalars, and angles as distinct newtypes prevent unit confusion at compile time.
2. **Geometry/serialization separation** - Layout in inches; convert to pixels once at emit time.
3. **Centralized state** - Single context for variables, directions, current position, object lookup.
4. **C layout semantics** - Match the reference implementation; rScale applied only at emission.

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

## Pipeline

```
  ┌───────┐    ┌────────┐    ┌──────────────────┐    ┌─────────────────┐
  │ Parse │───▶│ Expand │───▶│ Evaluate (inches)│───▶│ Emit (pixels)   │
  │  AST  │    │ Macros │    │  LayoutContext   │    │ Scaler → SVG    │
  └───────┘    └────────┘    └──────────────────┘    └─────────────────┘
```

## Coordinate System

C pikchr uses Y-up internally and flips to Y-down (SVG) at render time via `pik_append_xy`:
```c
y = bbox.ne.y - y;  // Y-flip
```

Rust pikru does the same: internal coordinates are Y-up, `to_svg()` method handles the flip.

## Progress Tracker

### Done

- [x] Primitives: `Length`, `Px`, `Scalar`, `Angle`, `Color` newtypes
- [x] Geometry: Generic `Point<T>`, `Size<T>`, `BBox<T>`
- [x] Scaler: `len()`, `point()`, `px()`, `size()`, `bbox()` methods
- [x] Operator traits on `Length`
- [x] Typed geometry in render.rs: `PtIn`, `BoxIn` aliases
- [x] Scaler in emitter: All coord/stroke/dash conversions via `Scaler`
- [x] Y-flip: `Point<Length>::to_svg()` with `max_y - y` transformation
- [x] Direction vectors: `Direction::unit_vector()` and `Direction::offset()`
- [x] Typed evaluator with `EvalValue` enum
- [x] Angle safety: validated constructors/accessors

### In Progress

- [ ] Layout semantics parity (advance/centering rules, chop + arrowheads, sublist local coords)

### Pending

| Step | Description |
|------|-------------|
| 3 | Layout semantics: advance, centering, chop/arrowheads, sublist local coords |
| 5 | `Drawable<Length>` model populated from AST; styles from vars |
| 6 | Emit: dash arrays from `dashwid`, arrow sizes, stroke widths via Scaler |
| 7 | Print/assert: HTML output with `<br>` and C-like error lines |
| 8 | Style parity: hex/rgb/rgba colors, fg/bg vars, font-size, `class="pikchr"` |
| 9 | Margins: margin + thickness + side margins in inches before viewBox |

## Notes

- Public API stays `pikchr(&str) -> Result<String>` returning SVG.
- Zero-cost: newtypes wrap `f64`; conversions inline; no runtime overhead.
- All layout math happens in inch space; pixel conversion is a one-time final pass.
