# Plan: Flip Y at Render Time + Use Geometric Types

## Current State

- C pikchr stores coordinates internally with Y-up (mathematical convention)
- C pikchr flips Y in `pik_append_xy`: `y = bbox.ne.y - y`
- Rust stores coordinates the same way as C (Y-up values) but never flips
- Code uses separate `x, y` variables everywhere instead of proper geometric types

## Proposed Changes

### 1. Geometric Types Strategy

Use `glam::DVec2` for all pixel-space coordinates in rendering:
- **Point operations**: positions, centers, endpoints
- **Vector operations**: deltas, offsets, perpendiculars

### 2. Y-Flip as Method on Point

Add method to `Point<Length>` in `src/types.rs`:

```rust
impl Point<Length> {
    /// Convert from pikchr coordinates (Y-up) to SVG pixels (Y-down)
    pub fn to_svg(&self, scaler: &Scaler, offset_x: Length, max_y: Length) -> DVec2 {
        DVec2::new(
            scaler.px(self.x + offset_x),
            scaler.px(max_y - self.y),  // Y-flip like C pikchr
        )
    }
}
```

Usage becomes clean:

```rust
let center = obj.center.to_svg(&scaler, offset_x, max_y);
let start = obj.start.to_svg(&scaler, offset_x, max_y);
let end = obj.end.to_svg(&scaler, offset_x, max_y);
```

### 3. Files to Modify

**`src/types.rs`** - Add method:
- Add `to_svg()` method on `Point<Length>`
- Add `use glam::DVec2` import

**`src/render/svg.rs`** - Major refactor:
- Replace `offset_y` with `max_y`
- Replace all `(tx, ty)`, `(sx, sy)`, `(ex, ey)` pairs with `DVec2`
- Use `point.to_svg(&scaler, offset_x, max_y)` everywhere
- Update all shape rendering to use `DVec2`
- Update arrowhead rendering to take `DVec2`

**`src/render/geometry.rs`**:
- `arc_control_point` already uses `DVec2` ✓
- `create_arc_path` already uses `DVec2` ✓
- Revert `arc_control_point` to original C formula (Y-flip happens in `to_svg`)
- Update other path functions to use `DVec2`
- Update chop/arrowhead functions to use `DVec2`

## Implementation Order

- [ ] 1. Add `to_svg()` method to `Point<Length>` in `src/types.rs`
- [ ] 2. Update `svg.rs`: replace `offset_y` with `max_y`
- [ ] 3. Convert main render loop variables (`tx/ty`, `sx/sy`, `ex/ey`) to `DVec2` via `to_svg()`
- [ ] 4. Update each `ObjectClass` rendering branch to use `DVec2`
- [ ] 5. Update geometry functions to use `DVec2`:
  - [ ] `create_rounded_box_path`
  - [ ] `create_oval_path`
  - [ ] `create_cylinder_paths_with_rad`
  - [ ] `create_file_paths`
  - [ ] `chop_line`
  - [ ] `render_arrowhead_dom`
- [ ] 6. Revert `arc_control_point` to original C formula
- [ ] 7. Run tests to verify

## Expected Outcome

- Arc direction matches C pikchr
- Absolute coordinates (like `at 0,0.5`) render correctly
- All pixel-space code uses `DVec2` for clarity
- Internal pikchr coordinates remain Y-up, flip happens only at SVG conversion
