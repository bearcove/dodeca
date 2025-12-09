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

## Implementation Plan

### Step 1: Add Vector Type to `src/types.rs`

```rust
/// A 2D vector (displacement), distinct from a Point (position)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec2 {
    pub dx: Length,
    pub dy: Length,
}

impl Vec2 {
    pub const ZERO: Self = Self { dx: Length::ZERO, dy: Length::ZERO };

    pub fn new(dx: Length, dy: Length) -> Self {
        Self { dx, dy }
    }

    /// Scale vector by a scalar
    pub fn scale(self, s: f64) -> Self {
        Self {
            dx: self.dx * s,
            dy: self.dy * s,
        }
    }
}

impl std::ops::Mul<Length> for Vec2 {
    type Output = Vec2;
    fn mul(self, rhs: Length) -> Vec2 {
        Vec2 {
            dx: Length(self.dx.0 * rhs.0),
            dy: Length(self.dy.0 * rhs.0),
        }
    }
}

impl std::ops::Add<Vec2> for Point<Length> {
    type Output = Point<Length>;
    fn add(self, rhs: Vec2) -> Point<Length> {
        Point {
            x: self.x + rhs.dx,
            y: self.y + rhs.dy,
        }
    }
}
```

### Step 2: Add `to_unit_vector()` to Direction (in `src/ast.rs` or `src/types.rs`)

```rust
impl Direction {
    /// Convert direction to a unit vector in SVG coordinate space.
    /// SVG Y increases downward, so:
    /// - Up = (0, -1)
    /// - Down = (0, +1)
    /// - Right = (+1, 0)
    /// - Left = (-1, 0)
    pub fn to_unit_vector(self) -> Vec2 {
        match self {
            Direction::Right => Vec2::new(Length::inches(1.0), Length::ZERO),
            Direction::Left => Vec2::new(Length::inches(-1.0), Length::ZERO),
            Direction::Up => Vec2::new(Length::ZERO, Length::inches(-1.0)),
            Direction::Down => Vec2::new(Length::ZERO, Length::inches(1.0)),
        }
    }

    /// Get offset for moving `distance` in this direction
    pub fn offset(self, distance: Length) -> Vec2 {
        let unit = self.to_unit_vector();
        Vec2 {
            dx: Length(unit.dx.0 * distance.0),
            dy: Length(unit.dy.0 * distance.0),
        }
    }
}
```

### Step 3: Refactor `RenderContext::advance()` in `src/render.rs`

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

### Step 4: Refactor `move_in_direction()` in `src/render.rs`

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

### Step 5: Refactor all other scattered `match dir` blocks

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

### Step 6: Consider using `Vec2` for accumulated offsets

Instead of:
```rust
let mut direction_offset_x: Inches = Inches::ZERO;
let mut direction_offset_y: Inches = Inches::ZERO;
```

Use:
```rust
let mut direction_offset = Vec2::ZERO;
// ...
direction_offset = direction_offset + dir.offset(distance);
```

### Step 7: Add unit tests for Direction::offset()

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
