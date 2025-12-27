//! Path representation for diagram lines and curves.
//!
//! Paths can be straight lines or Bezier curves, with optional styling
//! (dashed, double, squiggle).

// Many methods are provided for library consumers but not used internally
#![allow(dead_code)]

use std::fmt::Write;

/// Scaling factor: pixels per character cell
pub const SCALE: f64 = 8.0;

/// Aspect ratio for Y axis (characters are taller than wide)
pub const ASPECT: f64 = 2.0;

/// Bezier curve constant for smooth circular arcs
/// This is the "magic number" 4*(sqrt(2)-1)/3 for quarter-circle approximation
pub const CURVE: f64 = 0.551915;

/// Diagonal angle computed from aspect ratio
pub fn diagonal_angle() -> f64 {
    (ASPECT).atan().to_degrees()
}

/// 2D point/vector with SVG coordinate formatting
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Create from grid coordinates (applies SCALE and ASPECT)
    pub fn from_grid(x: i32, y: i32) -> Self {
        Self {
            x: (x as f64 + 1.0) * SCALE,
            y: (y as f64 + 1.0) * SCALE * ASPECT,
        }
    }

    /// Create from fractional grid coordinates (applies SCALE and ASPECT)
    pub fn from_grid_frac(x: f64, y: f64) -> Self {
        Self {
            x: (x + 1.0) * SCALE,
            y: (y + 1.0) * SCALE * ASPECT,
        }
    }

    /// Return a new Vec2 offset by dx, dy (in grid units)
    pub fn offset(&self, dx: f64, dy: f64) -> Self {
        Self {
            x: self.x + dx * SCALE,
            y: self.y + dy * SCALE * ASPECT,
        }
    }

    /// Return a new Vec2 offset by raw pixel amounts
    pub fn offset_pixels(&self, dx: f64, dy: f64) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
        }
    }

    /// Format as "x,y" for SVG path data
    pub fn coords(&self) -> String {
        format!("{},{}", format_coord(self.x), format_coord(self.y))
    }

    /// Format as "x,y " with trailing space for SVG path data
    pub fn to_svg(&self) -> String {
        format!("{},{} ", format_coord(self.x), format_coord(self.y))
    }
}

/// Format a coordinate for SVG output, matching the JS behavior:
/// - Use 5 decimal places max
/// - Strip trailing zeros and decimal point
fn format_coord(x: f64) -> String {
    let s = format!("{:.5}", x);
    // Strip trailing zeros and decimal point
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    s.to_string()
}

/// Line style flags
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PathStyle {
    pub dashed: bool,
    pub double: bool,
    pub squiggle: bool,
}

/// A path segment: either a straight line or a Bezier curve
#[derive(Debug, Clone)]
pub struct Path {
    /// Start point
    pub a: Vec2,
    /// End point
    pub b: Vec2,
    /// First control point (for Bezier curves)
    pub c: Option<Vec2>,
    /// Second control point (for Bezier curves)
    pub d: Option<Vec2>,
    /// Style flags
    pub style: PathStyle,
}

impl Path {
    /// Create a straight line path
    pub fn line(a: Vec2, b: Vec2) -> Self {
        Self {
            a,
            b,
            c: None,
            d: None,
            style: PathStyle::default(),
        }
    }

    /// Create a straight line from grid coordinates
    pub fn line_from_grid(x1: i32, y1: i32, x2: i32, y2: i32) -> Self {
        Self::line(Vec2::from_grid(x1, y1), Vec2::from_grid(x2, y2))
    }

    /// Create a cubic Bezier curve
    pub fn curve(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Self {
        Self {
            a,
            b,
            c: Some(c),
            d: Some(d),
            style: PathStyle::default(),
        }
    }

    /// Set the dashed style
    pub fn with_dashed(mut self, dashed: bool) -> Self {
        self.style.dashed = dashed;
        self
    }

    /// Set the double style
    pub fn with_double(mut self, double: bool) -> Self {
        self.style.double = double;
        self
    }

    /// Set the squiggle style
    pub fn with_squiggle(mut self, squiggle: bool) -> Self {
        self.style.squiggle = squiggle;
        self
    }

    /// Returns true if this is a degenerate (zero-length) path
    pub fn is_degenerate(&self) -> bool {
        (self.a.x - self.b.x).abs() < 0.01 && (self.a.y - self.b.y).abs() < 0.01
    }

    /// Returns true if this is a vertical line
    pub fn is_vertical(&self) -> bool {
        self.c.is_none() && (self.a.x - self.b.x).abs() < 0.01
    }

    /// Returns true if this is a horizontal line
    pub fn is_horizontal(&self) -> bool {
        self.c.is_none() && (self.a.y - self.b.y).abs() < 0.01
    }

    /// Returns true if this is a forward diagonal (/)
    pub fn is_diagonal(&self) -> bool {
        if self.c.is_some() {
            return false;
        }
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        // Forward diagonal goes down-left to up-right
        dx > 0.0 && dy < 0.0
    }

    /// Returns true if this is a back diagonal (\)
    pub fn is_back_diagonal(&self) -> bool {
        if self.c.is_some() {
            return false;
        }
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        // Back diagonal goes up-left to down-right
        dx > 0.0 && dy > 0.0
    }

    /// Returns true if this is a curved path
    pub fn is_curved(&self) -> bool {
        self.c.is_some()
    }

    /// Check if path ends at the given grid position
    pub fn ends_at(&self, x: i32, y: i32) -> bool {
        let target = Vec2::from_grid(x, y);
        self.ends_at_point(target)
    }

    fn ends_at_point(&self, target: Vec2) -> bool {
        let eps = SCALE / 2.0;
        ((self.a.x - target.x).abs() < eps && (self.a.y - target.y).abs() < eps)
            || ((self.b.x - target.x).abs() < eps && (self.b.y - target.y).abs() < eps)
    }

    /// Check if a vertical path ends at the top (min Y) at the given position
    /// JS semantics: checks if min(A.y, B.y) === y in grid coordinates
    pub fn up_ends_at(&self, x: i32, y: i32) -> bool {
        self.up_ends_at_frac(x as f64, y as f64)
    }

    /// Check if a vertical path ends at the top (min Y) at the given fractional position
    pub fn up_ends_at_frac(&self, x: f64, y: f64) -> bool {
        if !self.is_vertical() {
            return false;
        }
        let target = Vec2::from_grid_frac(x, y);
        let eps = SCALE / 2.0;
        // Check if path's min Y matches target Y (JS: min(A.y, B.y) === y)
        let min_y = self.a.y.min(self.b.y);
        (self.a.x - target.x).abs() < eps && (min_y - target.y).abs() < eps
    }

    /// Check if a vertical path ends at the bottom (max Y) at the given position
    /// JS semantics: checks if max(A.y, B.y) === y in grid coordinates
    pub fn down_ends_at(&self, x: i32, y: i32) -> bool {
        self.down_ends_at_frac(x as f64, y as f64)
    }

    /// Check if a vertical path ends at the bottom (max Y) at the given fractional position
    pub fn down_ends_at_frac(&self, x: f64, y: f64) -> bool {
        if !self.is_vertical() {
            return false;
        }
        let target = Vec2::from_grid_frac(x, y);
        let eps = SCALE / 2.0;
        // Check if path's max Y matches target Y (JS: max(A.y, B.y) === y)
        let max_y = self.a.y.max(self.b.y);
        (self.a.x - target.x).abs() < eps && (max_y - target.y).abs() < eps
    }

    /// Check if a horizontal path ends at the left of the given position
    pub fn left_ends_at(&self, x: i32, y: i32) -> bool {
        if !self.is_horizontal() {
            return false;
        }
        let target = Vec2::from_grid(x, y);
        let eps = SCALE / 2.0;
        let left_x = target.x - SCALE / 2.0;
        ((self.a.x - left_x).abs() < eps && (self.a.y - target.y).abs() < eps)
            || ((self.b.x - left_x).abs() < eps && (self.b.y - target.y).abs() < eps)
    }

    /// Check if a horizontal path ends at the right of the given position
    pub fn right_ends_at(&self, x: i32, y: i32) -> bool {
        if !self.is_horizontal() {
            return false;
        }
        let target = Vec2::from_grid(x, y);
        let eps = SCALE / 2.0;
        let right_x = target.x + SCALE / 2.0;
        ((self.a.x - right_x).abs() < eps && (self.a.y - target.y).abs() < eps)
            || ((self.b.x - right_x).abs() < eps && (self.b.y - target.y).abs() < eps)
    }

    /// Check if a vertical line passes through the given position
    pub fn vertical_passes_through(&self, x: i32, y: i32) -> bool {
        if !self.is_vertical() {
            return false;
        }
        let target = Vec2::from_grid(x, y);
        let eps = SCALE / 2.0;
        if (self.a.x - target.x).abs() > eps {
            return false;
        }
        let min_y = self.a.y.min(self.b.y);
        let max_y = self.a.y.max(self.b.y);
        target.y >= min_y - eps && target.y <= max_y + eps
    }

    /// Check if a horizontal line passes through the given position
    pub fn horizontal_passes_through(&self, x: i32, y: i32) -> bool {
        if !self.is_horizontal() {
            return false;
        }
        let target = Vec2::from_grid(x, y);
        let eps = SCALE / 2.0;
        if (self.a.y - target.y).abs() > eps {
            return false;
        }
        let min_x = self.a.x.min(self.b.x);
        let max_x = self.a.x.max(self.b.x);
        target.x >= min_x - eps && target.x <= max_x + eps
    }

    /// Check if a forward diagonal ends at the upper position
    pub fn diagonal_up_ends_at(&self, x: i32, y: i32) -> bool {
        if !self.is_diagonal() {
            return false;
        }
        let target = Vec2::from_grid(x, y);
        let eps = SCALE / 2.0;
        // Upper end has higher x and lower y
        let upper = if self.a.y < self.b.y { &self.a } else { &self.b };
        (upper.x - target.x).abs() < eps && (upper.y - target.y).abs() < eps
    }

    /// Check if a forward diagonal ends at the lower position
    pub fn diagonal_down_ends_at(&self, x: i32, y: i32) -> bool {
        if !self.is_diagonal() {
            return false;
        }
        let target = Vec2::from_grid(x, y);
        let eps = SCALE / 2.0;
        // Lower end has lower x and higher y
        let lower = if self.a.y > self.b.y { &self.a } else { &self.b };
        (lower.x - target.x).abs() < eps && (lower.y - target.y).abs() < eps
    }

    /// Check if a back diagonal ends at the upper position
    pub fn back_diagonal_up_ends_at(&self, x: i32, y: i32) -> bool {
        if !self.is_back_diagonal() {
            return false;
        }
        let target = Vec2::from_grid(x, y);
        let eps = SCALE / 2.0;
        // Upper end has lower x and lower y
        let upper = if self.a.y < self.b.y { &self.a } else { &self.b };
        (upper.x - target.x).abs() < eps && (upper.y - target.y).abs() < eps
    }

    /// Check if a back diagonal ends at the lower position
    pub fn back_diagonal_down_ends_at(&self, x: i32, y: i32) -> bool {
        if !self.is_back_diagonal() {
            return false;
        }
        let target = Vec2::from_grid(x, y);
        let eps = SCALE / 2.0;
        // Lower end has higher x and higher y
        let lower = if self.a.y > self.b.y { &self.a } else { &self.b };
        (lower.x - target.x).abs() < eps && (lower.y - target.y).abs() < eps
    }

    /// Generate SVG path data for this path
    /// Returns a Vec because double lines generate two separate path elements
    pub fn to_svg_paths(&self) -> Vec<String> {
        if self.style.squiggle && self.is_horizontal() {
            return vec![self.squiggle_svg()];
        }

        if self.style.double {
            // Draw two parallel lines as separate path elements
            // Compute perpendicular offset matching JS algorithm
            let vx = self.b.x - self.a.x;
            let vy = self.b.y - self.a.y;
            let s = (vx * vx + vy * vy).sqrt();

            // Normalize and scale to get perpendicular offset
            // In JS: vx /= s * SCALE; vy /= s * SCALE / ASPECT
            // Then offsetLine(vy, -vx) applies offset in screen coords
            // The perpendicular is (vy, -vx) in normalized screen coords
            let px = vy / (s * SCALE / ASPECT); // perpendicular x
            let py = -vx / (s * SCALE); // perpendicular y

            // Convert to pixel offset: multiply by SCALE for x, SCALE*ASPECT for y
            let offset_x = px * SCALE;
            let offset_y = py * SCALE * ASPECT;

            vec![
                self.offset_line_svg(offset_x, offset_y),
                self.offset_line_svg(-offset_x, -offset_y),
            ]
        } else {
            vec![self.single_line_svg()]
        }
    }

    fn single_line_svg(&self) -> String {
        if let (Some(c), Some(d)) = (self.c, self.d) {
            // Cubic Bezier curve
            format!(
                "M {} C {} {} {}",
                self.a.coords(),
                c.coords(),
                d.coords(),
                self.b.coords()
            )
        } else {
            // Straight line
            format!("M {} L {}", self.a.coords(), self.b.coords())
        }
    }

    fn offset_line_svg(&self, dx: f64, dy: f64) -> String {
        let a = self.a.offset_pixels(dx, dy);
        let b = self.b.offset_pixels(dx, dy);

        if let (Some(c), Some(d)) = (self.c, self.d) {
            let c = c.offset_pixels(dx, dy);
            let d = d.offset_pixels(dx, dy);
            format!(
                "M {} C {} {} {}",
                a.coords(),
                c.coords(),
                d.coords(),
                b.coords()
            )
        } else {
            format!("M {} L {}", a.coords(), b.coords())
        }
    }

    fn squiggle_svg(&self) -> String {
        // Generate a wavy horizontal line matching JS behavior
        // The JS iterates from A.x to B.x by 1 grid unit, keeping fractional coords
        let y = self.a.y;
        let amplitude = SCALE * ASPECT * 0.2;

        let mut result = format!("M {},{}", format_coord(self.a.x), format_coord(y));

        // Convert to grid coordinates (fractional) for iteration
        // JS: for (let x = x0; x < x1; x++) where x0/x1 are grid coords
        let grid_x0 = self.a.x / SCALE - 1.0;
        let grid_x1 = self.b.x / SCALE - 1.0;

        // Iterate by full grid units (1.0), keeping fractional start
        let step = SCALE / 4.0; // 0.25 grid units in pixels
        let mut x = self.a.x; // Start in pixel coords
        let mut grid_x = grid_x0;

        while grid_x < grid_x1 {
            // First half: up to mid
            let up_x = x + step;
            let up_y = y - amplitude;
            let mid_x = x + step * 2.0;
            let _ = write!(
                result,
                " Q {},{} {},{}",
                format_coord(up_x),
                format_coord(up_y),
                format_coord(mid_x),
                format_coord(y)
            );

            // Second half: down to start
            let down_x = mid_x + step;
            let down_y = y + amplitude;
            let next_x = mid_x + step * 2.0;
            let _ = write!(
                result,
                " Q {},{} {},{}",
                format_coord(down_x),
                format_coord(down_y),
                format_coord(next_x),
                format_coord(y)
            );

            x = next_x;
            grid_x += 1.0;
        }

        // JS outputs a trailing space after the last Q command
        result.push(' ');
        result
    }
}

/// Collection of paths with query methods
#[derive(Debug, Default)]
pub struct PathSet {
    paths: Vec<Path>,
}

impl PathSet {
    pub fn new() -> Self {
        Self { paths: Vec::new() }
    }

    pub fn insert(&mut self, path: Path) {
        // Don't insert degenerate (zero-length) lines
        if !path.is_degenerate() {
            self.paths.push(path);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Path> {
        self.paths.iter()
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Check if any path has its top end at the given position
    pub fn up_ends_at(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.up_ends_at(x, y))
    }

    /// Check if any path has its top end at the given fractional position
    pub fn up_ends_at_frac(&self, x: f64, y: f64) -> bool {
        self.paths.iter().any(|p| p.up_ends_at_frac(x, y))
    }

    /// Check if any path has its bottom end at the given position
    pub fn down_ends_at(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.down_ends_at(x, y))
    }

    /// Check if any path has its bottom end at the given fractional position
    pub fn down_ends_at_frac(&self, x: f64, y: f64) -> bool {
        self.paths.iter().any(|p| p.down_ends_at_frac(x, y))
    }

    /// Check if any path has its left end at the given position
    pub fn left_ends_at(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.left_ends_at(x, y))
    }

    /// Check if any path has its right end at the given position
    pub fn right_ends_at(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.right_ends_at(x, y))
    }

    /// Check if any vertical path passes through the given position
    pub fn vertical_passes_through(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.vertical_passes_through(x, y))
    }

    /// Check if any horizontal path passes through the given position
    pub fn horizontal_passes_through(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.horizontal_passes_through(x, y))
    }

    /// Check if any diagonal path ends at the upper position
    pub fn diagonal_up_ends_at(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.diagonal_up_ends_at(x, y))
    }

    /// Check if any diagonal path ends at the lower position
    pub fn diagonal_down_ends_at(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.diagonal_down_ends_at(x, y))
    }

    /// Check if any back diagonal path ends at the upper position
    pub fn back_diagonal_up_ends_at(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.back_diagonal_up_ends_at(x, y))
    }

    /// Check if any back diagonal path ends at the lower position
    pub fn back_diagonal_down_ends_at(&self, x: i32, y: i32) -> bool {
        self.paths.iter().any(|p| p.back_diagonal_down_ends_at(x, y))
    }

    /// Generate SVG for all paths
    pub fn to_svg(&self) -> String {
        let mut result = String::new();
        for path in &self.paths {
            let dash = if path.style.dashed {
                " stroke-dasharray=\"4,2\""
            } else {
                ""
            };
            // Double lines generate two separate path elements
            for path_data in path.to_svg_paths() {
                let _ = write!(
                    result,
                    "<path d=\"{}\" fill=\"none\" stroke=\"var(--aasvg-stroke)\"{}/>\n",
                    path_data, dash
                );
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vec2_from_grid() {
        let v = Vec2::from_grid(0, 0);
        assert_eq!(v.x, SCALE);
        assert_eq!(v.y, SCALE * ASPECT);

        let v = Vec2::from_grid(1, 1);
        assert_eq!(v.x, 2.0 * SCALE);
        assert_eq!(v.y, 2.0 * SCALE * ASPECT);
    }

    #[test]
    fn test_path_direction() {
        // Vertical line
        let v = Path::line_from_grid(0, 0, 0, 2);
        assert!(v.is_vertical());
        assert!(!v.is_horizontal());

        // Horizontal line
        let h = Path::line_from_grid(0, 0, 2, 0);
        assert!(h.is_horizontal());
        assert!(!h.is_vertical());
    }

    #[test]
    fn test_path_svg() {
        let p = Path::line(Vec2::new(10.0, 20.0), Vec2::new(30.0, 40.0));
        assert_eq!(p.to_svg_paths(), vec!["M 10,20 L 30,40"]);
    }
}
