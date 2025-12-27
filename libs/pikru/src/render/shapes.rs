//! Shape types for pikchr rendering
//!
//! Each shape is its own type that knows how to:
//! - Calculate its bounding box
//! - Find edge points for line connections
//! - Render itself to SVG

use crate::types::{BoxIn, Length as Inches, OffsetIn, Point, Scaler, Size, UnitVec};
use facet_svg::{Circle as SvgCircle, Ellipse as SvgEllipse, Path, PathData, SvgNode, SvgStyle};
use glam::DVec2;

use super::defaults;
use super::{TextVSlot, compute_text_vslots, sum_text_heights_above_below};

/// Bounding box type alias
pub type BoundingBox = BoxIn;
use super::geometry::{
    arc_control_point, chop_line, create_arc_path_with_control, create_cylinder_paths_with_rad,
    create_file_paths, create_line_path, create_oval_path, create_rounded_box_path,
    create_spline_path,
};
use super::svg::{color_to_rgb, color_to_string, render_arrowhead_dom};
use super::types::{ClassName, ObjectStyle, PointIn, PositionedText, RenderedObject};

use enum_dispatch::enum_dispatch;

/// Context for rendering shapes, bundling all the parameters needed for SVG generation
pub struct ShapeRenderContext<'a> {
    pub scaler: &'a Scaler,
    pub offset_x: Inches,
    pub max_y: Inches,
    pub dashwid: Inches,
    pub arrow_len: Inches,
    pub arrow_wid: Inches,
    pub thickness: Inches,
    pub use_css_vars: bool,
}

/// Shorten a point toward another point by a given amount (in pixels)
/// cref: pik_chop (pikchr.c:1958-1970)
fn chop_point(from: DVec2, to: DVec2, amount: f64) -> DVec2 {
    let delta = to - from;
    let dist = delta.length();

    if dist <= amount {
        return from;
    }

    let r = 1.0 - amount / dist;
    from + delta * r
}

/// Shorten a waypoint list from the start (for start arrows)
/// cref: pik_chop (pikchr.c:1958-1970)
fn chop_waypoint_start(waypoints: &mut Vec<PointIn>, amount: Inches) {
    if waypoints.len() < 2 {
        return;
    }
    let from = waypoints[1];
    let to = waypoints[0];
    let delta = to - from;
    let dist = (delta.dx.0 * delta.dx.0 + delta.dy.0 * delta.dy.0).sqrt();

    if dist <= amount.0 {
        waypoints[0] = from;
        return;
    }

    let r = 1.0 - amount.0 / dist;
    waypoints[0] = Point::new(
        Inches(from.x.0 + r * delta.dx.0),
        Inches(from.y.0 + r * delta.dy.0),
    );
}

/// Shorten a waypoint list from the end (for end arrows)
/// cref: pik_chop (pikchr.c:1958-1970)
fn chop_waypoint_end(waypoints: &mut Vec<PointIn>, amount: Inches) {
    if waypoints.len() < 2 {
        return;
    }
    let n = waypoints.len();
    let from = waypoints[n - 2];
    let to = waypoints[n - 1];
    let delta = to - from;
    let dist = (delta.dx.0 * delta.dx.0 + delta.dy.0 * delta.dy.0).sqrt();

    if dist <= amount.0 {
        waypoints[n - 1] = from;
        return;
    }

    let r = 1.0 - amount.0 / dist;
    waypoints[n - 1] = Point::new(
        Inches(from.x.0 + r * delta.dx.0),
        Inches(from.y.0 + r * delta.dy.0),
    );
}

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
    /// cref: pik_draw_arrowhead (pikchr.c:4666-4667) - arrow dimensions scale with stroke width
    fn render_svg(&self, obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode>;

    /// Get waypoints if this is a path-like shape (line, spline)
    /// Returns None for non-path shapes
    fn waypoints(&self) -> Option<&[PointIn]> {
        None
    }

    /// Translate this shape by an offset
    fn translate(&mut self, offset: OffsetIn);

    /// Expand a bounding box to include this shape's "core" bounds (without arrowheads).
    /// Used for computing sublist width/height (pObj->w/h in C).
    /// cref: pikchr.y:1757-1761 - sublist bbox computed from children's bbox (no arrowheads)
    /// Default implementation calls expand_bounds; LineShape overrides to exclude arrowheads.
    fn expand_core_bounds(&self, bounds: &mut BoundingBox) {
        self.expand_bounds(bounds);
    }

    /// Expand a bounding box to include this shape (including arrowheads for lines).
    /// Used for final SVG bounding box computation.
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

            // cref: pik_append_txt (pikchr.c:2484-2528) - text bounds are always added
            // Include text labels for visible objects too
            if !text.is_empty() {
                let charht = defaults::FONT_SIZE;
                let charwid = defaults::CHARWID;
                let (text_above, text_below) = sum_text_heights_above_below(text, charht);
                // Compute max text half-width (centered text extends +/- hw from center.x)
                let max_hw = text
                    .iter()
                    .map(|t| Inches(t.width_inches(charwid)) / 2.0)
                    .fold(Inches::ZERO, |acc, hw| if hw > acc { hw } else { acc });
                bounds.expand_point(Point::new(
                    center.x - max_hw,
                    center.y - Inches(text_below),
                ));
                bounds.expand_point(Point::new(
                    center.x + max_hw,
                    center.y + Inches(text_above),
                ));
            }
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

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: circleRender (pikchr.c:3961) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
        let r = ctx.scaler.px(self.radius);

        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);

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

    /// Calculate edge point for boxes, accounting for corner_radius on diagonal edges
    /// cref: boxOffset (pikchr.c:1104-1130) - diagonal corners are inset by (1-1/√2)*rad
    /// Note: Uses internal Y-up coordinates (positive Y = north)
    fn edge_point(&self, direction: EdgeDirection) -> PointIn {
        match direction {
            EdgeDirection::Center => return self.center,
            EdgeDirection::Start => return self.edge_point(EdgeDirection::West),
            EdgeDirection::End => return self.edge_point(EdgeDirection::East),
            _ => {}
        }

        let hw = self.width / 2.0;
        let hh = self.height / 2.0;

        // For boxes with corner_radius, diagonal edges are inset
        // cref: boxOffset uses rx = 0.29289321881345252392 * rad (which is (1 - 1/√2) * rad)
        let is_diagonal = matches!(
            direction,
            EdgeDirection::NorthEast
                | EdgeDirection::NorthWest
                | EdgeDirection::SouthEast
                | EdgeDirection::SouthWest
        );

        let (offset_x, offset_y) = if is_diagonal && self.corner_radius > Inches::ZERO {
            // Clamp radius like C does: rad = min(rad, w2, h2)
            let rad = self.corner_radius.min(hw).min(hh);
            // rx = (1 - 1/√2) * rad ≈ 0.29289 * rad
            let rx = Inches(0.29289321881345252392 * rad.0);

            // Determine signs based on direction (Y-up: north=+Y, south=-Y)
            let (sign_x, sign_y) = match direction {
                EdgeDirection::NorthEast => (1.0, 1.0),   // +x, +y (up in Y-up coords)
                EdgeDirection::NorthWest => (-1.0, 1.0),  // -x, +y
                EdgeDirection::SouthEast => (1.0, -1.0),  // +x, -y (down in Y-up coords)
                EdgeDirection::SouthWest => (-1.0, -1.0), // -x, -y
                _ => unreachable!(),
            };

            // Offset is (w2 - rx) for diagonal corners
            (Inches(sign_x * (hw.0 - rx.0)), Inches(sign_y * (hh.0 - rx.0)))
        } else if is_diagonal {
            // Non-rounded box: diagonal corners at full (hw, hh)
            let (sign_x, sign_y) = match direction {
                EdgeDirection::NorthEast => (1.0, 1.0),
                EdgeDirection::NorthWest => (-1.0, 1.0),
                EdgeDirection::SouthEast => (1.0, -1.0),
                EdgeDirection::SouthWest => (-1.0, -1.0),
                _ => unreachable!(),
            };
            (Inches(sign_x * hw.0), Inches(sign_y * hh.0))
        } else {
            // Cardinal directions: use unit vector scaled by hw/hh
            let unit = direction.unit_vec();
            (hw * unit.dx(), hh * unit.dy())
        };

        self.center + OffsetIn::new(offset_x, offset_y)
    }

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: boxRender (pikchr.c:3856) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
        let hw = ctx.scaler.px(self.width / 2.0);
        let hh = ctx.scaler.px(self.height / 2.0);
        let x1 = center_svg.x - hw;
        let x2 = center_svg.x + hw;
        let y1 = center_svg.y - hh;
        let y2 = center_svg.y + hh;

        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);

        let path_data = if self.corner_radius > Inches::ZERO {
            let r = ctx.scaler.px(self.corner_radius);
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

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: ovalRender (pikchr.c:3987) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
        let rx = ctx.scaler.px(self.width / 2.0);
        let ry = ctx.scaler.px(self.height / 2.0);

        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);

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

    /// Calculate edge point for ovals (pill shapes)
    /// cref: boxOffset (pikchr.c:1178-1212) - oval uses boxOffset with rad = min(w2, h2)
    /// The diagonal corners are inset by rx = 0.29289 * rad to sit on the rounded corner
    /// Note: Uses internal Y-up coordinates (positive Y = north)
    fn edge_point(&self, direction: EdgeDirection) -> PointIn {
        match direction {
            EdgeDirection::Center => return self.center,
            EdgeDirection::Start => return self.edge_point(EdgeDirection::West),
            EdgeDirection::End => return self.edge_point(EdgeDirection::East),
            _ => {}
        }

        let hw = self.width / 2.0;
        let hh = self.height / 2.0;

        // Oval uses rad = min(w2, h2), which is half the smaller dimension
        // cref: ovalNumProp line 1289: pObj->rad = 0.5*(pObj->h<pObj->w?pObj->h:pObj->w)
        let rad = hw.min(hh);

        // rx = (1 - cos(45°)) * rad ≈ 0.29289 * rad
        // cref: boxOffset lines 1181-1183
        let rx = Inches(0.29289321881345252392 * rad.0);

        let (offset_x, offset_y) = match direction {
            // Cardinal directions use full half-dimensions
            EdgeDirection::North => (Inches::ZERO, hh),
            EdgeDirection::East => (hw, Inches::ZERO),
            EdgeDirection::South => (Inches::ZERO, -hh),
            EdgeDirection::West => (-hw, Inches::ZERO),
            // Diagonal directions are inset by rx to sit on the rounded corner
            EdgeDirection::NorthEast => (hw - rx, hh - rx),
            EdgeDirection::SouthEast => (hw - rx, -(hh - rx)),
            EdgeDirection::SouthWest => (-(hw - rx), -(hh - rx)),
            EdgeDirection::NorthWest => (-(hw - rx), hh - rx),
            _ => return self.center,
        };

        self.center + OffsetIn::new(offset_x, offset_y)
    }

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: ovalRender (pikchr.c:3987) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
        let hw = ctx.scaler.px(self.width / 2.0);
        let hh = ctx.scaler.px(self.height / 2.0);
        let x1 = center_svg.x - hw;
        let x2 = center_svg.x + hw;
        let y1 = center_svg.y - hh;
        let y2 = center_svg.y + hh;

        // Oval radius is half the smaller dimension
        let rad = ctx.scaler.px(self.width.min(self.height) / 2.0);

        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);

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

    /// Calculate edge point for diamonds
    /// cref: diamondOffset (pikchr.c:1397-1417) - diagonal corners use quarter dimensions (w/4, h/4)
    /// Note: Uses internal Y-up coordinates (positive Y = north)
    fn edge_point(&self, direction: EdgeDirection) -> PointIn {
        match direction {
            EdgeDirection::Center => return self.center,
            EdgeDirection::Start => return self.edge_point(EdgeDirection::West),
            EdgeDirection::End => return self.edge_point(EdgeDirection::East),
            _ => {}
        }

        let hw = self.width / 2.0;
        let hh = self.height / 2.0;
        let qw = self.width / 4.0; // w4 in C
        let qh = self.height / 4.0; // h4 in C

        let (offset_x, offset_y) = match direction {
            // Cardinal directions use full half-dimensions (Y-up: north=+Y, south=-Y)
            EdgeDirection::North => (Inches::ZERO, hh),
            EdgeDirection::East => (hw, Inches::ZERO),
            EdgeDirection::South => (Inches::ZERO, -hh),
            EdgeDirection::West => (-hw, Inches::ZERO),
            // Diagonal directions use quarter dimensions
            EdgeDirection::NorthEast => (qw, qh),
            EdgeDirection::SouthEast => (qw, -qh),
            EdgeDirection::SouthWest => (-qw, -qh),
            EdgeDirection::NorthWest => (-qw, qh),
            _ => return self.center,
        };

        self.center + OffsetIn::new(offset_x, offset_y)
    }

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: diamondRender (pikchr.c:4058) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
        let hw = ctx.scaler.px(self.width / 2.0);
        let hh = ctx.scaler.px(self.height / 2.0);

        tracing::debug!(
            center_pikchr_x = self.center.x.0,
            center_pikchr_y = self.center.y.0,
            center_svg_x = center_svg.x,
            center_svg_y = center_svg.y,
            offset_x = ctx.offset_x.0,
            max_y = ctx.max_y.0,
            "DiamondShape render_svg"
        );

        let left = center_svg.x - hw;
        let right = center_svg.x + hw;
        let top = center_svg.y - hh;
        let bottom = center_svg.y + hh;

        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);

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

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: cylinderRender (pikchr.c:4112) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
        let w = ctx.scaler.px(self.width);
        let h = ctx.scaler.px(self.height);

        // cref: cylinderRender - use stored cylrad, clamped to half height
        let mut rad = ctx.scaler.px(self.ellipse_rad);
        let h2 = h / 2.0;
        if rad > h2 {
            rad = h2;
        } else if rad < 0.0 {
            rad = 0.0;
        }

        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);

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

    /// Cylinder edge points: diagonal corners are inset by the ellipse radius.
    /// cref: cylinderOffset (pikchr.c:1378-1417)
    fn edge_point(&self, direction: EdgeDirection) -> PointIn {
        match direction {
            EdgeDirection::Center => return self.center(),
            EdgeDirection::Start => return self.start(),
            EdgeDirection::End => return self.end(),
            _ => {}
        }

        let hw = self.width / 2.0;
        let hh = self.height / 2.0;

        // Diagonal corners are inset by the ellipse radius
        // cref: cylinderOffset - h2 = h1 - rad
        let hh_inner = hh - self.ellipse_rad;

        let offset = match direction {
            EdgeDirection::North => OffsetIn::new(Inches::ZERO, hh),
            EdgeDirection::NorthEast => OffsetIn::new(hw, hh_inner),
            EdgeDirection::East => OffsetIn::new(hw, Inches::ZERO),
            EdgeDirection::SouthEast => OffsetIn::new(hw, -hh_inner),
            EdgeDirection::South => OffsetIn::new(Inches::ZERO, -hh),
            EdgeDirection::SouthWest => OffsetIn::new(-hw, -hh_inner),
            EdgeDirection::West => OffsetIn::new(-hw, Inches::ZERO),
            EdgeDirection::NorthWest => OffsetIn::new(-hw, hh_inner),
            EdgeDirection::Center | EdgeDirection::Start | EdgeDirection::End => {
                unreachable!("handled above")
            }
        };

        self.center + offset
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

    /// Calculate edge point for file shapes
    /// cref: fileOffset (pikchr.c:1491-1540) - only NE corner is inset for the fold
    /// rx = 0.5 * rad, clamped to [mn*0.25, mn] where mn = min(w2, h2)
    /// Note: Uses internal Y-up coordinates (positive Y = north)
    fn edge_point(&self, direction: EdgeDirection) -> PointIn {
        match direction {
            EdgeDirection::Center => return self.center,
            EdgeDirection::Start => return self.edge_point(EdgeDirection::West),
            EdgeDirection::End => return self.edge_point(EdgeDirection::East),
            _ => {}
        }

        let hw = self.width / 2.0;
        let hh = self.height / 2.0;

        // cref: fileOffset lines 1493-1500
        // rx = 0.5 * rad, clamped to [mn*0.25, mn] where mn = min(w2, h2)
        let mn = hw.min(hh);
        let mut rx = self.fold_radius;
        if rx > mn {
            rx = mn;
        }
        if rx < mn * 0.25 {
            rx = mn * 0.25;
        }
        rx = rx * 0.5;

        let (offset_x, offset_y) = match direction {
            // Cardinal directions use full half-dimensions
            EdgeDirection::North => (Inches::ZERO, hh),
            EdgeDirection::East => (hw, Inches::ZERO),
            EdgeDirection::South => (Inches::ZERO, -hh),
            EdgeDirection::West => (-hw, Inches::ZERO),
            // Only NE is inset for the fold
            EdgeDirection::NorthEast => (hw - rx, hh - rx),
            // Other diagonals are NOT inset (unlike box/oval)
            EdgeDirection::SouthEast => (hw, -hh),
            EdgeDirection::SouthWest => (-hw, -hh),
            EdgeDirection::NorthWest => (-hw, hh),
            _ => return self.center,
        };

        self.center + OffsetIn::new(offset_x, offset_y)
    }

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: fileRender (pikchr.c:4171) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
        let w = ctx.scaler.px(self.width);
        let h = ctx.scaler.px(self.height);

        // cref: fileRender (pikchr.c:1546-1548) - clamp rad to fit and ensure minimum
        // mn = min(w/2, h/2)
        // rad clamped to: mn*0.25 <= rad <= mn
        let w2 = w / 2.0;
        let h2 = h / 2.0;
        let mn = w2.min(h2);
        let rad = ctx.scaler.px(self.fold_radius).min(mn).max(mn * 0.25);

        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);

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
                format!("{}", ctx.scaler.px(self.style.stroke_width)),
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
        // For closed polygons, center should be the center of the bounding box
        // cref: pik_bbox_add_elist (pikchr.c:7206)
        let mut min_x = self.waypoints[0].x;
        let mut max_x = self.waypoints[0].x;
        let mut min_y = self.waypoints[0].y;
        let mut max_y = self.waypoints[0].y;
        for pt in &self.waypoints {
            if pt.x < min_x {
                min_x = pt.x;
            }
            if pt.x > max_x {
                max_x = pt.x;
            }
            if pt.y < min_y {
                min_y = pt.y;
            }
            if pt.y > max_y {
                max_y = pt.y;
            }
        }
        Point::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0)
    }

    fn width(&self) -> Inches {
        if self.waypoints.is_empty() {
            return Inches::ZERO;
        }
        // Compute bounding box width from all waypoints
        // cref: pik_bbox_add_elist (pikchr.c:7206) - line bbox from all waypoints
        let mut min_x = self.waypoints[0].x;
        let mut max_x = self.waypoints[0].x;
        for pt in &self.waypoints {
            if pt.x < min_x {
                min_x = pt.x;
            }
            if pt.x > max_x {
                max_x = pt.x;
            }
        }
        max_x - min_x
    }

    fn height(&self) -> Inches {
        if self.waypoints.is_empty() {
            return Inches::ZERO;
        }
        // Compute bounding box height from all waypoints
        // cref: pik_bbox_add_elist (pikchr.c:7206) - line bbox from all waypoints
        let mut min_y = self.waypoints[0].y;
        let mut max_y = self.waypoints[0].y;
        for pt in &self.waypoints {
            if pt.y < min_y {
                min_y = pt.y;
            }
            if pt.y > max_y {
                max_y = pt.y;
            }
        }
        max_y - min_y
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

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: lineRender (pikchr.c:4228) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 || self.waypoints.len() < 2 {
            return nodes;
        }

        // cref: lineRender (pikchr.c:4253) - add stroke-linejoin:round for sharp-cornered paths
        // This applies to closed paths or multi-segment open paths with no corner radius
        let add_linejoin = (self.style.close_path || self.waypoints.len() > 2)
            && self.style.corner_radius.raw() == 0.0;
        // For non-closed lines, fill should be "none" even if specified
        // cref: lineRender (pikchr.c:4228) - only closed paths can be filled
        let allow_fill = self.style.close_path;
        let svg_style = build_svg_style_full(&self.style, ctx.scaler, ctx.dashwid, add_linejoin, allow_fill, ctx.use_css_vars);

        // cref: pik_draw_arrowhead (pikchr.c:4666-4667)
        // Arrow dimensions scale with object's stroke width relative to global thickness
        // h = p->hArrow * pObj->sw, where p->hArrow = arrowht / thickness
        // So: h = arrowht * (pObj->sw / thickness)
        let arrow_scale = if ctx.thickness.raw() > 0.0 {
            self.style.stroke_width.raw() / ctx.thickness.raw()
        } else {
            1.0
        };
        let arrow_len_px = ctx.scaler.px(ctx.arrow_len) * arrow_scale;
        let arrow_wid_px = ctx.scaler.px(ctx.arrow_wid) * arrow_scale;
        let arrow_chop = arrow_len_px / 2.0;

        let mut svg_points: Vec<DVec2> = self
            .waypoints
            .iter()
            .map(|pt| pt.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y))
            .collect();

        if svg_points.len() <= 2 {
            // Waypoints are already chopped during construction (see autochop_inches in mod.rs)
            // cref: pik_after_adding_attributes (pikchr.c:4372-4379)
            let mut draw_start = svg_points[0];
            let mut draw_end = svg_points[svg_points.len() - 1];

            // cref: lineRender (pikchr.c:4271-4276) - larrow first, then rarrow
            if self.style.arrow_start {
                if let Some(arrowhead) = render_arrowhead_dom(
                    draw_end,
                    draw_start,
                    &self.style,
                    arrow_len_px,
                    arrow_wid_px,
                    ctx.use_css_vars,
                ) {
                    nodes.push(SvgNode::Polygon(arrowhead));
                }
            }
            if self.style.arrow_end {
                if let Some(arrowhead) = render_arrowhead_dom(
                    draw_start,
                    draw_end,
                    &self.style,
                    arrow_len_px,
                    arrow_wid_px,
                    ctx.use_css_vars,
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

        // NOTE: Autochop is now handled in mod.rs via autochop_inches().
        // The style.chop flag is used there to determine if chopping should occur.
        // We don't need to do additional chopping here because:
        // 1. If there are object attachments, autochop_inches already chopped against them
        // 2. If there are no attachments but chop is set, the line doesn't need chopping
        //    (chop only makes sense when connecting to an object)
        //
        // The old code here was applying CIRCLE_RADIUS chop to both endpoints,
        // which was wrong when autochop had already been applied to one or both ends.

        // cref: lineRender (pikchr.c:4271-4276) - larrow first, then rarrow
        // Render arrowheads before chopping endpoints
        if svg_points.len() >= 2 {
            if self.style.arrow_start {
                if let Some(arrowhead) = render_arrowhead_dom(
                    svg_points[1],
                    svg_points[0],
                    &self.style,
                    arrow_len_px,
                    arrow_wid_px,
                    ctx.use_css_vars,
                ) {
                    nodes.push(SvgNode::Polygon(arrowhead));
                }
            }
            let n = svg_points.len();
            if self.style.arrow_end {
                if let Some(arrowhead) = render_arrowhead_dom(
                    svg_points[n - 2],
                    svg_points[n - 1],
                    &self.style,
                    arrow_len_px,
                    arrow_wid_px,
                    ctx.use_css_vars,
                ) {
                    nodes.push(SvgNode::Polygon(arrowhead));
                }
            }
            // Chop endpoints for arrow space
            if self.style.arrow_start {
                let (new_start, _) = chop_line(svg_points[0], svg_points[1], arrow_chop);
                svg_points[0] = new_start;
            }
            if self.style.arrow_end {
                let (_, new_end) = chop_line(svg_points[n - 2], svg_points[n - 1], arrow_chop);
                svg_points[n - 1] = new_end;
            }
        }

        // cref: lineRender (pikchr.c:4302-4336) - rounded corners with rad attribute
        let corner_radius_px = ctx.scaler.px(self.style.corner_radius);
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
        // Only expand by waypoints if visible (not invisible and stroke width non-negative)
        // cref: pikchr.y:693 - invis sets sw to negative in C, we use a separate flag
        if !self.style.invisible && self.style.stroke_width.0 >= 0.0 {
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


        // Include text labels with full horizontal and vertical extent
        // cref: pik_append_txt (pikchr.c:5169-5218) - text bbox expansion with justification
        if !self.text.is_empty() {
            let charht = Inches(defaults::FONT_SIZE);
            let charwid = defaults::CHARWID;
            let center = self.center();

            // Compute vertical slot assignments matching C's pik_txt_vertical_layout
            let vslots = compute_text_vslots(&self.text);

            // Compute heights for each region (max charht in each slot)
            // cref: pikchr.c:5104-5150
            // cref: pikchr.c:5106-5108 - for lines, hc starts at sw*1.5
            let sw = self.style.stroke_width.0.max(0.0);
            let mut hc = Inches(sw * 1.5);
            let mut ha1 = Inches::ZERO;
            let mut ha2 = Inches::ZERO;
            let mut hb1 = Inches::ZERO;
            let mut hb2 = Inches::ZERO;

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

            // Calculate Y position and expand bounds for each text line
            // cref: pikchr.c:5156-5218 - pik_append_txt bbox calculation
            let y_base = Inches::ZERO;

            // Compute line direction unit vector for aligned text rotation
            // cref: pikchr.c:5194-5212 - rotation transform for aligned text
            let (line_dx, line_dy) = if self.waypoints.len() >= 2 {
                let start = self.waypoints[0];
                let end = self.waypoints[self.waypoints.len() - 1];
                let dx = (end.x - start.x).0;
                let dy = (end.y - start.y).0;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > 0.0 {
                    (dx / dist, dy / dist)
                } else {
                    (1.0, 0.0) // Default to horizontal
                }
            } else {
                (1.0, 0.0)
            };

            for (i, t) in self.text.iter().enumerate() {
                let cw = Inches(t.width_inches(charwid));
                let ch = Inches(t.height(charht.0)) / 2.0;

                // Compute y offset based on vertical slot (as if line were horizontal)
                let y = match vslots.get(i).unwrap_or(&TextVSlot::Center) {
                    TextVSlot::Above2 => y_base + hc * 0.5 + ha1 + ha2 * 0.5,
                    TextVSlot::Above => y_base + hc * 0.5 + ha1 * 0.5,
                    TextVSlot::Center => y_base,
                    TextVSlot::Below => y_base - hc * 0.5 - hb1 * 0.5,
                    TextVSlot::Below2 => y_base - hc * 0.5 - hb1 - hb2 * 0.5,
                };

                // Compute text bbox corners relative to center (as if horizontal)
                // cref: pikchr.c:5175-5195
                let (x0, y0, x1, y1) = if t.rjust {
                    // rjust: text extends left from anchor
                    (Inches::ZERO, y - ch, -cw, y + ch)
                } else if t.ljust {
                    // ljust: text extends right from anchor
                    (Inches::ZERO, y - ch, cw, y + ch)
                } else {
                    // centered
                    (cw / 2.0, y + ch, -cw / 2.0, y - ch)
                };

                // For aligned text, rotate the bbox by line angle
                // cref: pikchr.c:5197-5211 - rotation transform
                let (rx0, ry0, rx1, ry1) = if t.aligned && (line_dx != 1.0 || line_dy != 0.0) {
                    // Rotation: x' = dx*x - dy*y, y' = dy*x - dx*y
                    let new_x0 = line_dx * x0.0 - line_dy * y0.0;
                    let new_y0 = line_dy * x0.0 - line_dx * y0.0;
                    let new_x1 = line_dx * x1.0 - line_dy * y1.0;
                    let new_y1 = line_dy * x1.0 - line_dx * y1.0;
                    (Inches(new_x0), Inches(new_y0), Inches(new_x1), Inches(new_y1))
                } else {
                    (x0, y0, x1, y1)
                };

                // Add rotated bbox corners to bounds
                // cref: pikchr.c:5213-5214
                bounds.expand_point(Point::new(center.x + rx0, center.y + ry0));
                bounds.expand_point(Point::new(center.x + rx1, center.y + ry1));
            }
        }
    }

    /// Expand bounds WITHOUT arrowheads - used for computing sublist width/height
    /// cref: pikchr.y:1757-1761 - sublist bbox uses children's pObj->bbox (no arrowheads)
    /// cref: pikchr.y:4527 - pik_bbox_addbox adds pObj->bbox not arrowhead ellipses
    fn expand_core_bounds(&self, bounds: &mut BoundingBox) {
        // Only expand by waypoints if visible (not invisible and stroke width non-negative)
        if !self.style.invisible && self.style.stroke_width.0 >= 0.0 {
            for pt in &self.waypoints {
                bounds.expand_point(*pt);
            }
        }
        // NOTE: Arrowhead expansion is intentionally omitted here
        // It gets added during final SVG bbox computation via expand_bounds()
    }
}

/// A spline (curved line) shape
/// cref: splineInit (pikchr.c:1653-1657) - splines have default rad = 1000
#[derive(Debug, Clone)]
pub struct SplineShape {
    pub waypoints: Vec<PointIn>,
    pub style: ObjectStyle,
    pub text: Vec<PositionedText>,
    /// Curve radius for corners. Default is 1000 inches (effectively infinite).
    /// cref: pObj->rad in splineRender (pikchr.c:1715)
    pub radius: Inches,
}

impl Shape for SplineShape {
    fn center(&self) -> PointIn {
        if self.waypoints.is_empty() {
            return Point::ORIGIN;
        }
        // For splines with multiple waypoints, use bounding box center
        // cref: pik_bbox_add_elist (pikchr.c:7206)
        let mut min_x = self.waypoints[0].x;
        let mut max_x = self.waypoints[0].x;
        let mut min_y = self.waypoints[0].y;
        let mut max_y = self.waypoints[0].y;
        for pt in &self.waypoints {
            if pt.x < min_x {
                min_x = pt.x;
            }
            if pt.x > max_x {
                max_x = pt.x;
            }
            if pt.y < min_y {
                min_y = pt.y;
            }
            if pt.y > max_y {
                max_y = pt.y;
            }
        }
        Point::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0)
    }

    fn width(&self) -> Inches {
        if self.waypoints.is_empty() {
            return Inches::ZERO;
        }
        // Compute bounding box width from all waypoints
        // cref: pik_bbox_add_elist (pikchr.c:7206)
        let mut min_x = self.waypoints[0].x;
        let mut max_x = self.waypoints[0].x;
        for pt in &self.waypoints {
            if pt.x < min_x {
                min_x = pt.x;
            }
            if pt.x > max_x {
                max_x = pt.x;
            }
        }
        max_x - min_x
    }

    fn height(&self) -> Inches {
        if self.waypoints.is_empty() {
            return Inches::ZERO;
        }
        // Compute bounding box height from all waypoints
        // cref: pik_bbox_add_elist (pikchr.c:7206)
        let mut min_y = self.waypoints[0].y;
        let mut max_y = self.waypoints[0].y;
        for pt in &self.waypoints {
            if pt.y < min_y {
                min_y = pt.y;
            }
            if pt.y > max_y {
                max_y = pt.y;
            }
        }
        max_y - min_y
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

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: splineRender (pikchr.c:1712-1713) - checks pObj->sw>0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 || self.waypoints.len() < 2 {
            return nodes;
        }

        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);

        // cref: pik_draw_arrowhead (pikchr.c:4666-4667)
        // Arrow dimensions scale with object's stroke width relative to global thickness
        let arrow_scale = if ctx.thickness.raw() > 0.0 {
            self.style.stroke_width.raw() / ctx.thickness.raw()
        } else {
            1.0
        };
        let arrow_len_px = ctx.scaler.px(ctx.arrow_len) * arrow_scale;
        let arrow_wid_px = ctx.scaler.px(ctx.arrow_wid) * arrow_scale;

        let n = self.waypoints.len();

        // cref: pik_draw_arrowhead (pikchr.c:1977-2004) - draws arrowhead first, then shortens endpoint
        // cref: pik_chop (pikchr.c:1958-1970) - shortens line by h/2 where h = arrowht
        // cref: lineRender (pikchr.c:4271-4276) - larrow first, then rarrow
        // In C pikchr, pik_draw_arrowhead modifies aPath in place before radiusPath is called.
        // We need to shorten waypoints by half the arrow height for the path rendering.
        if self.style.arrow_start && n >= 2 {
            let p1 = self.waypoints[0].to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
            let p2 = self.waypoints[1].to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
            if let Some(arrowhead) =
                render_arrowhead_dom(p2, p1, &self.style, arrow_len_px, arrow_wid_px, ctx.use_css_vars)
            {
                nodes.push(SvgNode::Polygon(arrowhead));
            }
        }
        if self.style.arrow_end && n >= 2 {
            let p1 = self.waypoints[n - 2].to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
            let p2 = self.waypoints[n - 1].to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
            if let Some(arrowhead) =
                render_arrowhead_dom(p1, p2, &self.style, arrow_len_px, arrow_wid_px, ctx.use_css_vars)
            {
                nodes.push(SvgNode::Polygon(arrowhead));
            }
        }

        // Clone waypoints and shorten endpoints where arrows exist
        // cref: pik_chop shortens by h/2 where h = p->hArrow * pObj->sw
        // Since hArrow = arrowht/thickness and we multiply by sw (stroke width),
        // the chop amount is: (arrowht/thickness) * sw / 2 = arrowht * arrow_scale / 2
        let mut waypoints = self.waypoints.clone();
        let chop_amount = Inches(ctx.arrow_len.raw() * arrow_scale / 2.0);

        if self.style.arrow_start && waypoints.len() >= 2 {
            chop_waypoint_start(&mut waypoints, chop_amount);
        }
        if self.style.arrow_end && waypoints.len() >= 2 {
            chop_waypoint_end(&mut waypoints, chop_amount);
        }

        // cref: splineRender (pikchr.c:1716-1718) - if n<3 or r<=0, use lineRender
        let path_data = if waypoints.len() < 3 || self.radius.raw() <= 0.0 {
            create_line_path(&waypoints, ctx.scaler, ctx.offset_x, ctx.max_y)
        } else {
            create_spline_path(&waypoints, ctx.scaler, ctx.offset_x, ctx.max_y, self.radius)
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
        // Only expand by waypoints if visible (not invisible and stroke width non-negative)
        if !self.style.invisible && self.style.stroke_width.0 >= 0.0 {
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

    /// Expand bounds WITHOUT arrowheads - used for computing sublist width/height
    /// cref: pikchr.y:1757-1761 - sublist bbox uses children's pObj->bbox (no arrowheads)
    fn expand_core_bounds(&self, bounds: &mut BoundingBox) {
        // Only expand by waypoints if visible (not invisible and stroke width non-negative)
        if !self.style.invisible && self.style.stroke_width.0 >= 0.0 {
            for pt in &self.waypoints {
                bounds.expand_point(*pt);
            }
        }
        // NOTE: Arrowhead expansion is intentionally omitted here
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

    // cref: dotCheck (pikchr.c:4042-4047)
    // C sets w = h = 0 for dots, so ptEnter = ptExit = ptAt (center)
    fn start(&self) -> PointIn {
        self.center
    }

    fn end(&self) -> PointIn {
        self.center
    }

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: dotRender (pikchr.c:3934) - dots are always filled, no sw check needed
        // But for consistency with other shapes, we still check for negative thickness
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        let center_svg = self.center.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
        let r = ctx.scaler.px(self.radius);

        tracing::debug!(
            fill = %self.style.fill,
            stroke = %self.style.stroke,
            "[Rust dot render] About to render dot"
        );

        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);

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

    fn render_svg(&self, _obj: &RenderedObject, _ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        // Text rendering is handled by render_object_text in svg.rs
        // This ensures TextShape uses the same slot-based layout as other shapes
        // cref: textRender just calls pik_append_txt (pikchr.c:1746-1748)
        Vec::new()
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

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        // cref: arcRender (pikchr.c:1064) - checks pObj->sw>=0.0
        if self.style.invisible || self.style.stroke_width.0 < 0.0 {
            return nodes;
        }

        // Convert points to SVG coordinates with proper Y-flipping
        let mut start_svg = self.start.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);
        let mut end_svg = self.end.to_svg(ctx.scaler, ctx.offset_x, ctx.max_y);

        // cref: arcRender (pikchr.c:1070) - calculate control point
        let control = arc_control_point(self.style.clockwise, start_svg, end_svg);

        // Calculate arrow dimensions
        // cref: pik_draw_arrowhead (pikchr.c:4666-4667)
        // Arrow dimensions scale with object's stroke width relative to global thickness
        let arrow_scale = if ctx.thickness.raw() > 0.0 {
            self.style.stroke_width.raw() / ctx.thickness.raw()
        } else {
            1.0
        };
        let arrow_len_px = ctx.scaler.px(ctx.arrow_len) * arrow_scale;
        let arrow_wid_px = ctx.scaler.px(ctx.arrow_wid) * arrow_scale;
        let arrow_chop = arrow_len_px / 2.0;

        // cref: arcRender (pikchr.c:1071-1076) - render arrowheads first, which modifies endpoints
        // pik_draw_arrowhead calls pik_chop to shorten the endpoint by h/2
        if self.style.arrow_start {
            if let Some(arrowhead) =
                render_arrowhead_dom(control, start_svg, &self.style, arrow_len_px, arrow_wid_px, ctx.use_css_vars)
            {
                nodes.push(SvgNode::Polygon(arrowhead));
            }
            // Chop start point: shorten from control toward start by arrow_chop
            start_svg = chop_point(control, start_svg, arrow_chop);
        }
        if self.style.arrow_end {
            if let Some(arrowhead) =
                render_arrowhead_dom(control, end_svg, &self.style, arrow_len_px, arrow_wid_px, ctx.use_css_vars)
            {
                nodes.push(SvgNode::Polygon(arrowhead));
            }
            // Chop end point: shorten from control toward end by arrow_chop
            end_svg = chop_point(control, end_svg, arrow_chop);
        }

        // cref: arcRender (pikchr.c:1077-1079) - render the arc path with chopped endpoints
        // but ORIGINAL control point (m is not modified in C)
        let svg_style = build_svg_style(&self.style, ctx.scaler, ctx.dashwid, ctx.use_css_vars);
        let arc_path_data = create_arc_path_with_control(start_svg, control, end_svg);

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

    /// cref: arcCheck (pikchr.c:1040-1063) - arc bbox samples 16 points along the curve
    fn expand_bounds(&self, bounds: &mut BoundingBox) {
        // Skip bounds expansion for invisible arcs
        if self.style.invisible {
            return;
        }

        // cref: arcCheck (pikchr.c:1048-1062) - sample 16 points along the quadratic bezier
        let f = self.start;
        let t = self.end;
        // Calculate control point (in pikchr coordinates, Y-up)
        let mid = f.midpoint(t);
        let dx = t.x - f.x;
        let dy = t.y - f.y;
        let m = if self.clockwise {
            Point::new(mid.x - dy * 0.5, mid.y + dx * 0.5)
        } else {
            Point::new(mid.x + dy * 0.5, mid.y - dx * 0.5)
        };

        let sw = self.style.stroke_width;
        for i in 1..16 {
            let t1 = 0.0625 * i as f64;
            let t2 = 1.0 - t1;
            let a = t2 * t2;
            let b = 2.0 * t1 * t2;
            let c = t1 * t1;
            let x = Inches(a * f.x.0 + b * m.x.0 + c * t.x.0);
            let y = Inches(a * f.y.0 + b * m.y.0 + c * t.y.0);
            // cref: pik_bbox_addellipse - expand by stroke width
            bounds.expand_point(Point::new(x - sw, y - sw));
            bounds.expand_point(Point::new(x + sw, y + sw));
        }

        // cref: pik_bbox_add_elist (pikchr.c:4532-4542) - add arrowhead bounds at endpoints
        // wArrow = 0.5 * arrowwid (default arrowwid = 0.05")
        let w_arrow = defaults::ARROW_WID * 0.5;
        if self.style.arrow_start {
            bounds.expand_point(Point::new(f.x - w_arrow, f.y - w_arrow));
            bounds.expand_point(Point::new(f.x + w_arrow, f.y + w_arrow));
        }
        if self.style.arrow_end {
            bounds.expand_point(Point::new(t.x - w_arrow, t.y - w_arrow));
            bounds.expand_point(Point::new(t.x + w_arrow, t.y + w_arrow));
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

    /// Expand bounds WITHOUT arrowheads - used for computing sublist width/height
    /// cref: pikchr.y:1757-1761 - sublist bbox uses children's pObj->bbox (no arrowheads)
    fn expand_core_bounds(&self, bounds: &mut BoundingBox) {
        // Skip bounds expansion for invisible arcs
        if self.style.invisible {
            return;
        }

        // Sample 16 points along the quadratic bezier (same as expand_bounds)
        let f = self.start;
        let t = self.end;
        let mid = f.midpoint(t);
        let dx = t.x - f.x;
        let dy = t.y - f.y;
        let m = if self.clockwise {
            Point::new(mid.x - dy * 0.5, mid.y + dx * 0.5)
        } else {
            Point::new(mid.x + dy * 0.5, mid.y - dx * 0.5)
        };

        let sw = self.style.stroke_width;
        for i in 1..16 {
            let t1 = 0.0625 * i as f64;
            let t2 = 1.0 - t1;
            let a = t2 * t2;
            let b = 2.0 * t1 * t2;
            let c = t1 * t1;
            let x = Inches(a * f.x.0 + b * m.x.0 + c * t.x.0);
            let y = Inches(a * f.y.0 + b * m.y.0 + c * t.y.0);
            bounds.expand_point(Point::new(x - sw, y - sw));
            bounds.expand_point(Point::new(x + sw, y + sw));
        }
        // NOTE: Arrowhead expansion is intentionally omitted here
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

    fn render_svg(&self, _obj: &RenderedObject, _ctx: &ShapeRenderContext) -> Vec<SvgNode> {
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

    fn render_svg(&self, _obj: &RenderedObject, ctx: &ShapeRenderContext) -> Vec<SvgNode> {
        let mut nodes = Vec::new();

        for child in &self.children {
            let child_shape = &child.shape;
            let child_nodes = child_shape.render_svg(child, ctx);
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

    /// cref: pik_bbox_add_elist (pikchr.c:7206) - sublist bbox from children (with arrowheads)
    fn expand_bounds(&self, bounds: &mut BoundingBox) {
        for child in &self.children {
            let shape = &child.shape;
            shape.expand_bounds(bounds);
        }
    }

    /// Expand bounds WITHOUT arrowheads - used for computing sublist width/height
    /// cref: pikchr.y:1757-1761 - sublist bbox uses children's pObj->bbox (no arrowheads)
    fn expand_core_bounds(&self, bounds: &mut BoundingBox) {
        for child in &self.children {
            let shape = &child.shape;
            shape.expand_core_bounds(bounds);
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

    /// Get reference to waypoints (for Line and Spline)
    /// cref: pik_same (pikchr.c:6775-6787) - used for "same as" path copying
    pub fn waypoints(&self) -> Option<&[PointIn]> {
        match self {
            ShapeEnum::Line(s) => Some(&s.waypoints),
            ShapeEnum::Spline(s) => Some(&s.waypoints),
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
fn build_svg_style(style: &ObjectStyle, scaler: &Scaler, dashwid: Inches, use_css_vars: bool) -> SvgStyle {
    build_svg_style_full(style, scaler, dashwid, false, true, use_css_vars)
}

/// Build an SVG style with optional stroke-linejoin and fill control
/// For non-closed lines, fill should be "none" even if specified
/// cref: lineRender (pikchr.c:4228) - lines without close can't be filled
fn build_svg_style_full(
    style: &ObjectStyle,
    scaler: &Scaler,
    dashwid: Inches,
    add_linejoin: bool,
    allow_fill: bool,
    use_css_vars: bool,
) -> SvgStyle {
    // For non-closed lines, force fill to "none"
    let fill_rgb = if allow_fill {
        color_to_string(&style.fill, use_css_vars)
    } else {
        "none".to_string()
    };
    let stroke_rgb = color_to_string(&style.stroke, use_css_vars);

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
