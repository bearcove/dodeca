//! Shape types for pikchr rendering
//!
//! Each shape is its own type that knows how to:
//! - Calculate its bounding box
//! - Find edge points for line connections
//! - Render itself to SVG

use crate::types::{BoxIn, Length as Inches, OffsetIn, Point, Scaler, Size, UnitVec};
use facet_svg::{
    Circle as SvgCircle, Ellipse as SvgEllipse, Path, PathData, SvgNode, SvgStyle, Text as SvgText,
};
use glam::DVec2;

use super::defaults;
use super::{TextVSlot, compute_text_vslots, count_text_above_below, sum_text_heights_above_below};

/// Bounding box type alias
pub type BoundingBox = BoxIn;
use super::geometry::{
    apply_auto_chop_simple_line, arc_control_point, chop_line, create_arc_path,
    create_cylinder_paths_with_rad, create_file_paths, create_oval_path, create_rounded_box_path,
    create_spline_path,
};
use super::svg::{color_to_rgb, render_arrowhead_dom};
use super::types::{ClassName, ObjectStyle, PointIn, PositionedText, RenderedObject};

use enum_dispatch::enum_dispatch;

/// Common behavior for all shapes
#[enum_dispatch]
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
        match direction {
            EdgeDirection::Center => return self.center(),
            EdgeDirection::Start => return self.start(),
            EdgeDirection::End => return self.end(),
            _ => {}
        }

        let center = self.center();
        let hw = self.width() / 2.0;
        let hh = self.height() / 2.0;

        // For round shapes, diagonal points are on the perimeter (scaled by 1/√2),
        // but cardinal points are at full radius
        let is_diagonal = matches!(
            direction,
            EdgeDirection::NorthEast
                | EdgeDirection::NorthWest
                | EdgeDirection::SouthEast
                | EdgeDirection::SouthWest
        );
        let diag = if is_diagonal {
            if self.is_round() {
                1.0
            } else {
                std::f64::consts::SQRT_2
            }
        } else {
            1.0
        };

        // Use UnitVec for direction, scale x by hw and y by hh
        let offset = direction.unit_vec().scale_xy(hw * diag, hh * diag);

        center + offset
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
    fn render_svg(
        &self,
        obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        arrow_len: Inches,
        arrow_wid: Inches,
    ) -> Vec<SvgNode>;

    /// Get waypoints if this is a path-like shape (line, spline)
    /// Returns None for non-path shapes
    fn waypoints(&self) -> Option<&[PointIn]> {
        None
    }

    /// Translate this shape by an offset
    fn translate(&mut self, offset: OffsetIn);

    /// Expand a bounding box to include this shape
    /// cref: pik_bbox_add_elist (pikchr.c:7206)
    /// Default implementation for box-like shapes
    fn expand_bounds(&self, bounds: &mut BoundingBox) {
        let style = self.style();
        let text = self.text();
        let center = self.center();

        let old_min_x = bounds.min.x.0;

        if style.invisible && !text.is_empty() {
            // For invisible objects, only include text bounds
            let charht = defaults::FONT_SIZE;
            let charwid = defaults::CHARWID;
            for t in text {
                let text_w = Inches(t.width_inches(charwid));
                let hh = Inches(t.height(charht)) / 2.0;
                let hw = text_w / 2.0;
                bounds.expand_point(Point::new(center.x - hw, center.y - hh));
                bounds.expand_point(Point::new(center.x + hw, center.y + hh));
            }
        } else if !style.invisible {
            bounds.expand_rect(
                center,
                Size {
                    w: self.width(),
                    h: self.height(),
                },
            );
        }

        tracing::debug!(
            old_min_x,
            new_min_x = bounds.min.x.0,
            center_x = center.x.0,
            width = self.width().0,
            height = self.height().0,
            invisible = style.invisible,
            "[BBOX]"
        );
    }
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

impl EdgeDirection {
    /// Get the unit vector for this direction (Y-up convention)
    pub fn unit_vec(self) -> UnitVec {
        match self {
            EdgeDirection::North => UnitVec::NORTH,
            EdgeDirection::South => UnitVec::SOUTH,
            EdgeDirection::East => UnitVec::EAST,
            EdgeDirection::West => UnitVec::WEST,
            EdgeDirection::NorthEast => UnitVec::NORTH_EAST,
            EdgeDirection::NorthWest => UnitVec::NORTH_WEST,
            EdgeDirection::SouthEast => UnitVec::SOUTH_EAST,
            EdgeDirection::SouthWest => UnitVec::SOUTH_WEST,
            EdgeDirection::Center | EdgeDirection::Start | EdgeDirection::End => UnitVec::ZERO,
        }
    }
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: circleRender (pikchr.c:3961) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(scaler, offset_x, max_y);
        let r = scaler.px(self.radius);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let circle = SvgCircle {
            cx: Some(center_svg.x),
            cy: Some(center_svg.y),
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

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: boxRender (pikchr.c:3856) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(scaler, offset_x, max_y);
        let hw = scaler.px(self.width / 2.0);
        let hh = scaler.px(self.height / 2.0);
        let x1 = center_svg.x - hw;
        let x2 = center_svg.x + hw;
        let y1 = center_svg.y - hh;
        let y2 = center_svg.y + hh;

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let path_data = if self.corner_radius > Inches::ZERO {
            let r = scaler.px(self.corner_radius);
            create_rounded_box_path(x1, y1, x2, y2, r)
        } else {
            // Regular box: start bottom-left, go clockwise
            PathData::new().m(x1, y2).l(x2, y2).l(x2, y1).l(x1, y1).z()
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

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: ovalRender (pikchr.c:3987) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(scaler, offset_x, max_y);
        let rx = scaler.px(self.width / 2.0);
        let ry = scaler.px(self.height / 2.0);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let ellipse = SvgEllipse {
            cx: Some(center_svg.x),
            cy: Some(center_svg.y),
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

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: ovalRender (pikchr.c:3987) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(scaler, offset_x, max_y);
        let hw = scaler.px(self.width / 2.0);
        let hh = scaler.px(self.height / 2.0);
        let x1 = center_svg.x - hw;
        let x2 = center_svg.x + hw;
        let y1 = center_svg.y - hh;
        let y2 = center_svg.y + hh;

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

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: diamondRender (pikchr.c:4058) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(scaler, offset_x, max_y);
        let hw = scaler.px(self.width / 2.0);
        let hh = scaler.px(self.height / 2.0);

        tracing::debug!(
            center_pikchr_x = self.center.x.0,
            center_pikchr_y = self.center.y.0,
            center_svg_x = center_svg.x,
            center_svg_y = center_svg.y,
            offset_x = offset_x.0,
            max_y = max_y.0,
            "DiamondShape render_svg"
        );

        let left = center_svg.x - hw;
        let right = center_svg.x + hw;
        let top = center_svg.y - hh;
        let bottom = center_svg.y + hh;

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        // Diamond: start at west edge to match C pikchr ordering
        let path_data = PathData::new()
            .m(left, center_svg.y) // West
            .l(center_svg.x, bottom) // South
            .l(right, center_svg.y) // East
            .l(center_svg.x, top) // North
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

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
    }
}

/// A cylinder shape
#[derive(Debug, Clone)]
pub struct CylinderShape {
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub ellipse_rad: Inches, // cref: cylrad - minor radius of top/bottom ellipses
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: cylinderRender (pikchr.c:4112) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(scaler, offset_x, max_y);
        let w = scaler.px(self.width);
        let h = scaler.px(self.height);

        // cref: cylinderRender - use stored cylrad, clamped to half height
        let mut rad = scaler.px(self.ellipse_rad);
        let h2 = h / 2.0;
        if rad > h2 {
            rad = h2;
        } else if rad < 0.0 {
            rad = 0.0;
        }

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let (body_path, bottom_arc_path) =
            create_cylinder_paths_with_rad(center_svg.x, center_svg.y, w, h, rad);

        let body = Path {
            d: Some(body_path),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style.clone(),
        };
        nodes.push(SvgNode::Path(body));

        if !bottom_arc_path.commands.is_empty() {
            let bottom_arc = Path {
                d: Some(bottom_arc_path),
                fill: None,
                stroke: None,
                stroke_width: None,
                stroke_dasharray: None,
                style: svg_style,
            };
            nodes.push(SvgNode::Path(bottom_arc));
        }

        nodes
    }

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: fileRender (pikchr.c:4171) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(scaler, offset_x, max_y);
        let w = scaler.px(self.width);
        let h = scaler.px(self.height);
        let rad = scaler.px(self.fold_radius);

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let (main_path, fold_path) = create_file_paths(center_svg.x, center_svg.y, w, h, rad);

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
        let fold_style = svg_style_from_entries(vec![
            ("fill", "none".to_string()),
            ("stroke", color_to_rgb(&self.style.stroke)),
            (
                "stroke-width",
                format!("{}", scaler.px(self.style.stroke_width)),
            ),
        ]);

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

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
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
            return Point::ORIGIN;
        }
        // Center is midpoint between start and end
        let start = *self.waypoints.first().unwrap();
        let end = *self.waypoints.last().unwrap();
        start.midpoint(end)
    }

    fn width(&self) -> Inches {
        if self.waypoints.len() < 2 {
            return Inches::ZERO;
        }
        let start = *self.waypoints.first().unwrap();
        let end = *self.waypoints.last().unwrap();
        let delta = end - start;
        delta.dx.abs()
    }

    fn height(&self) -> Inches {
        if self.waypoints.len() < 2 {
            return Inches::ZERO;
        }
        let start = *self.waypoints.first().unwrap();
        let end = *self.waypoints.last().unwrap();
        let delta = end - start;
        delta.dy.abs()
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn start(&self) -> PointIn {
        self.waypoints.first().copied().unwrap_or(Point::ORIGIN)
    }

    fn end(&self) -> PointIn {
        self.waypoints.last().copied().unwrap_or(Point::ORIGIN)
    }

    fn render_svg(
        &self,
        obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        arrow_len: Inches,
        arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: lineRender (pikchr.c:4228) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 || self.waypoints.len() < 2 {
            return nodes;
        }

        // cref: lineRender (pikchr.c:4253) - add stroke-linejoin:round for closed sharp-cornered paths
        let add_linejoin = self.style.close_path && self.style.corner_radius.raw() == 0.0;
        let svg_style = build_svg_style_ex(&self.style, scaler, dashwid, add_linejoin);

        let arrow_len_px = scaler.px(arrow_len);
        let arrow_wid_px = scaler.px(arrow_wid);
        let arrow_chop = arrow_len_px / 2.0;

        let mut svg_points: Vec<DVec2> = self
            .waypoints
            .iter()
            .map(|pt| pt.to_svg(scaler, offset_x, max_y))
            .collect();

        if svg_points.len() <= 2 {
            let start = svg_points[0];
            let end = svg_points[svg_points.len() - 1];
            let (mut draw_start, mut draw_end) =
                apply_auto_chop_simple_line(scaler, obj, start, end, offset_x, max_y);

            if self.style.arrow_end {
                if let Some(arrowhead) = render_arrowhead_dom(
                    draw_start,
                    draw_end,
                    &self.style,
                    arrow_len_px,
                    arrow_wid_px,
                ) {
                    nodes.push(SvgNode::Polygon(arrowhead));
                }
            }
            if self.style.arrow_start {
                if let Some(arrowhead) = render_arrowhead_dom(
                    draw_end,
                    draw_start,
                    &self.style,
                    arrow_len_px,
                    arrow_wid_px,
                ) {
                    nodes.push(SvgNode::Polygon(arrowhead));
                }
            }

            if self.style.arrow_start {
                let (new_start, _) = chop_line(draw_start, draw_end, arrow_chop);
                draw_start = new_start;
            }
            if self.style.arrow_end {
                let (_, new_end) = chop_line(draw_start, draw_end, arrow_chop);
                draw_end = new_end;
            }

            let mut path_data = PathData::new()
                .m(draw_start.x, draw_start.y)
                .l(draw_end.x, draw_end.y);
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
            return nodes;
        }

        if self.style.chop && svg_points.len() >= 2 {
            let chop_amount = scaler.px(defaults::CIRCLE_RADIUS);
            let (new_start, _) = chop_line(svg_points[0], svg_points[1], chop_amount);
            svg_points[0] = new_start;
            let n = svg_points.len();
            let (_, new_end) = chop_line(svg_points[n - 2], svg_points[n - 1], chop_amount);
            svg_points[n - 1] = new_end;
        }

        if (self.style.arrow_start || self.style.arrow_end) && svg_points.len() >= 2 {
            if self.style.arrow_start {
                let (new_start, _) = chop_line(svg_points[0], svg_points[1], arrow_chop);
                svg_points[0] = new_start;
            }
            if self.style.arrow_end {
                let n = svg_points.len();
                let (_, new_end) = chop_line(svg_points[n - 2], svg_points[n - 1], arrow_chop);
                svg_points[n - 1] = new_end;
            }
            // TODO: Render arrowheads for multi-segment lines
        }

        // cref: lineRender (pikchr.c:4302-4336) - rounded corners with rad attribute
        let corner_radius_px = scaler.px(self.style.corner_radius);
        let mut path_data = PathData::new();

        if corner_radius_px > 0.0 && svg_points.len() >= 3 {
            // cref: radiusPath (pikchr.c:1683-1711) - render rounded corners
            let n = svg_points.len();
            let i_last = if self.style.close_path { n } else { n - 1 };

            tracing::debug!(
                n,
                r = corner_radius_px,
                close = self.style.close_path,
                "[Rust radiusPath]"
            );

            // Start at first waypoint
            path_data = path_data.m(svg_points[0].x, svg_points[0].y);
            tracing::debug!(
                x = svg_points[0].x,
                y = svg_points[0].y,
                "[Rust radiusPath] M"
            );

            // Draw line to midpoint before second waypoint
            // cref: radiusMidpoint(a[0], a[1], r) returns a[1] - r*normalized(a[1]-a[0])
            if n >= 2 {
                let dir = (svg_points[1] - svg_points[0]).normalize();
                let m = svg_points[1] - dir * corner_radius_px;  // Go from wp1 back toward wp0
                path_data = path_data.l(m.x, m.y);
                tracing::debug!(
                    x = m.x,
                    y = m.y,
                    "[Rust radiusPath] L (before wp1)"
                );
            }

            // Loop through waypoints 1 to i_last-1, rounding each corner
            for i in 1..i_last {
                let a_i = svg_points[i];
                // Next waypoint (wraps to first for closed paths)
                let a_n = if i < n - 1 { svg_points[i + 1] } else { svg_points[0] };

                tracing::debug!(
                    i,
                    a_i_x = a_i.x,
                    a_i_y = a_i.y,
                    a_n_x = a_n.x,
                    a_n_y = a_n.y,
                    "[Rust radiusPath] loop start"
                );

                // Entry point: from a_n toward a_i, then back off by radius
                // cref: radiusMidpoint(an, a[i], r) returns a[i] - r*dir
                let dir_in = (a_i - a_n).normalize();
                let m_entry = a_i - dir_in * corner_radius_px;

                // Quadratic curve: control at a[i], end at entry point
                path_data = path_data.q(a_i.x, a_i.y, m_entry.x, m_entry.y);
                tracing::debug!(
                    ctrl_x = a_i.x,
                    ctrl_y = a_i.y,
                    end_x = m_entry.x,
                    end_y = m_entry.y,
                    i,
                    "[Rust radiusPath] Q (curve at wp)"
                );

                // Exit point: point before reaching next waypoint
                // cref: radiusMidpoint(a[i], an, r) returns an - r*dir = point near an
                let dist = (a_n - a_i).length();
                if corner_radius_px < dist * 0.5 {
                    let dir_out = (a_n - a_i).normalize();
                    let m_exit = a_n - dir_out * corner_radius_px;  // near a_n, not a_i!
                    path_data = path_data.l(m_exit.x, m_exit.y);
                    tracing::debug!(
                        x = m_exit.x,
                        y = m_exit.y,
                        toward = i + 1,
                        "[Rust radiusPath] L (toward wp)"
                    );
                }
            }

            // Line back to start (for closed paths)
            let a_n = if i_last == n { svg_points[0] } else { svg_points[n - 1] };
            path_data = path_data.l(a_n.x, a_n.y);
            tracing::debug!(
                x = a_n.x,
                y = a_n.y,
                "[Rust radiusPath] L (final)"
            );
        } else {
            // No rounding - simple polyline
            for (i, pt) in svg_points.iter().enumerate() {
                if i == 0 {
                    path_data = path_data.m(pt.x, pt.y);
                } else {
                    path_data = path_data.l(pt.x, pt.y);
                }
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

        nodes
    }

    fn waypoints(&self) -> Option<&[PointIn]> {
        Some(&self.waypoints)
    }

    fn translate(&mut self, offset: OffsetIn) {
        for pt in self.waypoints.iter_mut() {
            *pt += offset;
        }
    }

    /// cref: pik_bbox_add_elist (pikchr.c:7206) - line bbox from waypoints
    /// cref: pik_bbox_add_elist (pikchr.c:7243) - only if sw>=0
    /// cref: pik_bbox_add_elist (pikchr.c:7251-7260) - arrowheads always added as ellipses
    fn expand_bounds(&self, bounds: &mut BoundingBox) {
        let old_min_x = bounds.min.x.0;

        // Only expand by waypoints if stroke width is non-negative
        if self.style.stroke_width.0 >= 0.0 {
            for pt in &self.waypoints {
                bounds.expand_point(*pt);
            }
        }

        // Arrowheads are ALWAYS included as ellipses, even for negative thickness
        // C: pik_bbox_addellipse(&p->bbox, pObj->aPath[j].x, pObj->aPath[j].y, wArrow, wArrow)
        // where wArrow = 0.5 * arrowwid (line 7274)
        // This happens after the sw check in C (lines 7251-7260)
        if !self.waypoints.is_empty() {
            let w_arrow = defaults::ARROW_WID * 0.5;
            if self.style.arrow_start {
                let pt = self.waypoints[0];
                bounds.expand_point(Point::new(pt.x - w_arrow, pt.y - w_arrow));
                bounds.expand_point(Point::new(pt.x + w_arrow, pt.y + w_arrow));
            }
            if self.style.arrow_end {
                let pt = *self.waypoints.last().unwrap();
                bounds.expand_point(Point::new(pt.x - w_arrow, pt.y - w_arrow));
                bounds.expand_point(Point::new(pt.x + w_arrow, pt.y + w_arrow));
            }
        }

        tracing::debug!(
            old_min_x,
            new_min_x = bounds.min.x.0,
            num_waypoints = self.waypoints.len(),
            sw = self.style.stroke_width.0,
            "[BBOX Line]"
        );
        // Include text labels (they extend above and below the line)
        // Must account for font scaling from big/small modifiers
        if !self.text.is_empty() {
            let charht = defaults::FONT_SIZE;
            let (text_above, text_below) = sum_text_heights_above_below(&self.text, charht);
            let center = self.center();
            bounds.expand_point(Point::new(center.x, center.y + Inches(text_above)));
            bounds.expand_point(Point::new(center.x, center.y - Inches(text_below)));
        }
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
            return Point::ORIGIN;
        }
        let start = *self.waypoints.first().unwrap();
        let end = *self.waypoints.last().unwrap();
        start.midpoint(end)
    }

    fn width(&self) -> Inches {
        if self.waypoints.len() < 2 {
            return Inches::ZERO;
        }
        let start = *self.waypoints.first().unwrap();
        let end = *self.waypoints.last().unwrap();
        let delta = end - start;
        delta.dx.abs()
    }

    fn height(&self) -> Inches {
        if self.waypoints.len() < 2 {
            return Inches::ZERO;
        }
        let start = *self.waypoints.first().unwrap();
        let end = *self.waypoints.last().unwrap();
        let delta = end - start;
        delta.dy.abs()
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn start(&self) -> PointIn {
        self.waypoints.first().copied().unwrap_or(Point::ORIGIN)
    }

    fn end(&self) -> PointIn {
        self.waypoints.last().copied().unwrap_or(Point::ORIGIN)
    }

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        arrow_len: Inches,
        arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: splineRender (pikchr.c:4381) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 || self.waypoints.len() < 2 {
            return nodes;
        }

        let svg_style = build_svg_style(&self.style, scaler, dashwid);
        let path_data = create_spline_path(&self.waypoints, scaler, offset_x, max_y);
        let arrow_len_px = scaler.px(arrow_len);
        let arrow_wid_px = scaler.px(arrow_wid);

        let n = self.waypoints.len();
        if self.style.arrow_end && n >= 2 {
            let p1 = self.waypoints[n - 2].to_svg(scaler, offset_x, max_y);
            let p2 = self.waypoints[n - 1].to_svg(scaler, offset_x, max_y);
            if let Some(arrowhead) =
                render_arrowhead_dom(p1, p2, &self.style, arrow_len_px, arrow_wid_px)
            {
                nodes.push(SvgNode::Polygon(arrowhead));
            }
        }
        if self.style.arrow_start && n >= 2 {
            let p1 = self.waypoints[0].to_svg(scaler, offset_x, max_y);
            let p2 = self.waypoints[1].to_svg(scaler, offset_x, max_y);
            if let Some(arrowhead) =
                render_arrowhead_dom(p2, p1, &self.style, arrow_len_px, arrow_wid_px)
            {
                nodes.push(SvgNode::Polygon(arrowhead));
            }
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

        nodes
    }

    fn waypoints(&self) -> Option<&[PointIn]> {
        Some(&self.waypoints)
    }

    fn translate(&mut self, offset: OffsetIn) {
        for pt in self.waypoints.iter_mut() {
            *pt += offset;
        }
    }

    /// cref: pik_bbox_add_elist (pikchr.c:7206) - spline bbox from waypoints
    /// cref: pik_bbox_add_elist (pikchr.c:7243) - only if sw>=0
    /// cref: pik_bbox_add_elist (pikchr.c:7251-7260) - arrowheads always added as ellipses
    fn expand_bounds(&self, bounds: &mut BoundingBox) {
        // Only expand by waypoints if stroke width is non-negative
        if self.style.stroke_width.0 >= 0.0 {
            for pt in &self.waypoints {
                bounds.expand_point(*pt);
            }
        }

        // Arrowheads are ALWAYS included as ellipses, even for negative thickness
        // C: pik_bbox_addellipse(&p->bbox, pObj->aPath[j].x, pObj->aPath[j].y, wArrow, wArrow)
        // where wArrow = 0.5 * arrowwid (line 7274)
        // This happens after the sw check in C (lines 7251-7260)
        if !self.waypoints.is_empty() {
            let w_arrow = defaults::ARROW_WID * 0.5;
            if self.style.arrow_start {
                let pt = self.waypoints[0];
                bounds.expand_point(Point::new(pt.x - w_arrow, pt.y - w_arrow));
                bounds.expand_point(Point::new(pt.x + w_arrow, pt.y + w_arrow));
            }
            if self.style.arrow_end {
                let pt = *self.waypoints.last().unwrap();
                bounds.expand_point(Point::new(pt.x - w_arrow, pt.y - w_arrow));
                bounds.expand_point(Point::new(pt.x + w_arrow, pt.y + w_arrow));
            }
        }
        // Include text labels (must account for font scaling)
        if !self.text.is_empty() {
            let charht = defaults::FONT_SIZE;
            let (text_above, text_below) = sum_text_heights_above_below(&self.text, charht);
            let center = self.center();
            bounds.expand_point(Point::new(center.x, center.y + Inches(text_above)));
            bounds.expand_point(Point::new(center.x, center.y - Inches(text_below)));
        }
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: dotRender (pikchr.c:3934) - dots are always filled, no sw check needed
        // But for consistency with other shapes, we still check for negative thickness
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(scaler, offset_x, max_y);
        let r = scaler.px(self.radius);

        tracing::debug!(
            fill = %self.style.fill,
            stroke = %self.style.stroke,
            "[Rust dot render] About to render dot"
        );

        let svg_style = build_svg_style(&self.style, scaler, dashwid);

        let circle = SvgCircle {
            cx: Some(center_svg.x),
            cy: Some(center_svg.y),
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

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        _dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        if self.text.is_empty() {
            return nodes;
        }

        let center_svg = self.center.to_svg(scaler, offset_x, max_y);
        let charht_px = scaler.px(Inches(defaults::FONT_SIZE));
        let line_count = self.text.len();
        let start_y = center_svg.y - (line_count as f64 - 1.0) * charht_px / 2.0;

        let text_color = if self.style.stroke == "black" || self.style.stroke == "none" {
            "rgb(0,0,0)".to_string()
        } else {
            color_to_rgb(&self.style.stroke)
        };

        for (i, positioned_text) in self.text.iter().enumerate() {
            let anchor = if positioned_text.rjust {
                "end"
            } else if positioned_text.ljust {
                "start"
            } else {
                "middle"
            };
            let line_y = start_y + i as f64 * charht_px;

            // Font styling based on text attributes
            let font_family = if positioned_text.mono {
                Some("monospace".to_string())
            } else {
                None
            };
            let font_style = if positioned_text.italic {
                Some("italic".to_string())
            } else {
                None
            };
            let font_weight = if positioned_text.bold {
                Some("bold".to_string())
            } else {
                None
            };
            // Use font_scale() to get the correct scale (handles xtra for double big/small)
            // C pikchr uses percentage-based font sizes: 125% for big, 80% for small, squared if xtra
            // cref: pik_append_txt (pikchr.c:5183)
            let font_size = if positioned_text.big || positioned_text.small {
                let scale = positioned_text.font_scale();
                let percent = scale * 100.0;
                // Format with appropriate precision to avoid floating point artifacts
                Some(super::svg::fmt_num(percent) + "%")
            } else {
                None
            };

            let text_element = SvgText {
                x: Some(center_svg.x),
                y: Some(line_y),
                fill: Some(text_color.clone()),
                stroke: None,
                stroke_width: None,
                style: SvgStyle::default(),
                font_family,
                font_style,
                font_weight,
                font_size,
                text_anchor: Some(anchor.to_string()),
                dominant_baseline: Some("central".to_string()),
                content: positioned_text.value.replace(' ', "\u{00A0}"),
            };
            nodes.push(SvgNode::Text(text_element));
        }

        nodes
    }

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
    }

    /// cref: pik_bbox_add_elist (pikchr.c:7214) - adds both object bbox and text bbox
    fn expand_bounds(&self, bounds: &mut BoundingBox) {
        let charht = Inches(defaults::FONT_SIZE);
        let charwid = defaults::CHARWID;
        let center = self.center;

        // First, expand with object dimensions (which include fit padding)
        // cref: pikchr.c:7113-7114 - pik_bbox_add_xy for object's width/height
        bounds.expand_rect(
            center,
            Size {
                w: self.width,
                h: self.height,
            },
        );

        if self.text.is_empty() {
            return;
        }

        // Also expand with text line positions (they may extend beyond object bounds)
        // cref: pik_append_txt (pikchr.c:5077)
        {
            // Compute vertical slot assignments matching C's pik_txt_vertical_layout
            let vslots = compute_text_vslots(&self.text);

            // Compute heights for each region (max charht in each slot)
            // cref: pikchr.c:5104-5143
            let mut hc = Inches::ZERO;
            let mut ha1 = Inches::ZERO;
            let mut ha2 = Inches::ZERO;
            let mut hb1 = Inches::ZERO;
            let mut hb2 = Inches::ZERO;

            // cref: pik_append_txt (pikchr.c:5114-5147) - uses font_scale for each text
            for (t, slot) in self.text.iter().zip(vslots.iter()) {
                let h = Inches(t.height(charht.0));
                match slot {
                    TextVSlot::Center => hc = hc.max(h),
                    TextVSlot::Above => ha1 = ha1.max(h),
                    TextVSlot::Above2 => ha2 = ha2.max(h),
                    TextVSlot::Below => hb1 = hb1.max(h),
                    TextVSlot::Below2 => hb2 = hb2.max(h),
                }
            }

            // Calculate Y position for each text line
            // cref: pikchr.c:5155-5158
            let y_base = Inches::ZERO;
            for (i, t) in self.text.iter().enumerate() {
                let text_w = Inches(t.width_inches(charwid));
                let ch = Inches(t.height(charht.0)) / 2.0;

                let y = match vslots.get(i).unwrap_or(&TextVSlot::Center) {
                    TextVSlot::Above2 => y_base + hc * 0.5 + ha1 + ha2 * 0.5,
                    TextVSlot::Above => y_base + hc * 0.5 + ha1 * 0.5,
                    TextVSlot::Center => y_base,
                    TextVSlot::Below => y_base - hc * 0.5 - hb1 * 0.5,
                    TextVSlot::Below2 => y_base - hc * 0.5 - hb1 - hb2 * 0.5,
                };

                let line_y = center.y + y;

                if t.rjust {
                    bounds.expand_point(Point::new(center.x - text_w, line_y - ch));
                    bounds.expand_point(Point::new(center.x, line_y + ch));
                } else if t.ljust {
                    bounds.expand_point(Point::new(center.x, line_y - ch));
                    bounds.expand_point(Point::new(center.x + text_w, line_y + ch));
                } else {
                    bounds.expand_point(Point::new(center.x - text_w / 2.0, line_y - ch));
                    bounds.expand_point(Point::new(center.x + text_w / 2.0, line_y + ch));
                }
            }
        }
    }
}

/// An arc shape - a curved arc between two points
#[derive(Debug, Clone)]
pub struct ArcShape {
    pub start: PointIn,
    pub end: PointIn,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
    pub clockwise: bool,
}

impl ArcShape {
    pub fn new(start: PointIn, end: PointIn, clockwise: bool) -> Self {
        Self {
            start,
            end,
            style: ObjectStyle::default(),
            text: Vec::new(),
            clockwise,
        }
    }
}

impl Shape for ArcShape {
    fn center(&self) -> PointIn {
        self.start.midpoint(self.end)
    }

    fn width(&self) -> Inches {
        let delta = self.end - self.start;
        delta.dx.abs()
    }

    fn height(&self) -> Inches {
        let delta = self.end - self.start;
        delta.dy.abs()
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn start(&self) -> PointIn {
        self.start
    }

    fn end(&self) -> PointIn {
        self.end
    }

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        arrow_len: Inches,
        arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: arcRender (pikchr.c:4485) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        // Convert points to SVG coordinates with proper Y-flipping
        let start_svg = self.start.to_svg(scaler, offset_x, max_y);
        let end_svg = self.end.to_svg(scaler, offset_x, max_y);

        // Calculate control point for arrowheads
        let control = arc_control_point(self.style.clockwise, start_svg, end_svg);

        // Calculate arrow dimensions
        let arrow_len = scaler.px(arrow_len);
        let arrow_wid = scaler.px(arrow_wid);

        // Render arrowheads first (like svg.rs does)
        if self.style.arrow_start {
            if let Some(arrowhead) =
                render_arrowhead_dom(control, start_svg, &self.style, arrow_len, arrow_wid)
            {
                nodes.push(SvgNode::Polygon(arrowhead));
            }
        }
        if self.style.arrow_end {
            if let Some(arrowhead) =
                render_arrowhead_dom(control, end_svg, &self.style, arrow_len, arrow_wid)
            {
                nodes.push(SvgNode::Polygon(arrowhead));
            }
        }

        // Render the arc path
        let svg_style = build_svg_style(&self.style, scaler, dashwid);
        let arc_path_data = create_arc_path(start_svg, end_svg, self.style.clockwise);

        let arc_path = Path {
            d: Some(arc_path_data),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dasharray: None,
            style: svg_style,
        };
        nodes.push(SvgNode::Path(arc_path));

        nodes
    }

    fn translate(&mut self, offset: OffsetIn) {
        self.start += offset;
        self.end += offset;
    }

    /// cref: pik_bbox_add_elist (pikchr.c:7206) - arc bbox from endpoints
    fn expand_bounds(&self, bounds: &mut BoundingBox) {
        bounds.expand_point(self.start);
        bounds.expand_point(self.end);
        // Include text labels (must account for font scaling)
        if !self.text.is_empty() {
            let charht = defaults::FONT_SIZE;
            let (text_above, text_below) = sum_text_heights_above_below(&self.text, charht);
            let center = self.center();
            bounds.expand_point(Point::new(center.x, center.y + Inches(text_above)));
            bounds.expand_point(Point::new(center.x, center.y - Inches(text_below)));
        }
    }
}

/// A move shape - invisible positioning, renders nothing
#[derive(Debug, Clone)]
pub struct MoveShape {
    pub start: PointIn,
    pub end: PointIn,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
}

impl MoveShape {
    pub fn new(start: PointIn, end: PointIn) -> Self {
        Self {
            start,
            end,
            style: ObjectStyle::default(),
            text: Vec::new(),
        }
    }
}

impl Shape for MoveShape {
    fn center(&self) -> PointIn {
        self.start.midpoint(self.end)
    }

    fn width(&self) -> Inches {
        let delta = self.end - self.start;
        delta.dx.abs()
    }

    fn height(&self) -> Inches {
        let delta = self.end - self.start;
        delta.dy.abs()
    }

    fn style(&self) -> &ObjectStyle {
        &self.style
    }

    fn text(&self) -> &[PositionedText] {
        &self.text
    }

    fn start(&self) -> PointIn {
        self.start
    }

    fn end(&self) -> PointIn {
        self.end
    }

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        _scaler: &Scaler,
        _offset_x: Inches,
        _offset_y: Inches,
        _dashwid: Inches,
        _arrow_len: Inches,
        _arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        // Move shapes are invisible - render nothing
        Vec::new()
    }

    fn translate(&mut self, offset: OffsetIn) {
        self.start += offset;
        self.end += offset;
    }
}

/// A sublist shape - container for child shapes
#[derive(Debug, Clone)]
pub struct SublistShape {
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
    pub children: Vec<RenderedObject>,
}

impl SublistShape {
    pub fn new(center: PointIn, width: Inches, height: Inches) -> Self {
        Self {
            center,
            width,
            height,
            style: ObjectStyle::default(),
            text: Vec::new(),
            children: Vec::new(),
        }
    }
}

impl Shape for SublistShape {
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

    fn render_svg(
        &self,
        _obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        arrow_len: Inches,
        arrow_wid: Inches,
    ) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        for child in &self.children {
            let child_shape = &child.shape;
            let child_nodes = child_shape.render_svg(
                child, scaler, offset_x, max_y, dashwid, arrow_len, arrow_wid,
            );
            nodes.extend(child_nodes);
        }

        nodes
    }

    fn translate(&mut self, offset: OffsetIn) {
        self.center += offset;
        for child in self.children.iter_mut() {
            child.translate(offset);
        }
    }

    /// cref: pik_bbox_add_elist (pikchr.c:7206) - sublist bbox from children
    fn expand_bounds(&self, bounds: &mut BoundingBox) {
        for child in &self.children {
            let shape = &child.shape;
            shape.expand_bounds(bounds);
        }
    }
}

// ============================================================================
// Shape Enum
// ============================================================================

/// A shape enum wrapping all shape types
///
/// This provides uniform storage while each variant holds shape-specific geometry.
#[derive(Debug, Clone)]
#[enum_dispatch(Shape)]
pub enum ShapeEnum {
    Box(BoxShape),
    Circle(CircleShape),
    Ellipse(EllipseShape),
    Oval(OvalShape),
    Diamond(DiamondShape),
    Cylinder(CylinderShape),
    File(FileShape),
    Line(LineShape),
    Spline(SplineShape),
    Dot(DotShape),
    Text(TextShape),
    Arc(ArcShape),
    Move(MoveShape),
    Sublist(SublistShape),
}

impl ShapeEnum {
    /// Whether this shape is a path (line-like)
    pub fn is_path(&self) -> bool {
        matches!(
            self,
            ShapeEnum::Line(_) | ShapeEnum::Spline(_) | ShapeEnum::Arc(_) | ShapeEnum::Move(_)
        )
    }

    /// Set the center point of the shape
    pub fn set_center(&mut self, center: PointIn) {
        match self {
            ShapeEnum::Box(s) => s.center = center,
            ShapeEnum::Circle(s) => s.center = center,
            ShapeEnum::Ellipse(s) => s.center = center,
            ShapeEnum::Oval(s) => s.center = center,
            ShapeEnum::Diamond(s) => s.center = center,
            ShapeEnum::Cylinder(s) => s.center = center,
            ShapeEnum::File(s) => s.center = center,
            ShapeEnum::Line(_) | ShapeEnum::Spline(_) | ShapeEnum::Arc(_) | ShapeEnum::Move(_) => {
                // For line-like shapes, center is computed from waypoints/endpoints
                // This operation doesn't make sense, but we can no-op for compatibility
            }
            ShapeEnum::Dot(s) => s.center = center,
            ShapeEnum::Text(s) => s.center = center,
            ShapeEnum::Sublist(s) => s.center = center,
        }
    }

    /// Get mutable reference to the style
    pub fn style_mut(&mut self) -> &mut ObjectStyle {
        match self {
            ShapeEnum::Box(s) => &mut s.style,
            ShapeEnum::Circle(s) => &mut s.style,
            ShapeEnum::Ellipse(s) => &mut s.style,
            ShapeEnum::Oval(s) => &mut s.style,
            ShapeEnum::Diamond(s) => &mut s.style,
            ShapeEnum::Cylinder(s) => &mut s.style,
            ShapeEnum::File(s) => &mut s.style,
            ShapeEnum::Line(s) => &mut s.style,
            ShapeEnum::Spline(s) => &mut s.style,
            ShapeEnum::Dot(s) => &mut s.style,
            ShapeEnum::Text(s) => &mut s.style,
            ShapeEnum::Arc(s) => &mut s.style,
            ShapeEnum::Move(s) => &mut s.style,
            ShapeEnum::Sublist(s) => &mut s.style,
        }
    }

    /// Get mutable reference to text labels
    pub fn text_mut(&mut self) -> &mut Vec<PositionedText> {
        match self {
            ShapeEnum::Box(s) => &mut s.text,
            ShapeEnum::Circle(s) => &mut s.text,
            ShapeEnum::Ellipse(s) => &mut s.text,
            ShapeEnum::Oval(s) => &mut s.text,
            ShapeEnum::Diamond(s) => &mut s.text,
            ShapeEnum::Cylinder(s) => &mut s.text,
            ShapeEnum::File(s) => &mut s.text,
            ShapeEnum::Line(s) => &mut s.text,
            ShapeEnum::Spline(s) => &mut s.text,
            ShapeEnum::Dot(s) => &mut s.text,
            ShapeEnum::Text(s) => &mut s.text,
            ShapeEnum::Arc(s) => &mut s.text,
            ShapeEnum::Move(s) => &mut s.text,
            ShapeEnum::Sublist(s) => &mut s.text,
        }
    }

    /// Get reference to children (for Sublist only)
    pub fn children(&self) -> Option<&Vec<RenderedObject>> {
        match self {
            ShapeEnum::Sublist(s) => Some(&s.children),
            _ => None,
        }
    }

    /// Get mutable reference to children (for Sublist only)
    pub fn children_mut(&mut self) -> Option<&mut Vec<RenderedObject>> {
        match self {
            ShapeEnum::Sublist(s) => Some(&mut s.children),
            _ => None,
        }
    }

    /// Get mutable reference to waypoints (for Line and Spline)
    pub fn waypoints_mut(&mut self) -> Option<&mut Vec<PointIn>> {
        match self {
            ShapeEnum::Line(s) => Some(&mut s.waypoints),
            ShapeEnum::Spline(s) => Some(&mut s.waypoints),
            _ => None,
        }
    }

    /// Get the ClassName for this shape
    pub fn class(&self) -> ClassName {
        match self {
            ShapeEnum::Box(_) => ClassName::Box,
            ShapeEnum::Circle(_) => ClassName::Circle,
            ShapeEnum::Ellipse(_) => ClassName::Ellipse,
            ShapeEnum::Oval(_) => ClassName::Oval,
            ShapeEnum::Diamond(_) => ClassName::Diamond,
            ShapeEnum::Cylinder(_) => ClassName::Cylinder,
            ShapeEnum::File(_) => ClassName::File,
            ShapeEnum::Line(_) => ClassName::Line,
            ShapeEnum::Spline(_) => ClassName::Spline,
            ShapeEnum::Dot(_) => ClassName::Dot,
            ShapeEnum::Text(_) => ClassName::Text,
            ShapeEnum::Arc(_) => ClassName::Arc,
            ShapeEnum::Move(_) => ClassName::Move,
            ShapeEnum::Sublist(_) => ClassName::Sublist,
        }
    }
}

// Note: From impls are automatically generated by enum_dispatch

// ============================================================================
// Helper Functions
// ============================================================================

/// Build an SVG style from an ObjectStyle
/// cref: pik_append_style (pikchr.c:2277)
fn build_svg_style(style: &ObjectStyle, scaler: &Scaler, dashwid: Inches) -> SvgStyle {
    build_svg_style_ex(style, scaler, dashwid, false)
}

/// Build an SVG style with optional stroke-linejoin
fn build_svg_style_ex(
    style: &ObjectStyle,
    scaler: &Scaler,
    dashwid: Inches,
    add_linejoin: bool,
) -> SvgStyle {
    let fill_rgb = color_to_rgb(&style.fill);
    let stroke_rgb = color_to_rgb(&style.stroke);

    tracing::debug!(
        fill_input = %style.fill,
        fill_output = %fill_rgb,
        stroke_input = %style.stroke,
        stroke_output = %stroke_rgb,
        "[Rust build_svg_style] Converting colors"
    );

    let mut entries = vec![
        ("fill", fill_rgb),
        ("stroke", stroke_rgb),
        (
            "stroke-width",
            format!("{}", scaler.px(style.stroke_width)),
        ),
    ];

    // Dashed: dash and gap are both the stored width
    // cref: pik_append_style
    if let Some(dash_width) = style.dashed {
        let dash = scaler.px(dash_width);
        entries.push(("stroke-dasharray", format!("{},{}", dash, dash)));
    }
    // Dotted: dot is stroke width, gap is the stored width
    // cref: pik_append_style
    else if let Some(gap_width) = style.dotted {
        let dot = scaler.px(style.stroke_width);
        let gap = scaler.px(gap_width);
        entries.push(("stroke-dasharray", format!("{},{}", dot, gap)));
    }

    // Add stroke-linejoin:round for closed paths with sharp corners
    // cref: lineRender (pikchr.c:4253)
    if add_linejoin {
        entries.push(("stroke-linejoin", "round".to_string()));
    }

    // Note: dashwid parameter is kept for potential future use but not needed
    // when the style stores the width directly
    let _ = dashwid;

    svg_style_from_entries(entries)
}

pub(crate) fn svg_style_from_entries(entries: Vec<(&'static str, String)>) -> SvgStyle {
    let mut css = String::new();
    for (name, value) in entries {
        if value.is_empty() {
            continue;
        }
        css.push_str(name);
        css.push(':');
        css.push_str(&value);
        css.push(';');
    }

    if css.is_empty() {
        SvgStyle::default()
    } else {
        match SvgStyle::parse(&css) {
            Ok(style) => style,
            Err(err) => {
                tracing::warn!(css = %css, %err, "failed to parse generated SVG style");
                SvgStyle::default()
            }
        }
    }
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
        assert_eq!(north.y, Inches(1.0)); // Y-up: North = +Y

        let east = circle.edge_point(EdgeDirection::East);
        assert_eq!(east.x, Inches(1.0));
        assert_eq!(east.y, Inches(0.0));

        // Diagonal should be at 1/sqrt(2) distance, not corner
        // Y-up: NorthEast = (+x, +y)
        let ne = circle.edge_point(EdgeDirection::NorthEast);
        let expected = std::f64::consts::FRAC_1_SQRT_2;
        assert!((ne.x.0 - expected).abs() < 0.001);
        assert!((ne.y.0 - expected).abs() < 0.001); // Y-up: +Y for North
    }

    #[test]
    fn box_dimensions() {
        let bx = BoxShape::new(
            Point::new(Inches(0.0), Inches(0.0)),
            Inches(2.0),
            Inches(1.0),
        );
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
