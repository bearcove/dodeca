//! Decoration rendering for arrows, points, jumps, and fills.

// Many methods are provided for library consumers but not used internally
#![allow(dead_code)]

use crate::chars::{gray_level, tri_angle};
use crate::path::{diagonal_angle, Vec2, ASPECT, SCALE};

/// Type of decoration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DecorationType {
    /// Arrow head (>)
    Arrow,
    /// Closed/filled point (*)
    ClosedPoint,
    /// Open point (o)
    OpenPoint,
    /// Dotted point (◌)
    DottedPoint,
    /// Shaded point (◍)
    ShadedPoint,
    /// XOR point (⊕)
    XorPoint,
    /// Jump curve (bridge over line crossing)
    /// Parameter is the jump character: '(' or ')'
    Jump(char),
    /// Gray fill rectangle
    Gray(u8),
    /// Triangle decoration
    Triangle,
}

/// A single decoration at a position
#[derive(Debug, Clone)]
pub struct Decoration {
    /// Center position
    pub pos: Vec2,
    /// Type of decoration
    pub kind: DecorationType,
    /// Rotation angle in degrees
    pub angle: f64,
    /// For jumps: the curve control points
    pub jump_from: Option<Vec2>,
    pub jump_to: Option<Vec2>,
}

impl Decoration {
    /// Create an arrow decoration at integer grid coordinates
    pub fn arrow(x: i32, y: i32, angle: f64) -> Self {
        Self {
            pos: Vec2::from_grid(x, y),
            kind: DecorationType::Arrow,
            angle,
            jump_from: None,
            jump_to: None,
        }
    }

    /// Create an arrow decoration at fractional grid coordinates
    pub fn arrow_frac(x: f64, y: f64, angle: f64) -> Self {
        Self {
            pos: Vec2::from_grid_frac(x, y),
            kind: DecorationType::Arrow,
            angle,
            jump_from: None,
            jump_to: None,
        }
    }

    /// Create a closed point decoration (*)
    pub fn closed_point(x: i32, y: i32) -> Self {
        Self {
            pos: Vec2::from_grid(x, y),
            kind: DecorationType::ClosedPoint,
            angle: 0.0,
            jump_from: None,
            jump_to: None,
        }
    }

    /// Create an open point decoration (o)
    pub fn open_point(x: i32, y: i32) -> Self {
        Self {
            pos: Vec2::from_grid(x, y),
            kind: DecorationType::OpenPoint,
            angle: 0.0,
            jump_from: None,
            jump_to: None,
        }
    }

    /// Create a dotted point decoration (◌)
    pub fn dotted_point(x: i32, y: i32) -> Self {
        Self {
            pos: Vec2::from_grid(x, y),
            kind: DecorationType::DottedPoint,
            angle: 0.0,
            jump_from: None,
            jump_to: None,
        }
    }

    /// Create a shaded point decoration (◍)
    pub fn shaded_point(x: i32, y: i32) -> Self {
        Self {
            pos: Vec2::from_grid(x, y),
            kind: DecorationType::ShadedPoint,
            angle: 0.0,
            jump_from: None,
            jump_to: None,
        }
    }

    /// Create an XOR point decoration (⊕)
    pub fn xor_point(x: i32, y: i32) -> Self {
        Self {
            pos: Vec2::from_grid(x, y),
            kind: DecorationType::XorPoint,
            angle: 0.0,
            jump_from: None,
            jump_to: None,
        }
    }

    /// Create a jump (bridge) decoration
    /// `c` should be '(' or ')' to indicate which way the curve bends
    pub fn jump(x: i32, y: i32, c: char) -> Self {
        Self {
            pos: Vec2::from_grid(x, y),
            kind: DecorationType::Jump(c),
            angle: 0.0,
            jump_from: None,
            jump_to: None,
        }
    }

    /// Create a gray fill decoration
    pub fn gray(x: i32, y: i32, c: char) -> Self {
        Self {
            pos: Vec2::from_grid(x, y),
            kind: DecorationType::Gray(gray_level(c)),
            angle: 0.0,
            jump_from: None,
            jump_to: None,
        }
    }

    /// Create a triangle decoration
    pub fn triangle(x: i32, y: i32, c: char) -> Self {
        Self {
            pos: Vec2::from_grid(x, y),
            kind: DecorationType::Triangle,
            angle: tri_angle(c),
            jump_from: None,
            jump_to: None,
        }
    }

    /// Generate SVG for this decoration
    pub fn to_svg(&self) -> String {
        match self.kind {
            DecorationType::Arrow => self.arrow_svg(),
            DecorationType::ClosedPoint => self.closed_point_svg(),
            DecorationType::OpenPoint => self.open_point_svg(),
            DecorationType::DottedPoint => self.dotted_point_svg(),
            DecorationType::ShadedPoint => self.shaded_point_svg(),
            DecorationType::XorPoint => self.xor_point_svg(),
            DecorationType::Jump(c) => self.jump_svg(c),
            DecorationType::Gray(level) => self.gray_svg(level),
            DecorationType::Triangle => self.triangle_svg(),
        }
    }

    fn arrow_svg(&self) -> String {
        let cx = self.pos.x;
        let cy = self.pos.y;

        // Arrow head triangle points in absolute coordinates (matching JS)
        // JS: tip at C.x + 1, back at C.x - 0.5, up/down at C.y ± 0.35
        // In pixels: tip offset = 1 * SCALE = 8, back offset = -0.5 * SCALE = -4
        // up/down offset = 0.35 * SCALE * ASPECT = 5.6
        let tip_x = cx + SCALE;
        let tip_y = cy;
        let back_x = cx - 0.5 * SCALE;
        let back_up_y = cy - 0.35 * SCALE * ASPECT;
        let back_dn_y = cy + 0.35 * SCALE * ASPECT;

        format!(
            "<polygon points=\"{},{} {},{} {},{}\" fill=\"var(--aasvg-fill)\" transform=\"rotate({},{},{})\"/>\n",
            tip_x, tip_y,
            back_x, back_up_y,
            back_x, back_dn_y,
            self.angle, cx, cy
        )
    }

    fn closed_point_svg(&self) -> String {
        let r = SCALE - 2.0;
        format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"var(--aasvg-fill)\"/>\n",
            self.pos.x, self.pos.y, r
        )
    }

    fn open_point_svg(&self) -> String {
        let r = SCALE - 2.0;
        format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"var(--aasvg-bg)\" stroke=\"var(--aasvg-stroke)\"/>\n",
            self.pos.x, self.pos.y, r
        )
    }

    fn dotted_point_svg(&self) -> String {
        let r = SCALE - 2.0;
        format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"var(--aasvg-bg)\" stroke=\"var(--aasvg-stroke)\" stroke-dasharray=\"0,1.8\"/>\n",
            self.pos.x, self.pos.y, r
        )
    }

    fn shaded_point_svg(&self) -> String {
        let r = SCALE - 2.0;
        format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#666\" stroke=\"var(--aasvg-stroke)\"/>\n",
            self.pos.x, self.pos.y, r
        )
    }

    fn xor_point_svg(&self) -> String {
        let r = SCALE - 2.0;
        let cx = self.pos.x;
        let cy = self.pos.y;

        format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"var(--aasvg-bg)\" stroke=\"var(--aasvg-stroke)\"/>\n\
             <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"var(--aasvg-stroke)\"/>\n\
             <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"var(--aasvg-stroke)\"/>\n",
            cx, cy, r,
            cx - r, cy, cx + r, cy,  // Horizontal line through center
            cx, cy - r, cx, cy + r   // Vertical line through center
        )
    }

    fn jump_svg(&self, c: char) -> String {
        // JS: var dx = (decoration.type === ')') ? +0.75 : -0.75;
        let dx = if c == ')' { 0.75 } else { -0.75 };

        // JS: C.x, C.y is the grid position (0-indexed)
        // Our pos is already in pixel coords from from_grid(x, y)
        // from_grid(x, y) = ((x+1)*8, (y+1)*16)
        // We need grid coords, so: grid_x = pos.x/8 - 1, grid_y = pos.y/16 - 1
        let grid_x = self.pos.x / SCALE - 1.0;
        let grid_y = self.pos.y / (SCALE * ASPECT) - 1.0;

        // JS: up = Vec2(C.x, C.y - 0.5), dn = Vec2(C.x, C.y + 0.5)
        // Vec2.coords() converts to pixel: (x+1)*SCALE, (y+1)*SCALE*ASPECT
        let up_x = (grid_x + 1.0) * SCALE;
        let up_y = (grid_y - 0.5 + 1.0) * SCALE * ASPECT;
        let dn_x = (grid_x + 1.0) * SCALE;
        let dn_y = (grid_y + 0.5 + 1.0) * SCALE * ASPECT;

        // JS: cup = Vec2(C.x + dx, C.y - 0.5), cdn = Vec2(C.x + dx, C.y + 0.5)
        let cup_x = (grid_x + dx + 1.0) * SCALE;
        let cup_y = (grid_y - 0.5 + 1.0) * SCALE * ASPECT;
        let cdn_x = (grid_x + dx + 1.0) * SCALE;
        let cdn_y = (grid_y + 0.5 + 1.0) * SCALE * ASPECT;

        // JS: 'M ' + dn + 'C ' + cdn + cup + up.coords()
        // Path goes: dn -> cdn, cup -> up
        format!(
            "<path d=\"M {},{} C {},{} {},{} {},{}\" fill=\"none\" stroke=\"var(--aasvg-bg)\" stroke-width=\"3\"/>\n\
             <path d=\"M {},{} C {},{} {},{} {},{}\" fill=\"none\" stroke=\"var(--aasvg-stroke)\"/>\n",
            dn_x, dn_y, cdn_x, cdn_y, cup_x, cup_y, up_x, up_y,
            dn_x, dn_y, cdn_x, cdn_y, cup_x, cup_y, up_x, up_y
        )
    }

    fn gray_svg(&self, level: u8) -> String {
        // Gray fill rectangle
        let x = self.pos.x - SCALE / 2.0;
        let y = self.pos.y - SCALE * ASPECT / 2.0;
        let w = SCALE;
        let h = SCALE * ASPECT;

        format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"rgb({},{},{})\"/>\n",
            x, y, w, h, level, level, level
        )
    }

    fn triangle_svg(&self) -> String {
        let cx = self.pos.x;
        let cy = self.pos.y;
        let s = SCALE / 2.0;
        let h = SCALE * ASPECT / 2.0;

        // Triangle pointing right, then rotated
        format!(
            "<polygon points=\"{},{} {},{} {},{}\" fill=\"var(--aasvg-fill)\" transform=\"translate({},{}) rotate({})\"/>\n",
            s, 0.0,    // Right point
            -s, -h,    // Top-left
            -s, h,     // Bottom-left
            cx, cy,
            self.angle
        )
    }
}

/// Angle for right-pointing arrow
pub const ARROW_RIGHT: f64 = 0.0;
/// Angle for down-pointing arrow
pub const ARROW_DOWN: f64 = 90.0;
/// Angle for left-pointing arrow
pub const ARROW_LEFT: f64 = 180.0;
/// Angle for up-pointing arrow
pub const ARROW_UP: f64 = 270.0;

/// Get arrow angle for diagonal directions
pub fn arrow_angle_diagonal_up() -> f64 {
    360.0 - diagonal_angle()
}

pub fn arrow_angle_diagonal_down() -> f64 {
    diagonal_angle()
}

pub fn arrow_angle_back_diagonal_up() -> f64 {
    180.0 + diagonal_angle()
}

pub fn arrow_angle_back_diagonal_down() -> f64 {
    180.0 - diagonal_angle()
}

/// Collection of decorations
#[derive(Debug, Default)]
pub struct DecorationSet {
    decorations: Vec<Decoration>,
}

impl DecorationSet {
    pub fn new() -> Self {
        Self {
            decorations: Vec::new(),
        }
    }

    pub fn insert(&mut self, decoration: Decoration) {
        self.decorations.push(decoration);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Decoration> {
        self.decorations.iter()
    }

    pub fn len(&self) -> usize {
        self.decorations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.decorations.is_empty()
    }

    /// Generate SVG for all decorations
    pub fn to_svg(&self) -> String {
        let mut result = String::new();
        for decoration in &self.decorations {
            result.push_str(&decoration.to_svg());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arrow_creation() {
        let arrow = Decoration::arrow(0, 0, ARROW_RIGHT);
        assert_eq!(arrow.kind, DecorationType::Arrow);
        assert_eq!(arrow.angle, 0.0);
    }

    #[test]
    fn test_point_creation() {
        let closed = Decoration::closed_point(1, 1);
        assert_eq!(closed.kind, DecorationType::ClosedPoint);

        let open = Decoration::open_point(2, 2);
        assert_eq!(open.kind, DecorationType::OpenPoint);
    }

    #[test]
    fn test_decoration_set() {
        let mut set = DecorationSet::new();
        set.insert(Decoration::arrow(0, 0, 0.0));
        set.insert(Decoration::closed_point(1, 1));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_arrow_svg_output() {
        let arrow = Decoration::arrow(0, 0, ARROW_RIGHT);
        let svg = arrow.to_svg();
        assert!(svg.contains("polygon"));
        assert!(svg.contains("var(--aasvg-fill)"));
    }
}
