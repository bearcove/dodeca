# Strong Types Playbook

Guidelines for keeping Pikru layout/emit code safe and unit-correct using Rust types.

## Goals

- No raw `f64` in domain logic: lengths, pixels, scalars, and angles are distinct.
- Illegal states unrepresentable: construction fails early; conversions explicit.
- Errors stay structured: type/arith issues reported with context, never panics.

## Core Rules

- Keep the base newtypes: `Length`, `Px`, `Scalar`, `Angle`, `Color` (all `Copy`, `#[repr(transparent)]`).
- Implement only valid ops (e.g., `Length + Length`, `Length * f64`; forbid `Length + Scalar`).
- Provide `Display`/`Debug` on newtypes for diagnostics; avoid exposing inner `f64` publicly.
- Convert spaces only via `Scaler`; inch-space types never mixed with pixel-space types.

## Constructors & Validation

- `Length::try_new(f64)`, `Angle::from_degrees`, `from_radians` reject NaN/inf.
- `Scaler::try_new(r_scale: NonZeroScalar)`; disallow zero/negative scale factors.
- Prefer `NonZeroLength`, `PositiveScalar`, `Opacity` wrappers where semantics require.
- `Color::try_from_css(&str)` returns typed errors; no `unwrap`/`expect` on parsing.

## Geometry & Units

- Use typed geometry aliases everywhere: `Point<Length>`, `BBox<Length>`, `Size<Length>`.
- Layout/state stays in inches; emit converts to pixels once with `Scaler::point/len/size/bbox`.
- Add `PhantomData<Inches>`/`PhantomData<Pixels>` to structs when the coordinate space matters.

## Styling & Drawables

- `Stroke { width: Length, dash: DashPattern, invisible: bool }`
- `DashPattern = Solid | Dotted { on: Length, off: Length } | Dashed { on: Length, off: Length }`
- `TextSpan { pos: Point<Length>, font: FontSpec, value: SmolStr }`
- Shapes remain generic on space: `Shape<T> = Box | Circle | Ellipse | Line { pts: Vec<Point<T>> } | ...`

## State & Errors

- `LayoutContext` holds typed fields: `dir: Direction`, `pos: Point<Length>`, `bbox: BBox<Length>`, typed vars map.
- Expression evaluator returns `EvalValue = Length | Scalar | Angle | Color`; arithmetic only on compatible variants.
- `EvalError` variants: `DivisionByZero`, `SqrtOfNegative`, `TypeMismatch { expected, got }`, `UndefinedVar`, `Overflow`.

## Testing Guardrails

- Add UI/`trybuild` compile-fail cases to forbid `Length + Angle`, `Scaler::len(Px)`, etc.
- Property tests for `Scaler` round-trips (inches → px → inches within tolerance).
- Golden tests ensure emitted SVG respects scale for text and stroke widths.
- Diagnostic tests assert error variants/messages for divide-by-zero and sqrt-negative paths.

## Reviewer Checklist

- Are there any raw `f64` or tuples in layout/state code? Replace with typed newtypes.
- Do constructors reject NaN/inf/zero where required? No `unwrap` on parse.
- Are conversions between inch/pixel spaces only via `Scaler`? No ad hoc multipliers.
- Are dash/stroke/text structures carrying typed units? No magic numbers.
- Do tests cover both correctness and compile-time rejections of invalid mixes?

## Implementation Status

### Done ✓

- `Length`: newtype with `try_new`, `try_non_negative`, `abs`, `min`, `max`, `checked_div`, `is_finite`
- `Scalar`: newtype with `raw()`, `is_finite()`; `Scalar * Length = Length`
- `Length::checked_div() → Option<Scalar>` (no unchecked `/` trait to prevent silent infinity)
- `Offset<T>`: displacement vector type; `Point + Offset = Point`, `Point - Point = Offset`
- `UnitVec`: normalized direction vectors with `FRAC_1_SQRT_2` for diagonals; `UnitVec * Length → Offset`
- `Point<Length>::midpoint()`: typed midpoint calculation
- `BBox<Length>`: typed `width()`, `height()`, `size()`, `center()`, `is_empty()`
- `Scaler::try_new()`: rejects NaN/infinite/zero/negative scale factors; wired into `generate_svg`
- `NumericError`: error type for validation failures
- `defaults` module uses typed `Inches` constants
- `EvalValue` enum (`Length | Scalar | Color`) replaces `HashMap<String, f64>` for typed variable storage
- Variable initialization categorized: lengths, scalars, colors stored with proper `EvalValue` variants
- Bidirectional `Value ↔ EvalValue` conversions for expression evaluation
- Evaluator uses `Length::try_new` for parsed numbers; validates results are finite after arithmetic
- Evaluator uses typed ops (`a + b`, `a.abs()`, `a.checked_div(b)`) instead of raw `f64` access
- `validate_value()` helper catches overflow to infinity/NaN in expression results
- `Angle`: newtype with `try_new`, `from_degrees`, `from_radians`, `degrees`, `to_radians`, `raw`, `is_finite`, `normalized`

### Status

- Type-safety milestone is complete. Compile-fail (trybuild) UI tests are deferred so we can focus on the PARITY_PLAN layout semantics work; reintroduce them later if we need automated guardrails for invalid type mixes.
