//! Path builder for line-like objects.
//!
//! This module implements a state machine that mirrors C pikchr's path building
//! approach using `aTPath[]`, `mTPath`, and `thenFlag`.
//!
//! # Key Concepts
//!
//! - **mTPath flags**: Track which coordinates have been set on the current path point
//!   - Bit 0 (value 1): X coordinate has been set
//!   - Bit 1 (value 2): Y coordinate has been set
//!   - Value 3: Both coordinates set (triggers new point on next move)
//!
//! - **thenFlag**: When set, the next movement creates a new path point
//!
//! - **add_direction vs set_even_with**:
//!   - `add_direction()` uses `+=` (relative offset)
//!   - `set_even_with()` uses `=` (absolute coordinate from target)
//!
//! # C pikchr References
//!
//! - `pik_add_direction()`: pikchr.y:3272
//! - `pik_evenwith()`: pikchr.y:3374
//! - `pik_move_hdg()`: pikchr.y:3323
//! - `pik_then()`: pikchr.y:3240
//! - `pik_next_rpath()`: pikchr.y:3256
//! - `mTPath` flags: pikchr.y:399
//! - `thenFlag`: pikchr.y:390

use crate::ast::Direction;
use crate::types::Length as Inches;

use super::types::PointIn;

/// Flags tracking which coordinates have been set on the current path point.
/// Mirrors C pikchr's `mTPath` variable.
///
/// cref: pikchr.y:399 - int mTPath; /* For last entry: 1=x set, 2=y set */
#[derive(Debug, Clone, Copy, Default)]
struct CoordFlags {
    value: u8,
}

impl CoordFlags {
    const X_SET: u8 = 1;
    const Y_SET: u8 = 2;
    const BOTH_SET: u8 = 3;

    fn new() -> Self {
        Self { value: 0 }
    }

    fn x_is_set(self) -> bool {
        self.value & Self::X_SET != 0
    }

    fn y_is_set(self) -> bool {
        self.value & Self::Y_SET != 0
    }

    fn both_set(self) -> bool {
        self.value == Self::BOTH_SET
    }

    fn mark_x_set(&mut self) {
        self.value |= Self::X_SET;
    }

    fn mark_y_set(&mut self) {
        self.value |= Self::Y_SET;
    }

    fn reset(&mut self) {
        self.value = 0;
    }

    fn set_both(&mut self) {
        self.value = Self::BOTH_SET;
    }
}

/// Builder for constructing paths for line-like objects.
///
/// This mirrors C pikchr's approach of building paths incrementally using
/// `aTPath[]`, `mTPath`, and `thenFlag`.
///
/// # Example
///
/// ```ignore
/// let mut builder = PathBuilder::new(start_point);
///
/// // "right 2in up 1in" - accumulates on same point
/// builder.add_direction(Direction::Right, Inches::inches(2.0));
/// builder.add_direction(Direction::Up, Inches::inches(1.0));
///
/// // "then left 1in" - creates new point
/// builder.mark_then();
/// builder.add_direction(Direction::Left, Inches::inches(1.0));
///
/// let path = builder.build();
/// // path = [start, (start.x + 2, start.y + 1), (start.x + 1, start.y + 1)]
/// ```
#[derive(Debug)]
pub struct PathBuilder {
    /// Path points under construction (equivalent to C's aTPath[])
    /// cref: pikchr.y:400 - PPoint aTPath[1000];
    points: Vec<PointIn>,

    /// Flags for current point (equivalent to C's mTPath)
    /// cref: pikchr.y:399 - int mTPath;
    coord_flags: CoordFlags,

    /// True if "then" was seen (equivalent to C's thenFlag)
    /// cref: pikchr.y:390 - char thenFlag;
    then_flag: bool,

    /// Current direction (for tracking exit direction)
    /// cref: pikchr.y:393 - unsigned char eDir;
    current_direction: Direction,
}

impl PathBuilder {
    /// Create a new path builder starting at the given point.
    ///
    /// cref: pikchr.y:5641 - Initial aTPath\[0\] setup
    pub fn new(start: PointIn) -> Self {
        Self {
            points: vec![start],
            coord_flags: CoordFlags::new(),
            then_flag: false,
            current_direction: Direction::Right,
        }
    }

    /// Mark that a "then" keyword was seen. The next movement will create a new path point.
    ///
    /// cref: pikchr.y:3240 - pik_then()
    pub fn mark_then(&mut self) {
        self.then_flag = true;
    }

    /// Get the current (last) point in the path.
    fn current_point(&self) -> PointIn {
        *self.points.last().expect("Path should never be empty")
    }

    /// Get mutable reference to the current (last) point.
    fn current_point_mut(&mut self) -> &mut PointIn {
        self.points.last_mut().expect("Path should never be empty")
    }

    /// Create a new path point by copying the current point.
    /// Equivalent to C's pik_next_rpath().
    ///
    /// cref: pikchr.y:3256-3267 - pik_next_rpath()
    fn push_new_point(&mut self) {
        let current = self.current_point();
        self.points.push(current);
        self.coord_flags.reset();
    }

    /// Check if we need to create a new point before modifying coordinates.
    /// This implements the logic from C's movement functions.
    ///
    /// cref: pikchr.y:3287-3290 in pik_add_direction()
    fn maybe_create_new_point(&mut self) {
        if self.then_flag || self.coord_flags.both_set() || self.points.len() <= 1 {
            // For the initial setup case (points.len() == 1), we only create a new point
            // if there's actual movement happening. The C code checks n==0 which is
            // the index, not the count. Since we start with 1 point, index 0 exists.
            // The C behavior: if n==0 and we're adding direction, create point at n=1.
            if self.then_flag || self.coord_flags.both_set() {
                self.push_new_point();
            }
            self.then_flag = false;
        }
    }

    /// Add a direction-based movement (relative offset).
    /// Equivalent to C's pik_add_direction().
    ///
    /// This function:
    /// - Uses `+=` to accumulate the offset
    /// - Checks mTPath flags to avoid overwriting the same coordinate
    /// - Creates a new point if thenFlag is set or both coordinates are already set
    ///
    /// cref: pikchr.y:3272-3315 - pik_add_direction()
    pub fn add_direction(&mut self, dir: Direction, distance: Inches) {
        self.maybe_create_new_point();

        match dir {
            Direction::Up => {
                // cref: pikchr.y:3294 - if( p->mTPath & 2 ) n = pik_next_rpath(p, pDir);
                if self.coord_flags.y_is_set() {
                    self.push_new_point();
                }
                // cref: pikchr.y:3295 - p->aTPath[n].y += ...
                self.current_point_mut().y = self.current_point().y + distance;
                self.coord_flags.mark_y_set();
            }
            Direction::Down => {
                // cref: pikchr.y:3299
                if self.coord_flags.y_is_set() {
                    self.push_new_point();
                }
                // cref: pikchr.y:3300 - p->aTPath[n].y -= ...
                self.current_point_mut().y = self.current_point().y - distance;
                self.coord_flags.mark_y_set();
            }
            Direction::Right => {
                // cref: pikchr.y:3304 - if( p->mTPath & 1 ) n = pik_next_rpath(p, pDir);
                if self.coord_flags.x_is_set() {
                    self.push_new_point();
                }
                // cref: pikchr.y:3305 - p->aTPath[n].x += ...
                self.current_point_mut().x = self.current_point().x + distance;
                self.coord_flags.mark_x_set();
            }
            Direction::Left => {
                // cref: pikchr.y:3309
                if self.coord_flags.x_is_set() {
                    self.push_new_point();
                }
                // cref: pikchr.y:3310 - p->aTPath[n].x -= ...
                self.current_point_mut().x = self.current_point().x - distance;
                self.coord_flags.mark_x_set();
            }
        }

        // cref: pikchr.y:3314 - pObj->outDir = dir;
        self.current_direction = dir;
    }

    /// Set coordinate to match a target position (absolute positioning).
    /// Equivalent to C's pik_evenwith().
    ///
    /// This function:
    /// - Uses `=` to SET the coordinate (not add)
    /// - Does NOT apply any linewid offset
    /// - Uses the same mTPath logic as add_direction
    ///
    /// cref: pikchr.y:3374-3402 - pik_evenwith()
    pub fn set_even_with(&mut self, dir: Direction, target: PointIn) {
        self.maybe_create_new_point();

        match dir {
            Direction::Up | Direction::Down => {
                // cref: pikchr.y:3390 - if( p->mTPath & 2 ) n = pik_next_rpath(p, pDir);
                if self.coord_flags.y_is_set() {
                    self.push_new_point();
                }
                // cref: pikchr.y:3391 - p->aTPath[n].y = pPlace->y; (SET, not add!)
                self.current_point_mut().y = target.y;
                self.coord_flags.mark_y_set();
            }
            Direction::Right | Direction::Left => {
                // cref: pikchr.y:3396 - if( p->mTPath & 1 ) n = pik_next_rpath(p, pDir);
                if self.coord_flags.x_is_set() {
                    self.push_new_point();
                }
                // cref: pikchr.y:3397 - p->aTPath[n].x = pPlace->x; (SET, not add!)
                self.current_point_mut().x = target.x;
                self.coord_flags.mark_x_set();
            }
        }

        // cref: pikchr.y:3401 - pObj->outDir = pDir->eCode;
        self.current_direction = dir;
    }

    /// Set the endpoint of the path (absolute positioning).
    /// Equivalent to C's pik_add_to().
    ///
    /// cref: pikchr.y:3464-3481 - pik_add_to()
    pub fn set_endpoint(&mut self, point: PointIn) {
        // cref: pikchr.y:3471-3474
        if self.points.len() <= 1 || self.coord_flags.both_set() || self.then_flag {
            self.push_new_point();
        }
        *self.current_point_mut() = point;
        self.coord_flags.set_both();
        self.then_flag = false;
    }

    /// Add a heading-based movement (arbitrary angle).
    /// Equivalent to C's pik_move_hdg().
    ///
    /// The angle is in degrees, measured clockwise from north (up).
    /// - 0° = up
    /// - 90° = right
    /// - 180° = down
    /// - 270° = left
    ///
    /// cref: pikchr.y:3323-3365 - pik_move_hdg()
    pub fn add_heading(&mut self, angle_degrees: f64, distance: Inches) {
        // cref: pikchr.y:3339-3341 - Heading ALWAYS creates new point
        self.push_new_point();
        self.then_flag = false;

        // cref: pikchr.y:3361 - Convert to radians
        let angle_rad = angle_degrees.to_radians();

        // cref: pikchr.y:3362-3363 - Apply offset using sin/cos
        // Note: C uses sin for x and cos for y because heading 0 = north (up)
        let dx = distance.raw() * angle_rad.sin();
        let dy = distance.raw() * angle_rad.cos();

        let pt = self.current_point_mut();
        pt.x += Inches::inches(dx);
        pt.y += Inches::inches(dy);

        // cref: pikchr.y:3350-3360 - Determine cardinal direction from angle
        let normalized = angle_degrees.rem_euclid(360.0);
        self.current_direction = if normalized <= 45.0 || normalized > 315.0 {
            Direction::Up
        } else if normalized <= 135.0 {
            Direction::Right
        } else if normalized <= 225.0 {
            Direction::Down
        } else {
            Direction::Left
        };

        // cref: pikchr.y:3364 - p->mTPath = 2; (treat as Y movement)
        self.coord_flags.mark_y_set();
    }

    /// Get the current exit direction.
    pub fn direction(&self) -> Direction {
        self.current_direction
    }

    /// Set the exit direction without adding a movement.
    pub fn set_direction(&mut self, dir: Direction) {
        self.current_direction = dir;
    }

    /// Build and return the final path.
    pub fn build(self) -> Vec<PointIn> {
        self.points
    }

    /// Get the number of points currently in the path.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Check if the path is empty (should never be true after construction).
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Get the start point of the path.
    pub fn start(&self) -> PointIn {
        self.points[0]
    }

    /// Get the end point of the path.
    pub fn end(&self) -> PointIn {
        self.current_point()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Point;

    fn pt(x: f64, y: f64) -> PointIn {
        Point::new(Inches::inches(x), Inches::inches(y))
    }

    fn assert_point_eq(actual: PointIn, expected: PointIn) {
        const EPSILON: f64 = 1e-10;
        assert!(
            (actual.x.raw() - expected.x.raw()).abs() < EPSILON,
            "x mismatch: {} != {}",
            actual.x.raw(),
            expected.x.raw()
        );
        assert!(
            (actual.y.raw() - expected.y.raw()).abs() < EPSILON,
            "y mismatch: {} != {}",
            actual.y.raw(),
            expected.y.raw()
        );
    }

    #[test]
    fn test_simple_direction_right() {
        // "line right 2"
        let mut builder = PathBuilder::new(pt(0.0, 0.0));
        builder.add_direction(Direction::Right, Inches::inches(2.0));

        let path = builder.build();
        assert_eq!(path.len(), 1); // Single point with accumulated offset
        assert_point_eq(path[0], pt(2.0, 0.0));
    }

    #[test]
    fn test_direction_up_then_right_same_point() {
        // "line up 1 right 2" - accumulates on same point (no "then")
        let mut builder = PathBuilder::new(pt(0.0, 0.0));
        builder.add_direction(Direction::Up, Inches::inches(1.0));
        builder.add_direction(Direction::Right, Inches::inches(2.0));

        let path = builder.build();
        assert_eq!(path.len(), 1); // Still one point
        assert_point_eq(path[0], pt(2.0, 1.0));
    }

    #[test]
    fn test_direction_up_then_keyword_right_new_point() {
        // "line up 1 then right 2" - creates new point
        let mut builder = PathBuilder::new(pt(0.0, 0.0));
        builder.add_direction(Direction::Up, Inches::inches(1.0));
        builder.mark_then();
        builder.add_direction(Direction::Right, Inches::inches(2.0));

        let path = builder.build();
        assert_eq!(path.len(), 2);
        assert_point_eq(path[0], pt(0.0, 1.0));
        assert_point_eq(path[1], pt(2.0, 1.0));
    }

    #[test]
    fn test_even_with_left() {
        // "line left until even with B" where B is at (5, 10)
        let start = pt(10.0, 3.0);
        let target = pt(5.0, 10.0);

        let mut builder = PathBuilder::new(start);
        builder.set_even_with(Direction::Left, target);

        let path = builder.build();
        assert_eq!(path.len(), 1);
        // Should set X to target.x, keep Y unchanged
        assert_point_eq(path[0], pt(5.0, 3.0));
    }

    #[test]
    fn test_even_with_up() {
        // "line up until even with B" where B is at (5, 10)
        let start = pt(3.0, 2.0);
        let target = pt(5.0, 10.0);

        let mut builder = PathBuilder::new(start);
        builder.set_even_with(Direction::Up, target);

        let path = builder.build();
        assert_eq!(path.len(), 1);
        // Should set Y to target.y, keep X unchanged
        assert_point_eq(path[0], pt(3.0, 10.0));
    }

    #[test]
    fn test_even_with_then_direction() {
        // "line left until even with B then up 1"
        // This is the critical case that was broken!
        let start = pt(10.0, 3.0);
        let target = pt(5.0, 10.0);

        let mut builder = PathBuilder::new(start);
        builder.set_even_with(Direction::Left, target);
        builder.mark_then();
        builder.add_direction(Direction::Up, Inches::inches(1.0));

        let path = builder.build();
        assert_eq!(path.len(), 2);
        // First point: X aligned with target
        assert_point_eq(path[0], pt(5.0, 3.0));
        // Second point: moved up by 1
        assert_point_eq(path[1], pt(5.0, 4.0));
    }

    #[test]
    fn test_direction_then_even_with() {
        // "line right 2 then up until even with B"
        let start = pt(0.0, 0.0);
        let target = pt(10.0, 5.0);

        let mut builder = PathBuilder::new(start);
        builder.add_direction(Direction::Right, Inches::inches(2.0));
        builder.mark_then();
        builder.set_even_with(Direction::Up, target);

        let path = builder.build();
        assert_eq!(path.len(), 2);
        // First point: moved right
        assert_point_eq(path[0], pt(2.0, 0.0));
        // Second point: Y aligned with target, X unchanged
        assert_point_eq(path[1], pt(2.0, 5.0));
    }

    #[test]
    fn test_same_axis_movements_create_new_point() {
        // "line right 2 right 3" - same axis, should create new point
        // cref: pikchr.y:3304 - if( p->mTPath & 1 ) n = pik_next_rpath(p, pDir);
        let mut builder = PathBuilder::new(pt(0.0, 0.0));
        builder.add_direction(Direction::Right, Inches::inches(2.0));
        builder.add_direction(Direction::Right, Inches::inches(3.0));

        let path = builder.build();
        assert_eq!(path.len(), 2);
        assert_point_eq(path[0], pt(2.0, 0.0));
        assert_point_eq(path[1], pt(5.0, 0.0));
    }

    #[test]
    fn test_set_endpoint() {
        // "line to (5, 5)"
        let mut builder = PathBuilder::new(pt(0.0, 0.0));
        builder.set_endpoint(pt(5.0, 5.0));

        let path = builder.build();
        assert_eq!(path.len(), 2);
        assert_point_eq(path[0], pt(0.0, 0.0));
        assert_point_eq(path[1], pt(5.0, 5.0));
    }

    #[test]
    fn test_heading_movement() {
        // "line heading 90 1in" (90° = east/right)
        let mut builder = PathBuilder::new(pt(0.0, 0.0));
        builder.add_heading(90.0, Inches::inches(1.0));

        let path = builder.build();
        assert_eq!(path.len(), 2);
        assert_point_eq(path[0], pt(0.0, 0.0));
        // Heading 90° = right, so x increases by ~1
        assert!((path[1].x.raw() - 1.0).abs() < 1e-10);
        assert!(path[1].y.raw().abs() < 1e-10);
    }

    #[test]
    fn test_complex_path() {
        // "line from (0,0) right 2 up 1 then left until even with (1,5) then down 0.5"
        let mut builder = PathBuilder::new(pt(0.0, 0.0));

        // right 2 up 1 (accumulates on first waypoint)
        builder.add_direction(Direction::Right, Inches::inches(2.0));
        builder.add_direction(Direction::Up, Inches::inches(1.0));

        // then left until even with (1, 5)
        builder.mark_then();
        builder.set_even_with(Direction::Left, pt(1.0, 5.0));

        // then down 0.5
        builder.mark_then();
        builder.add_direction(Direction::Down, Inches::inches(0.5));

        let path = builder.build();
        assert_eq!(path.len(), 3);
        assert_point_eq(path[0], pt(2.0, 1.0)); // right 2 up 1
        assert_point_eq(path[1], pt(1.0, 1.0)); // left until even with x=1
        assert_point_eq(path[2], pt(1.0, 0.5)); // down 0.5
    }
}
