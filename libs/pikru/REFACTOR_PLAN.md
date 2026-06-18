# Plan: Add RenderedObject Parameter to Shape::render_svg()

## Goal
Enable Line/Arrow shapes to access attachment information for auto-chop logic by passing `&RenderedObject` to all shape `render_svg()` methods.

## Approach: Clean Architecture
Pass the full `&RenderedObject` to all shapes for consistency and future extensibility.

**Rationale:**
1. **Consistency** - All shapes have the same signature, easier to understand
2. **Future-proof** - If other shapes need name, attachments, or future fields, the API is already ready
3. **Simple** - No need for optional parameters or special cases
4. **Existing pattern** - The `obj` is already available in svg.rs where we call `render_svg()`

## Signature Change

### Current (broken)
```rust
fn render_svg(&self, scaler: &Scaler, offset_x: Inches, max_y: Inches, dashwid: Inches) -> Vec<SvgNode>;
```

### New (with RenderedObject)
```rust
fn render_svg(&self, obj: &RenderedObject, scaler: &Scaler, offset_x: Inches, max_y: Inches, dashwid: Inches) -> Vec<SvgNode>;
```

**Parameter order rationale:** `obj` comes first as the primary context, followed by rendering parameters.

## Implementation Steps

### 1. Update Shape trait signature
**File:** `src/render/shapes.rs:95`
- Change trait method signature to include `obj: &RenderedObject` parameter
- Update trait documentation

### 2. Update all Shape implementations
**File:** `src/render/shapes.rs`

Update these implementations (all currently at various line numbers):
- CircleShape::render_svg (line 223)
- BoxShape::render_svg (line 301)
- EllipseShape::render_svg (line 383)
- OvalShape::render_svg (line 453)
- DiamondShape::render_svg (line 522)
- CylinderShape::render_svg (line 593)
- FileShape::render_svg (line 685)
- LineShape::render_svg (line 809) - **needs auto-chop logic added**
- SplineShape::render_svg (line 932)
- DotShape::render_svg (line 1032)
- TextShape::render_svg (line 1096) - no-op, just update signature
- ArcShape::render_svg (line 1229)
- MoveShape::render_svg (line 1367) - no-op, just update signature
- SublistShape::render_svg (line 1423)

**Changes per shape:**
- Add `obj: &RenderedObject` parameter (most shapes won't use it yet)
- For LineShape: Port auto-chop logic from svg.rs
- For SplineShape: Verify arrowhead logic is complete
- For ArcShape: Already has arrowhead logic

### 3. Port Line/Arrow auto-chop logic to LineShape
**File:** `src/render/shapes.rs` LineShape::render_svg

Port from `src/render/svg.rs:353-490`:
- Call `apply_auto_chop_simple_line()` using `obj.start_attachment` and `obj.end_attachment`
- Render arrowhead polygons before line
- Chop line endpoints for arrowheads (by arrowht/2)
- Handle multi-segment polylines with chop

**Required imports to add:**
```rust
use super::geometry::{apply_auto_chop_simple_line, chop_line};
use facet_svg::Polygon;
```

**Required access:**
- `obj.start_attachment` - for auto-chop
- `obj.end_attachment` - for auto-chop
- Arrow dimensions from defaults (already available)

### 4. Update svg.rs call site
**File:** `src/render/svg.rs:154`

Change from:
```rust
let shape_nodes = obj.shape.render_svg(&scaler, offset_x, max_y, dashwid);
```

To:
```rust
let shape_nodes = obj.shape.render_svg(obj, &scaler, offset_x, max_y, dashwid);
```

### 5. Delete bypassed code
**File:** `src/render/svg.rs:158-639`

Remove the entire `if false { match obj.class() { ... } }` block once Line/Arrow logic is ported.

## Files to Modify
1. `src/render/shapes.rs` - Shape trait + all 14 implementations
2. `src/render/svg.rs` - Update call site, delete old code
3. `src/render/geometry.rs` - Export `apply_auto_chop_simple_line` (may already be pub)

## Testing Strategy
1. After trait signature change: Verify compilation
2. After LineShape auto-chop: Run `cargo test autochop` to verify auto-chop works
3. After all changes: Run `cargo test` to verify no regressions
4. Specific test: `cargo test diamond01` for the original Y-coordinate bug

## Edge Cases
- **LineShape with no waypoints** - Already handled (returns early if < 2)
- **LineShape with no attachments** - `apply_auto_chop_simple_line` handles this (returns unchanged)
- **Polylines** - Existing logic in svg.rs handles this, needs porting
- **Closed paths** - No arrowheads, already handled

## Benefits
- Auto-chop functionality restored for Line/Arrow
- Single source of truth for shape rendering
- ~500 lines of duplicate code eliminated
- Future shapes can access RenderedObject fields if needed
- Consistent API across all shapes
