//! Shape types for pikchr rendering
//!
//! Each shape is its own type that knows how to:
//! - Calculate its bounding box
//! - Find edge points for line connections
//! - Render itself to SVG

use crate::types::{Length as Inches, Point, Scaler};
use facet_svg::{Circle as SvgCircle, Ellipse as SvgEllipse, Path, PathData, SvgNode, SvgStyle};

use super::defaults;
use super::geometry::{
    create_cylinder_paths_with_rad, create_file_paths, create_oval_path,
    create_rounded_box_path, create_spline_path,
};
use super::svg::color_to_rgb;
use super::types::{ObjectStyle, PointIn, PositionedText};

/// Common behavior for all shapes
pub trait Shape {
    /// The center point of the shape
    fn center(&self) -> PointIn;

    /// Width of the shape's bounding box
    fn width(&self) -> Inches;

    /// Height of the shape's bounding box
    fn height(&self) -> Inches;

    /// The style properties (stroke, fill, etc.)
    fn style(&self) -> &ObjectStyle;

    /// Text labels on this shape
    fn text(&self) -> &[PositionedText];

    /// Whether this shape is "round" (affects diagonal edge point calculations)
    fn is_round(&self) -> bool {
        false
    }

    /// Calculate the edge point in a given direction
    /// Default implementation uses bounding box; shapes can override for precise edges
    fn edge_point(&self, direction: EdgeDirection) -> PointIn {
        let center = self.center();
        let hw = self.width() / 2.0;
        let hh = self.height() / 2.0;

        // For round shapes, diagonal points are on the perimeter, not bounding box corners
        let diag = if self.is_round() {
            std::f64::consts::FRAC_1_SQRT_2
        } else {
            1.0
        };

        match direction {
            EdgeDirection::North => Point::new(center.x, center.y - hh),
            EdgeDirection::South => Point::new(center.x, center.y + hh),
            EdgeDirection::East => Point::new(center.x + hw, center.y),
            EdgeDirection::West => Point::new(center.x - hw, center.y),
            EdgeDirection::NorthEast => Point::new(center.x + hw * diag, center.y - hh * diag),
            EdgeDirection::NorthWest => Point::new(center.x - hw * diag, center.y - hh * diag),
            EdgeDirection::SouthEast => Point::new(center.x + hw * diag, center.y + hh * diag),
            EdgeDirection::SouthWest => Point::new(center.x - hw * diag, center.y + hh * diag),
            EdgeDirection::Center => center,
            EdgeDirection::Start => self.start(),
            EdgeDirection::End => self.end(),
        }
    }

    /// Start point (for lines, this is the first waypoint; for shapes, usually center or west edge)
    fn start(&self) -> PointIn {
        self.edge_point(EdgeDirection::West)
    }

    /// End point (for lines, this is the last waypoint; for shapes, usually center or east edge)
    fn end(&self) -> PointIn {
        self.edge_point(EdgeDirection::East)
    }

    /// Render this shape to SVG nodes
    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode>;
}

/// Cardinal and intercardinal directions for edge points
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
    Center,
    Start,
    End,
}

// ============================================================================
// Shape Types
// ============================================================================

/// A circle shape
#[derive(Debug, Clone)]
pub struct CircleShape {
    pub center: PointIn,
    pub radius: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl CircleShape {
    pub fn new(center: PointIn, radius: Inches) -> Self {
        Self {
            center,
            radius,
            style: ObjectStyle::default(),
            text: Vec::new(),
        }
    }

    pub fn with_style(mut self, style: ObjectStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_text(mut self, text: Vec<PositionedText>) -> Self {
        self.text = text;
        self
    }
}

impl Shape for CircleShape {
    fn center(&self) -> PointIn {
        self.center
    }

    fn width(&self) -> Inches {
        self.radius * 2.0
    }

    fn height(&self) -> Inches {
        self.radius * 2.0
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn is_round(&self) -> bool {
        true
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible {
            return nodes;
        }

        let tx = scaler.px(self.center.x + offset_x);
        let ty = scaler.px(self.center.y + offset_y);
        let r = scaler.px(self.radius);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let circle = SvgCircle {
            cx: Some(tx),
            cy: Some(ty),
            r: Some(r),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Circle(circle));

        nodes
    }
}

/// A box (rectangle) shape
#[derive(Debug, Clone)]
pub struct BoxShape {
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub corner_radius: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl BoxShape {
    pub fn new(center: PointIn, width: Inches, height: Inches) -> Self {
        Self {
            center,
            width,
            height,
            corner_radius: Inches::ZERO,
            style: ObjectStyle::default(),
            text: Vec::new(),
        }
    }
}

impl Shape for BoxShape {
    fn center(&self) -> PointIn {
        self.center
    }

    fn width(&self) -> Inches {
        self.width
    }

    fn height(&self) -> Inches {
        self.height
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible {
            return nodes;
        }

        let tx = scaler.px(self.center.x + offset_x);
        let ty = scaler.px(self.center.y + offset_y);
        let x1 = tx - scaler.px(self.width / 2.0);
        let x2 = tx + scaler.px(self.width / 2.0);
        let y1 = ty - scaler.px(self.height / 2.0);
        let y2 = ty + scaler.px(self.height / 2.0);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let path_data = if self.corner_radius > Inches::ZERO {
            let r = scaler.px(self.corner_radius);
            create_rounded_box_path(x1, y1, x2, y2, r)
        } else {
            // Regular box: start bottom-left, go clockwise
            PathData::new()
                .m(x1, y2)
                .l(x2, y2)
                .l(x2, y1)
                .l(x1, y1)
                .z()
        };

        let path = Path {
            d: Some(path_data),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Path(path));

        nodes
    }
}

/// An ellipse shape
#[derive(Debug, Clone)]
pub struct EllipseShape {
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl Shape for EllipseShape {
    fn center(&self) -> PointIn {
        self.center
    }

    fn width(&self) -> Inches {
        self.width
    }

    fn height(&self) -> Inches {
        self.height
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn is_round(&self) -> bool {
        true
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible {
            return nodes;
        }

        let tx = scaler.px(self.center.x + offset_x);
        let ty = scaler.px(self.center.y + offset_y);
        let rx = scaler.px(self.width / 2.0);
        let ry = scaler.px(self.height / 2.0);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let ellipse = SvgEllipse {
            cx: Some(tx),
            cy: Some(ty),
            rx: Some(rx),
            ry: Some(ry),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Ellipse(ellipse));

        nodes
    }
}

/// An oval (pill) shape - box with fully rounded ends
#[derive(Debug, Clone)]
pub struct OvalShape {
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl Shape for OvalShape {
    fn center(&self) -> PointIn {
        self.center
    }

    fn width(&self) -> Inches {
        self.width
    }

    fn height(&self) -> Inches {
        self.height
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn is_round(&self) -> bool {
        true
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible {
            return nodes;
        }

        let tx = scaler.px(self.center.x + offset_x);
        let ty = scaler.px(self.center.y + offset_y);
        let x1 = tx - scaler.px(self.width / 2.0);
        let x2 = tx + scaler.px(self.width / 2.0);
        let y1 = ty - scaler.px(self.height / 2.0);
        let y2 = ty + scaler.px(self.height / 2.0);

        // Oval radius is half the smaller dimension
        let rad = scaler.px(self.width.min(self.height) / 2.0);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let path_data = create_oval_path(x1, y1, x2, y2, rad);
        let path = Path {
            d: Some(path_data),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Path(path));

        nodes
    }
}

/// A diamond shape
#[derive(Debug, Clone)]
pub struct DiamondShape {
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl Shape for DiamondShape {
    fn center(&self) -> PointIn {
        self.center
    }

    fn width(&self) -> Inches {
        self.width
    }

    fn height(&self) -> Inches {
        self.height
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible {
            return nodes;
        }

        let tx = scaler.px(self.center.x + offset_x);
        let ty = scaler.px(self.center.y + offset_y);
        let hw = scaler.px(self.width / 2.0);
        let hh = scaler.px(self.height / 2.0);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        // Diamond: vertices at N, E, S, W
        let path_data = PathData::new()
            .m(tx, ty - hh)      // North
            .l(tx + hw, ty)      // East
            .l(tx, ty + hh)      // South
            .l(tx - hw, ty)      // West
            .z();

        let path = Path {
            d: Some(path_data),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Path(path));

        nodes
    }
}

/// A cylinder shape
#[derive(Debug, Clone)]
pub struct CylinderShape {
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl Shape for CylinderShape {
    fn center(&self) -> PointIn {
        self.center
    }

    fn width(&self) -> Inches {
        self.width
    }

    fn height(&self) -> Inches {
        self.height
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible {
            return nodes;
        }

        let tx = scaler.px(self.center.x + offset_x);
        let ty = scaler.px(self.center.y + offset_y);
        let w = scaler.px(self.width);
        let h = scaler.px(self.height);

        // Cylinder oval radius (from C pikchr)
        let rad = w * 0.1;

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let (body_path, bottom_arc_path) = create_cylinder_paths_with_rad(tx, ty, w, h, rad);

        let body = Path {
            d: Some(body_path),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style.clone(),
        };
        nodes.push(SvgNode::Path(body));

        let bottom_arc = Path {
            d: Some(bottom_arc_path),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Path(bottom_arc));

        nodes
    }
}

/// A file shape (document with folded corner)
#[derive(Debug, Clone)]
pub struct FileShape {
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub fold_radius: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl FileShape {
    pub fn new(center: PointIn, width: Inches, height: Inches) -> Self {
        Self {
            center,
            width,
            height,
            fold_radius: defaults::FILE_RAD,
            style: ObjectStyle::default(),
            text: Vec::new(),
        }
    }
}

impl Shape for FileShape {
    fn center(&self) -> PointIn {
        self.center
    }

    fn width(&self) -> Inches {
        self.width
    }

    fn height(&self) -> Inches {
        self.height
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible {
            return nodes;
        }

        let tx = scaler.px(self.center.x + offset_x);
        let ty = scaler.px(self.center.y + offset_y);
        let w = scaler.px(self.width);
        let h = scaler.px(self.height);
        let rad = scaler.px(self.fold_radius);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let (main_path, fold_path) = create_file_paths(tx, ty, w, h, rad);

        let main = Path {
            d: Some(main_path),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style.clone(),
        };
        nodes.push(SvgNode::Path(main));

        // Fold line uses same style but no fill
        let fold_style = SvgStyle::new()
            .add("fill", "none")
            .add("stroke", &color_to_rgb(&self.style.stroke))
            .add("stroke-width", &format!("{}", scaler.px(self.style.stroke_width)));

        let fold = Path {
            d: Some(fold_path),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: fold_style,
        };
        nodes.push(SvgNode::Path(fold));

        nodes
    }
}

/// A line or arrow shape
#[derive(Debug, Clone)]
pub struct LineShape {
    pub waypoints: Vec<PointIn>,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl LineShape {
    pub fn new(start: PointIn, end: PointIn) -> Self {
        Self {
            waypoints: vec![start, end],
            style: ObjectStyle::default(),
            text: Vec::new(),
        }
    }

    pub fn with_waypoints(waypoints: Vec<PointIn>) -> Self {
        Self {
            waypoints,
            style: ObjectStyle::default(),
            text: Vec::new(),
        }
    }
}

impl Shape for LineShape {
    fn center(&self) -> PointIn {
        if self.waypoints.is_empty() {
            return Point::new(Inches::ZERO, Inches::ZERO);
        }
        // Center is midpoint between start and end
        let start = self.waypoints.first().unwrap();
        let end = self.waypoints.last().unwrap();
        Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0)
    }

    fn width(&self) -> Inches {
        if self.waypoints.len() < 2 {
            return Inches::ZERO;
        }
        let start = self.waypoints.first().unwrap();
        let end = self.waypoints.last().unwrap();
        (end.x - start.x).abs()
    }

    fn height(&self) -> Inches {
        if self.waypoints.len() < 2 {
            return Inches::ZERO;
        }
        let start = self.waypoints.first().unwrap();
        let end = self.waypoints.last().unwrap();
        (end.y - start.y).abs()
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn start(&self) -> PointIn {
        self.waypoints.first().copied().unwrap_or(Point::new(Inches::ZERO, Inches::ZERO))
    }

    fn end(&self) -> PointIn {
        self.waypoints.last().copied().unwrap_or(Point::new(Inches::ZERO, Inches::ZERO))
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible || self.waypoints.len() < 2 {
            return nodes;
        }

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        // Build path from waypoints
        let mut path_data = PathData::new();
        for (i, pt) in self.waypoints.iter().enumerate() {
            let px = scaler.px(pt.x + offset_x);
            let py = scaler.px(pt.y + offset_y);
            if i == 0 {
                path_data = path_data.m(px, py);
            } else {
                path_data = path_data.l(px, py);
            }
        }

        if self.style.close_path {
            path_data = path_data.z();
        }

        let path = Path {
            d: Some(path_data),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Path(path));

        // TODO: Render arrowheads

        nodes
    }
}

/// A spline (curved line) shape
#[derive(Debug, Clone)]
pub struct SplineShape {
    pub waypoints: Vec<PointIn>,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl Shape for SplineShape {
    fn center(&self) -> PointIn {
        if self.waypoints.is_empty() {
            return Point::new(Inches::ZERO, Inches::ZERO);
        }
        let start = self.waypoints.first().unwrap();
        let end = self.waypoints.last().unwrap();
        Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0)
    }

    fn width(&self) -> Inches {
        if self.waypoints.len() < 2 {
            return Inches::ZERO;
        }
        let start = self.waypoints.first().unwrap();
        let end = self.waypoints.last().unwrap();
        (end.x - start.x).abs()
    }

    fn height(&self) -> Inches {
        if self.waypoints.len() < 2 {
            return Inches::ZERO;
        }
        let start = self.waypoints.first().unwrap();
        let end = self.waypoints.last().unwrap();
        (end.y - start.y).abs()
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn start(&self) -> PointIn {
        self.waypoints.first().copied().unwrap_or(Point::new(Inches::ZERO, Inches::ZERO))
    }

    fn end(&self) -> PointIn {
        self.waypoints.last().copied().unwrap_or(Point::new(Inches::ZERO, Inches::ZERO))
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible || self.waypoints.len() < 2 {
            return nodes;
        }

        let svg_style = build_svg_style(&self.style, scaler, dashwid);
        let path_data = create_spline_path(&self.waypoints, offset_x, offset_y);

        let path = Path {
            d: Some(path_data),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Path(path));

        nodes
    }
}

/// A dot shape (small filled circle)
#[derive(Debug, Clone)]
pub struct DotShape {
    pub center: PointIn,
    pub radius: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl DotShape {
    pub fn new(center: PointIn) -> Self {
        Self {
            center,
            radius: Inches(0.025), // Small default radius
            style: ObjectStyle {
                fill: "black".to_string(),
                ..ObjectStyle::default()
            },
            text: Vec::new(),
        }
    }
}

impl Shape for DotShape {
    fn center(&self) -> PointIn {
        self.center
    }

    fn width(&self) -> Inches {
        self.radius * 2.0
    }

    fn height(&self) -> Inches {
        self.radius * 2.0
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn is_round(&self) -> bool {
        true
    }

    fn render_svg(&self, scaler: &Scaler, offset_x: Inches, offset_y: Inches, dashwid: Inches) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.style.invisible {
            return nodes;
        }

        let tx = scaler.px(self.center.x + offset_x);
        let ty = scaler.px(self.center.y + offset_y);
        let r = scaler.px(self.radius);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let circle = SvgCircle {
            cx: Some(tx),
            cy: Some(ty),
            r: Some(r),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Circle(circle));

        nodes
    }
}

/// A standalone text shape
#[derive(Debug, Clone)]
pub struct TextShape {
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl Shape for TextShape {
    fn center(&self) -> PointIn {
        self.center
    }

    fn width(&self) -> Inches {
        self.width
    }

    fn height(&self) -> Inches {
        self.height
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn render_svg(&self, _scaler: &Scaler, _offset_x: Inches, _offset_y: Inches, _dashwid: Inches) -> Vec<SvgNode> {
        // Text rendering is handled separately
        // This shape type exists mainly for bounds calculation
        Vec::new()
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Build an SVG style from an ObjectStyle
fn build_svg_style(style: &ObjectStyle, scaler: &Scaler, dashwid: Inches) -> SvgStyle {
    let mut svg_style = SvgStyle::new();
    svg_style = svg_style.add("fill", &color_to_rgb(&style.fill));
    svg_style = svg_style.add("stroke", &color_to_rgb(&style.stroke));
    svg_style = svg_style.add("stroke-width", &format!("{}", scaler.px(style.stroke_width)));

    if style.dashed {
        let dash = scaler.px(dashwid);
        svg_style = svg_style.add("stroke-dasharray", &format!("{},{}", dash, dash));
    } else if style.dotted {
        let dot = scaler.px(style.stroke_width);
        let gap = scaler.px(dashwid);
        svg_style = svg_style.add("stroke-dasharray", &format!("{},{}", dot, gap));
    }

    svg_style
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Point;

    #[test]
    fn circle_dimensions() {
        let circle = CircleShape::new(Point::new(Inches(1.0), Inches(2.0)), Inches(0.5));
        assert_eq!(circle.width(), Inches(1.0));
        assert_eq!(circle.height(), Inches(1.0));
        assert!(circle.is_round());
    }

    #[test]
    fn circle_edge_points() {
        let circle = CircleShape::new(Point::new(Inches(0.0), Inches(0.0)), Inches(1.0));

        let north = circle.edge_point(EdgeDirection::North);
        assert_eq!(north.x, Inches(0.0));
        assert_eq!(north.y, Inches(-1.0));

        let east = circle.edge_point(EdgeDirection::East);
        assert_eq!(east.x, Inches(1.0));
        assert_eq!(east.y, Inches(0.0));

        // Diagonal should be at 1/sqrt(2) distance, not corner
        let ne = circle.edge_point(EdgeDirection::NorthEast);
        let expected = std::f64::consts::FRAC_1_SQRT_2;
        assert!((ne.x.0 - expected).abs() < 0.001);
        assert!((ne.y.0 - (-expected)).abs() < 0.001);
    }

    #[test]
    fn box_dimensions() {
        let bx = BoxShape::new(Point::new(Inches(0.0), Inches(0.0)), Inches(2.0), Inches(1.0));
        assert_eq!(bx.width(), Inches(2.0));
        assert_eq!(bx.height(), Inches(1.0));
        assert!(!bx.is_round());
    }

    #[test]
    fn line_start_end() {
        let line = LineShape::new(
            Point::new(Inches(0.0), Inches(0.0)),
            Point::new(Inches(1.0), Inches(1.0)),
        );
        assert_eq!(line.start(), Point::new(Inches(0.0), Inches(0.0)));
        assert_eq!(line.end(), Point::new(Inches(1.0), Inches(1.0)));
        assert_eq!(line.center(), Point::new(Inches(0.5), Inches(0.5)));
    }
}
