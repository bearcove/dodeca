# Pikru Parity Refactor Plan (Rust-first, zero-cost abstractions)

## Core Ideas
- Make units explicit: lengths (inches), scalars, angles, colors as distinct newtypes.
- Separate geometry from serialization: draw in inches, convert to px once at emit.
- Centralize state/resolution: one context for variables, directions, object lookup.
- Treat outputs as variants: SVG vs print/assert diagnostics.
- Mirror C layout semantics; rScale applied only at emission.

## Planned Types
- `Length(pub f64)` inches; `Px(pub f64)`; `Angle(pub f64)` degrees; `Scalar(pub f64)`.
- `Color` enum: Named(String) | Rgb(u8,u8,u8) | Rgba(u8,u8,u8,u8) | Raw(String).
- `Point<T>` with `x: T, y: T`; `Size<T>` for width/height.
- `Stroke { color: Color, width: Length, dash: DashPattern, invisible: bool }`
- `Fill { color: Color }`
- `DashPattern` enum (Solid, Dotted(Length,Length), Dashed(Length,Length)).
- `Shape<Len>`: Box, Circle, Ellipse, Line { pts: Vec<Point<Len>> }, etc.
- `Drawable<Len>`: { shape: Shape<Len>, stroke: Stroke, fill: Fill, text: Vec<TextSpan> }.
- `TextSpan { pos: Point<Len>, value: String, font: FontSpec }`.
- `Scaler { r_scale: f64 }` with `fn to_px(Point<Len>) -> Point<Px>`, etc.
- `EvalValue`: Length | Scalar | Color | Angle (type-checked ops).
- `RenderResult`: Svg(String) | Html(String) | Error(Diagnostic).
- `LayoutContext`: dir/pos, typed vars, last/prev objects, bbox in inches; helpers `advance(len)`, `place`, `edge`, `even_with`, `until_even_with`.

## Pipeline
1. Parse → AST (unchanged grammar).
2. Macro expand.
3. Evaluate AST into `Drawable<Length>` using `LayoutContext`:
   - context holds current dir/pos, vars (typed maps), last/prev objects.
   - helpers: `advance(len)`, `place(obj_ref)`, `edge(obj_ref, ep)`.
4. Emit: map `Drawable<Length>` to `Drawable<Px>` via `Scaler`; serialize SVG (formatters on Color/Stroke/Fill).
5. Print/assert: produce Html/Error variants; tests can compare accordingly.

## Incremental Steps
1) Introduce newtypes (`Length`, `Scalar`, `Angle`, `Color`) + `Scaler`; adjust formatting helpers. **(types done; Scaler ready)**
2) Refactor emitter to use inch space + `Scaler` for all coords/strokes/dashes/text; ensure bbox/margins stay in inches. **(partially done)**
3) Port layout semantics: advance, centering, even/with, until even with, chop/arrowheads, sublist local coords, bbox accumulation (in inches).
4) Rework evaluator to return `EvalValue`; enforce unit-safe arithmetic; propagate math errors for assert/print diagnostics.
5) Model `Drawable` and populate from AST (box/circle/line/arrow/spline/text) in inches; add style (stroke/fill/dash from vars/dashwid) and text spans with char metrics.
6) Emit: apply `Scaler` once; dash arrays from `dashwid`, arrow sizes from `arrowwid/arrowht`, stroke widths from `thickness`.
7) Print/assert path: produce HTML with `<br>` and C-like error lines; fall back to SVG only when objects exist.
8) Style parity: colors (hex/rgb/rgba/named), fg/bg vars, font-size initial, data-pikchr-date (manifest date) and class="pikchr".
9) Margins: apply margin+thickness+side margins in inches before viewBox.

## Notes
- Keep public API `pikchr(&str) -> Result<String>` returning SVG; add internal helper for Html diagnostics.
- Zero-cost: newtypes wrap `f64`; conversions inline; no runtime overhead beyond method calls optimized away.
