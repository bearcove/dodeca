# Plan: Direction/Vector Refactor for Type-Safe Coordinate System

## Problem

The codebase has **8+ places** where `Direction::Up` and `Direction::Down` manually compute Y offsets, and they're inconsistently implemented. Some add for Up, some subtract. This is because:

1. SVG uses Y-increasing-downward coordinates
2. The code manually handles this in scattered `match` statements
3. No type safety - raw `f64` everywhere means the compiler can't catch coordinate system bugs

## Solution

Use Rust's type system to make incorrect code impossible to write:

1. **Direction should produce a unit vector**
2. **Multiply vector by distance to get offset**
3. **Add offset to position**

This way, the Y-flip logic lives in ONE place (the `to_unit_vector()` method), and everywhere else just does vector math.

## Use `glam` crate

Instead of rolling our own vector types, use the `glam` crate which is:
- Battle-tested, widely used in game dev and graphics
- Zero-cost abstractions, SIMD-optimized
- Has `DVec2` for f64 vectors (we need f64 precision for inches)

```toml
# Cargo.toml
[dependencies]
glam = { version = "0.29", features = ["std"] }
```

## Implementation Plan

### Step 1: Add glam dependency

```toml
# Cargo.toml
[dependencies]
glam = "0.29"
```

### Step 2: Create typed wrappers in `src/types.rs`

We can use `glam::DVec2` directly, or wrap it for type safety. The key is having a clear way to convert between our `Length` type and glam's vectors.

```rust
use glam::DVec2;

/// A 2D offset/displacement in inches
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Offset(pub DVec2);

impl Offset {
    pub const ZERO: Self = Self(DVec2::ZERO);

    pub fn new(dx: Length, dy: Length) -> Self {
        Self(DVec2::new(dx.0, dy.0))
    }

    pub fn dx(&self) -> Length { Length(self.0.x) }
    pub fn dy(&self) -> Length { Length(self.0.y) }
}

impl std::ops::Add<Offset> for Point<Length> {
    type Output = Point<Length>;
    fn add(self, rhs: Offset) -> Point<Length> {
        Point {
            x: Length(self.x.0 + rhs.0.x),
            y: Length(self.y.0 + rhs.0.y),
        }
    }
}

impl std::ops::AddAssign<Offset> for Offset {
    fn add_assign(&mut self, rhs: Offset) {
        self.0 += rhs.0;
    }
}
```

### Step 3: Add `offset()` to Direction (in `src/ast.rs` or `src/types.rs`)

```rust
use glam::DVec2;

impl Direction {
    /// Unit vector for this direction in SVG coordinate space.
    /// SVG Y increases downward, so:
    /// - Up = (0, -1)
    /// - Down = (0, +1)
    /// - Right = (+1, 0)
    /// - Left = (-1, 0)
    pub fn unit_vector(self) -> DVec2 {
        match self {
            Direction::Right => DVec2::X,
            Direction::Left => DVec2::NEG_X,
            Direction::Up => DVec2::NEG_Y,  // SVG Y-down!
            Direction::Down => DVec2::Y,
        }
    }

    /// Get offset for moving `distance` in this direction
    pub fn offset(self, distance: Length) -> Offset {
        Offset(self.unit_vector() * distance.0)
    }
}
```

Note: `glam` provides `DVec2::X`, `DVec2::Y`, `DVec2::NEG_X`, `DVec2::NEG_Y` as pre-defined unit vectors.

### Step 4: Refactor `RenderContext::advance()` in `src/render.rs`

Before:
```rust
pub fn advance(&mut self, distance: Inches) {
    match self.direction {
        Direction::Right => self.position.x += distance,
        Direction::Left => self.position.x -= distance,
        Direction::Up => self.position.y -= distance,
        Direction::Down => self.position.y += distance,
    }
}
```

After:
```rust
pub fn advance(&mut self, distance: Inches) {
    let offset = self.direction.offset(distance);
    self.position = self.position + offset;
}
```

### Step 5: Refactor `move_in_direction()` in `src/render.rs`

Before:
```rust
fn move_in_direction(pos: PointIn, dir: Direction, distance: Inches) -> PointIn {
    match dir {
        Direction::Right => Point::new(pos.x + distance, pos.y),
        Direction::Left => Point::new(pos.x - distance, pos.y),
        Direction::Up => Point::new(pos.x, pos.y - distance),
        Direction::Down => Point::new(pos.x, pos.y + distance),
    }
}
```

After:
```rust
fn move_in_direction(pos: PointIn, dir: Direction, distance: Inches) -> PointIn {
    pos + dir.offset(distance)
}
```

### Step 6: Refactor all other scattered `match dir` blocks

Search for all occurrences of patterns like:
- `Direction::Up =>`
- `Direction::Down =>`
- `direction_offset_y`

Replace with vector operations. The key locations from the audit:

1. **Lines 1055-1062**: Direction attribute parsing
   ```rust
   // Before: manual match with += and -=
   // After:
   let offset = dir.offset(distance);
   direction_offset_x += offset.dx;
   direction_offset_y += offset.dy;
   ```

   Or better, use a `Vec2` accumulator instead of separate x/y.

2. **Lines 1082-1086**: BareExpr direction move
   Same pattern as above.

3. **Lines 1526-1527**: Line endpoint calculation
   ```rust
   // Before: manual match
   // After:
   let end = start + ctx.direction.offset(width);
   ```

4. **Lines 1540-1541**: Shape center positioning
   ```rust
   // Before: manual match
   // After:
   let center = ctx.position + ctx.direction.offset(half_h);
   ```

### Step 7: Use `Offset` for accumulated offsets

Instead of:
```rust
let mut direction_offset_x: Inches = Inches::ZERO;
let mut direction_offset_y: Inches = Inches::ZERO;
```

Use:
```rust
let mut direction_offset = Offset::ZERO;
// ...
direction_offset += dir.offset(distance);
```

### Step 8: Add unit tests for Direction::offset()

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direction_offset_svg_coordinates() {
        let d = Length::inches(1.0);

        // Right increases X
        let r = Direction::Right.offset(d);
        assert!(r.dx > Length::ZERO);
        assert_eq!(r.dy, Length::ZERO);

        // Left decreases X
        let l = Direction::Left.offset(d);
        assert!(l.dx < Length::ZERO);
        assert_eq!(l.dy, Length::ZERO);

        // Up decreases Y (SVG Y increases downward)
        let u = Direction::Up.offset(d);
        assert_eq!(u.dx, Length::ZERO);
        assert!(u.dy < Length::ZERO);

        // Down increases Y
        let down = Direction::Down.offset(d);
        assert_eq!(down.dx, Length::ZERO);
        assert!(down.dy > Length::ZERO);
    }
}
```

## Files to Modify

1. `src/types.rs` - Add `Vec2` type with operations
2. `src/ast.rs` or `src/types.rs` - Add `Direction::to_unit_vector()` and `Direction::offset()`
3. `src/render.rs` - Refactor all 8 locations identified in audit

## Benefits

1. **Single source of truth** - Y-flip logic in ONE place
2. **Compile-time safety** - Can't accidentally mix up coordinates
3. **Readable code** - `pos + dir.offset(distance)` is self-documenting
4. **Zero runtime cost** - All inlines to the same math
5. **Future-proof** - If we ever need a different coordinate system, change one method

## Locations to Fix (from audit)

| Location | Lines | Current Pattern | After |
|----------|-------|-----------------|-------|
| advance() | 485-492 | manual match | `pos + dir.offset(d)` |
| direction attr | 1055-1062 | manual match | `offset += dir.offset(d)` |
| BareExpr | 1082-1086 | manual match | `offset += dir.offset(d)` |
| move_in_direction | 1408-1415 | manual match | `pos + dir.offset(d)` |
| line endpoint | 1526-1527 | manual match | `start + dir.offset(w)` |
| shape center | 1540-1541 | manual match | `pos + dir.offset(h)` |
| shape start | 1546-1547 | relative to center | may need adjustment |
| shape end | 1552-1553 | relative to center | may need adjustment |

## Execution Order

1. First, add `Vec2` type and `Direction::offset()` method
2. Add `Point + Vec2` operator
3. Write and verify unit tests
4. Refactor `advance()` first (simplest)
5. Refactor `move_in_direction()`
6. Refactor direction attribute accumulation (may need `Vec2` accumulator)
7. Refactor remaining locations
8. Run full test suite after each change
9. Clean up any dead code

## Estimated Scope

- ~50-100 lines of new code in types.rs
- ~100 lines modified in render.rs
- Should fix test12 and other Y-direction bugs
- Makes future direction-related bugs nearly impossible
