//! Geometry functions: chop calculations and path creation

use crate::types::{Length as Inches, Point, Scaler};
use facet_svg::PathData;
use glam::{DVec2, dvec2};

use super::defaults;
use super::types::*;

/// Compass points for discrete attachment like C pikchr
#[derive(Debug, Clone, Copy)]
pub enum CompassPoint {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl CompassPoint {
    /// Determine the compass point for a normalized direction vector.
    ///
    /// C pikchr uses slope thresholds to divide 360 degrees into 8 sectors.
    /// The magic numbers 2.414 ≈ tan(67.5°) and 0.414 ≈ tan(22.5°) define
    /// the sector boundaries.
    ///
    /// # Arguments
    /// * `dir` - Direction vector where x is east (+) / west (-) and y is north (+) / south (-).
    ///           Should be pre-normalized for aspect ratio if the shape isn't square.
    pub fn from_direction(dir: DVec2) -> Self {
        let (dx, dy) = (dir.x, dir.y);
        if dx > 0.0 {
            if dy >= 2.414 * dx {
                CompassPoint::North // > 67.5 degrees
            } else if dy > 0.414 * dx {
                CompassPoint::NorthEast // 22.5 to 67.5 degrees
            } else if dy > -0.414 * dx {
                CompassPoint::East // -22.5 to 22.5 degrees
            } else if dy > -2.414 * dx {
                CompassPoint::SouthEast // -67.5 to -22.5 degrees
            } else {
                CompassPoint::South // < -67.5 degrees
            }
        } else if dx < 0.0 {
            if dy >= -2.414 * dx {
                CompassPoint::North // > 67.5 degrees
            } else if dy > -0.414 * dx {
                CompassPoint::NorthWest // 22.5 to 67.5 degrees
            } else if dy > 0.414 * dx {
                CompassPoint::West // -22.5 to 22.5 degrees
            } else if dy > 2.414 * dx {
                CompassPoint::SouthWest // -67.5 to -22.5 degrees
            } else {
                CompassPoint::South // < -67.5 degrees
            }
        } else {
            // dx == 0, vertical line
            if dy >= 0.0 {
                CompassPoint::North
            } else {
                CompassPoint::South
            }
        }
    }

    /// Determine compass point from SVG coordinates.
    ///
    /// Handles coordinate transform (SVG Y-down → compass Y-up) and
    /// aspect ratio normalization for non-square shapes.
    ///
    /// # Arguments
    /// * `center` - Shape center in SVG pixels (Y-down)
    /// * `toward` - Target point in SVG pixels (Y-down)
    /// * `half_size` - Half width (x) and half height (y) of the shape in pixels
    pub fn from_svg_direction(center: DVec2, toward: DVec2, half_size: DVec2) -> Self {
        // C pikchr scales dx by h/w to normalize the box to a square for angle calculations
        let dx = (toward.x - center.x) * half_size.y / half_size.x;
        // Coordinates are in SVG space (Y-down). Negate dy to convert to compass convention.
        let dy = -(toward.y - center.y);
        Self::from_direction(dvec2(dx, dy))
    }
}

/// Shorten a line by `amount` from both ends
/// Returns (new_start, new_end) as DVec2
pub fn chop_line(start: DVec2, end: DVec2, amount: f64) -> (DVec2, DVec2) {
    let delta = end - start;
    let len = delta.length();

    if len < amount * 2.0 {
        // Line is too short to chop, return midpoint for both
        let mid = (start + end) * 0.5;
        return (mid, mid);
    }

    // Unit vector along the line
    let unit = delta / len;

    // New endpoints
    let new_start = start + unit * amount;
    let new_end = end - unit * amount;

    (new_start, new_end)
}

pub fn apply_auto_chop_simple_line(
    scaler: &Scaler,
    obj: &RenderedObject,
    start: DVec2,
    end: DVec2,
    offset_x: Inches,
    max_y: Inches,
) -> (DVec2, DVec2) {
    if obj.start_attachment.is_none() && obj.end_attachment.is_none() {
        return (start, end);
    }

    // Pikchr auto-chop semantics:
    // - If explicit "chop" attribute is set: chop both endpoints
    // - If line connects two objects (both attachments): chop both endpoints
    // - If line has only end attachment (to Object): chop end only
    // - If line has only start attachment (from Object): do NOT chop start
    let has_explicit_chop = obj.style().chop;
    let has_both_attachments = obj.start_attachment.is_some() && obj.end_attachment.is_some();
    let should_chop_start = has_explicit_chop || has_both_attachments;
    let should_chop_end = obj.end_attachment.is_some(); // Always chop end if attached

    // Convert attachment centers to SVG pixels (Y-flipped)
    let end_center_px = obj
        .end_attachment
        .as_ref()
        .map(|info| info.center.to_svg(scaler, offset_x, max_y))
        .unwrap_or(end);

    let start_center_px = obj
        .start_attachment
        .as_ref()
        .map(|info| info.center.to_svg(scaler, offset_x, max_y))
        .unwrap_or(start);

    let mut new_start = start;
    if should_chop_start {
        if let Some(ref start_info) = obj.start_attachment {
            // Chop against start object, toward the end object's center
            if let Some(chopped) =
                chop_against_endpoint(scaler, start_info, end_center_px, offset_x, max_y)
            {
                new_start = chopped;
            }
        }
    }

    let mut new_end = end;
    if should_chop_end {
        if let Some(ref end_info) = obj.end_attachment {
            // Chop against end object, toward the start object's center
            if let Some(chopped) =
                chop_against_endpoint(scaler, end_info, start_center_px, offset_x, max_y)
            {
                new_end = chopped;
            }
        }
    }

    (new_start, new_end)
}

/// Chop against box using discrete compass points like C pikchr
fn chop_against_box_compass_point(
    center: DVec2,
    half_size: DVec2,
    corner_radius: f64,
    toward: DVec2,
) -> Option<DVec2> {
    if half_size.x <= 0.0 || half_size.y <= 0.0 {
        return None;
    }

    let compass_point = CompassPoint::from_svg_direction(center, toward, half_size);

    // Calculate corner inset for rounded corners
    // This is (1 - cos(45°)) * rad = (1 - 1/√2) * rad ≈ 0.29289 * rad
    // Matches C pikchr's boxOffset function
    let rad = corner_radius.min(half_size.x).min(half_size.y);
    let rx = if rad > 0.0 {
        0.29289321881345252392 * rad
    } else {
        0.0
    };

    // Return coordinates of the specific compass point
    // For diagonal points, adjust inward by rx to account for rounded corners
    let offset = match compass_point {
        CompassPoint::North => dvec2(0.0, -half_size.y),
        CompassPoint::NorthEast => dvec2(half_size.x - rx, -half_size.y + rx),
        CompassPoint::East => dvec2(half_size.x, 0.0),
        CompassPoint::SouthEast => dvec2(half_size.x - rx, half_size.y - rx),
        CompassPoint::South => dvec2(0.0, half_size.y),
        CompassPoint::SouthWest => dvec2(-half_size.x + rx, half_size.y - rx),
        CompassPoint::West => dvec2(-half_size.x, 0.0),
        CompassPoint::NorthWest => dvec2(-half_size.x + rx, -half_size.y + rx),
    };

    Some(center + offset)
}

/// Chop against file using discrete compass points like C pikchr
/// File has special offset: only NE corner is inset for the fold
/// From C pikchr fileOffset: rx = 0.5 * rad (clamped)
fn chop_against_file_compass_point(
    center: DVec2,
    half_size: DVec2,
    filerad: f64,
    toward: DVec2,
) -> Option<DVec2> {
    if half_size.x <= 0.0 || half_size.y <= 0.0 {
        return None;
    }

    let compass_point = CompassPoint::from_svg_direction(center, toward, half_size);

    // C pikchr fileOffset: rx = 0.5 * rad, clamped to [mn*0.25, mn] where mn = min(w2, h2)
    let mn = half_size.x.min(half_size.y);
    let mut rx = filerad;
    if rx > mn {
        rx = mn;
    }
    if rx < mn * 0.25 {
        rx = mn * 0.25;
    }
    rx *= 0.5;

    // File compass points - only NE is different (inset for fold)
    let offset = match compass_point {
        CompassPoint::North => dvec2(0.0, -half_size.y),
        CompassPoint::NorthEast => dvec2(half_size.x - rx, -half_size.y + rx), // NE: inset for fold
        CompassPoint::East => dvec2(half_size.x, 0.0),
        CompassPoint::SouthEast => dvec2(half_size.x, half_size.y), // SE: no inset
        CompassPoint::South => dvec2(0.0, half_size.y),
        CompassPoint::SouthWest => dvec2(-half_size.x, half_size.y),
        CompassPoint::West => dvec2(-half_size.x, 0.0),
        CompassPoint::NorthWest => dvec2(-half_size.x, -half_size.y),
    };

    Some(center + offset)
}

/// Chop against diamond using discrete compass points like C pikchr
/// Diamond corners (NE/SE/SW/NW) are at quarter width/height, not half
fn chop_against_diamond_compass_point(
    center: DVec2,
    half_size: DVec2,
    toward: DVec2,
) -> Option<DVec2> {
    if half_size.x <= 0.0 || half_size.y <= 0.0 {
        return None;
    }

    let compass_point = CompassPoint::from_svg_direction(center, toward, half_size);

    // Diamond: cardinal points at half, diagonals at quarter
    let quarter = half_size / 2.0;

    let offset = match compass_point {
        CompassPoint::North => dvec2(0.0, -half_size.y),
        CompassPoint::NorthEast => dvec2(quarter.x, -quarter.y),
        CompassPoint::East => dvec2(half_size.x, 0.0),
        CompassPoint::SouthEast => dvec2(quarter.x, quarter.y),
        CompassPoint::South => dvec2(0.0, half_size.y),
        CompassPoint::SouthWest => dvec2(-quarter.x, quarter.y),
        CompassPoint::West => dvec2(-half_size.x, 0.0),
        CompassPoint::NorthWest => dvec2(-quarter.x, -quarter.y),
    };

    Some(center + offset)
}

/// Chop against cylinder using discrete compass points like C pikchr
/// Cylinder has special offsets: NE/SE/SW/NW corners are inset by the ellipse radius
fn chop_against_cylinder_compass_point(
    center: DVec2,
    half_size: DVec2,
    ellipse_ry: f64,
    toward: DVec2,
) -> Option<DVec2> {
    if half_size.x <= 0.0 || half_size.y <= 0.0 {
        return None;
    }

    let compass_point = CompassPoint::from_svg_direction(center, toward, half_size);

    // Cylinder offset: h2 = h1 - rad (diagonal corners are inset by ellipse radius)
    let h2 = half_size.y - ellipse_ry;

    let offset = match compass_point {
        CompassPoint::North => dvec2(0.0, -half_size.y),
        CompassPoint::NorthEast => dvec2(half_size.x, -h2),
        CompassPoint::East => dvec2(half_size.x, 0.0),
        CompassPoint::SouthEast => dvec2(half_size.x, h2),
        CompassPoint::South => dvec2(0.0, half_size.y),
        CompassPoint::SouthWest => dvec2(-half_size.x, h2),
        CompassPoint::West => dvec2(-half_size.x, 0.0),
        CompassPoint::NorthWest => dvec2(-half_size.x, -h2),
    };

    Some(center + offset)
}

fn chop_against_endpoint(
    scaler: &Scaler,
    endpoint: &EndpointObject,
    toward: DVec2,
    offset_x: Inches,
    max_y: Inches,
) -> Option<DVec2> {
    let center = endpoint.center.to_svg(scaler, offset_x, max_y);
    let half_size = dvec2(
        scaler.px(endpoint.width / 2.0),
        scaler.px(endpoint.height / 2.0),
    );
    let corner_radius = scaler.px(endpoint.corner_radius);

    // C pikchr chop function mapping:
    // - boxChop: box, cylinder, diamond, file, oval, text
    // - circleChop: circle, dot (uses radius, continuous intersection)
    // - ellipseChop: ellipse (uses width/height, continuous intersection)
    match endpoint.class {
        ObjectClass::Circle => {
            // circleChop - continuous ray intersection with circle
            chop_against_ellipse(center, half_size, toward)
        }
        ObjectClass::Ellipse => {
            // ellipseChop - continuous ray intersection with ellipse
            chop_against_ellipse(center, half_size, toward)
        }
        ObjectClass::Box => {
            // boxChop - discrete compass points
            chop_against_box_compass_point(center, half_size, corner_radius, toward)
        }
        ObjectClass::File => {
            // fileOffset - like box but NE corner is inset for the fold
            let filerad = scaler.px(defaults::FILE_RAD);
            chop_against_file_compass_point(center, half_size, filerad, toward)
        }
        ObjectClass::Cylinder => {
            // cylinderOffset - special compass points with ellipse inset
            let cylrad = scaler.px(Inches::inches(0.075)); // default cylrad
            chop_against_cylinder_compass_point(center, half_size, cylrad, toward)
        }
        ObjectClass::Oval => {
            // boxChop with corner radius = half of smaller dimension
            let oval_radius = half_size.x.min(half_size.y);
            chop_against_box_compass_point(center, half_size, oval_radius, toward)
        }
        ObjectClass::Diamond => {
            // diamondOffset - corners at quarter width/height
            chop_against_diamond_compass_point(center, half_size, toward)
        }
        _ => None,
    }
}

fn chop_against_ellipse(center: DVec2, half_size: DVec2, toward: DVec2) -> Option<DVec2> {
    if half_size.x <= 0.0 || half_size.y <= 0.0 {
        return None;
    }

    let delta = toward - center;
    if delta.x.abs() < f64::EPSILON && delta.y.abs() < f64::EPSILON {
        return None;
    }

    let denom = (delta.x * delta.x) / (half_size.x * half_size.x)
        + (delta.y * delta.y) / (half_size.y * half_size.y);
    if denom <= 0.0 {
        return None;
    }

    let scale = 1.0 / denom.sqrt();
    Some(center + delta * scale)
}

/// Render an oval (pill shape)
/// Render a rounded box as a path (matching C pikchr output)
/// Create a rounded box path using PathData fluent API (matching C pikchr output)
pub fn create_rounded_box_path(x1: f64, y1: f64, x2: f64, y2: f64, r: f64) -> PathData {
    // C pikchr path format for rounded box:
    // Start at bottom-left corner (after radius), go clockwise
    PathData::new()
        .m(x1 + r, y2) // M: start bottom-left after radius
        .l(x2 - r, y2) // L: line to bottom-right before radius
        .a(r, r, 0.0, false, false, x2, y2 - r) // A: arc to right edge
        .l(x2, y1 + r) // L: line up to top-right before radius
        .a(r, r, 0.0, false, false, x2 - r, y1) // A: arc to top edge
        .l(x1 + r, y1) // L: line left to top-left after radius
        .a(r, r, 0.0, false, false, x1, y1 + r) // A: arc to left edge
        .l(x1, y2 - r) // L: line down to bottom-left before radius
        .a(r, r, 0.0, false, false, x1 + r, y2) // A: arc back to start
        .z() // Z: close path
}

/// Create oval (pill shape) path using PathData fluent API (matching C pikchr output)
/// Oval has fully rounded ends where rad = min(width, height) / 2
pub fn create_oval_path(x1: f64, y1: f64, x2: f64, y2: f64, rad: f64) -> PathData {
    // C pikchr path format for oval - uses 4 quarter-circle arcs:
    // Start bottom-left, line bottom, arc to east, arc to top-right,
    // line top, arc to west, arc back to bottom-left
    let cy = (y1 + y2) / 2.0; // vertical center
    PathData::new()
        .m(x1 + rad, y2) // M: start at bottom-left inner corner
        .l(x2 - rad, y2) // L: line to bottom-right inner corner
        .a(rad, rad, 0.0, false, false, x2, cy) // A: quarter arc to east edge center
        .a(rad, rad, 0.0, false, false, x2 - rad, y1) // A: quarter arc to top-right inner corner
        .l(x1 + rad, y1) // L: line to top-left inner corner
        .a(rad, rad, 0.0, false, false, x1, cy) // A: quarter arc to west edge center
        .a(rad, rad, 0.0, false, false, x1 + rad, y2) // A: quarter arc back to start
        .z() // Z: close path
}

/// Create cylinder path using PathData fluent API (matching C pikchr output)
/// C pikchr renders cylinder as single path with 3 arcs
pub fn create_cylinder_paths_with_rad(
    cx: f64,
    cy: f64,
    width: f64,
    height: f64,
    ry: f64,
) -> (PathData, PathData) {
    let rx = width / 2.0;
    let h2 = height / 2.0;

    // C pikchr cylinder path format:
    // M left,top  L left,bottom  A bottom-arc  L right,top  A top-back-arc  A top-front-arc
    let top_y = cy - h2 + ry;
    let bottom_y = cy + h2 - ry;

    // Single path with body and top ellipse (3 arcs total)
    let body_path = PathData::new()
        .m(cx - rx, top_y) // M: start at left, top edge of body
        .l(cx - rx, bottom_y) // L: line down left side
        .a(rx, ry, 0.0, false, false, cx + rx, bottom_y) // A: arc across bottom
        .l(cx + rx, top_y) // L: line up right side
        .a(rx, ry, 0.0, false, false, cx - rx, top_y) // A: arc back across top (back half)
        .a(rx, ry, 0.0, false, false, cx + rx, top_y); // A: arc across top (front half)

    // Empty bottom arc path (C pikchr doesn't render a separate bottom arc)
    let bottom_arc_path = PathData::new();

    (body_path, bottom_arc_path)
}

/// Create file paths using PathData fluent API (matching C pikchr output)
pub fn create_file_paths(
    cx: f64,
    cy: f64,
    width: f64,
    height: f64,
    fold_size: f64,
) -> (PathData, PathData) {
    // C pikchr file: fold cuts into top-right corner
    // Path goes counter-clockwise from bottom-left
    let left = cx - width / 2.0;
    let right = cx + width / 2.0;
    let top = cy - height / 2.0;
    let bottom = cy + height / 2.0;

    // Main outline path (counter-clockwise from bottom-left, matching C pikchr)
    let main_path = PathData::new()
        .m(left, bottom) // Bottom-left
        .l(right, bottom) // Bottom-right
        .l(right, top + fold_size) // Right side, stopping at fold
        .l(right - fold_size, top) // Diagonal to fold point on top
        .l(left, top) // Top-left
        .z(); // Close path

    // Fold line path (the crease inside the corner)
    let fold_path = PathData::new()
        .m(right - fold_size, top) // Start at fold point on top edge
        .l(right - fold_size, top + fold_size) // Down
        .l(right, top + fold_size); // Across to right edge

    (main_path, fold_path)
}

/// Create spline path using PathData fluent API (matching C pikchr output)
pub fn create_spline_path(
    waypoints: &[Point<Inches>],
    scaler: &Scaler,
    offset_x: Inches,
    max_y: Inches,
) -> PathData {
    if waypoints.is_empty() {
        return PathData::new();
    }

    // Convert waypoints to SVG coordinates (Y-flipped)
    let points: Vec<DVec2> = waypoints
        .iter()
        .map(|p| p.to_svg(scaler, offset_x, max_y))
        .collect();

    let mut path = PathData::new().m(points[0].x, points[0].y);

    if points.len() == 2 {
        // Just a line
        path = path.l(points[1].x, points[1].y);
    } else {
        // Use quadratic bezier curves for smoothness
        // For each segment, use the midpoint as control point
        for i in 1..points.len() {
            let prev = points[i - 1];
            let curr = points[i];

            if i == 1 {
                // First segment: quadratic from start
                let mid = (prev + curr) * 0.5;
                path = path.q(prev.x, prev.y, mid.x, mid.y);
            }

            if i < points.len() - 1 {
                // Middle segments: curve through midpoints
                let next = points[i + 1];
                let mid = (curr + next) * 0.5;
                path = path.q(curr.x, curr.y, mid.x, mid.y);
            } else {
                // Last segment: end at the final point
                let mid = (prev + curr) * 0.5;
                path = path.q(mid.x, mid.y, curr.x, curr.y);
            }
        }
    }

    path
}

/// Calculate the control point for a quadratic bezier arc.
///
/// Based on C pikchr's `arcControlPoint`, adapted for SVG coordinates (Y-down).
/// The control point is offset perpendicular to the line from start to end,
/// at a distance of half the line length.
///
/// # Arguments
/// * `clockwise` - true for clockwise arc, false for counter-clockwise (visual, in SVG space)
/// * `from` - start point (in SVG pixel coordinates, Y increases downward)
/// * `to` - end point (in SVG pixel coordinates, Y increases downward)
///
/// # Returns
/// The control point for the quadratic bezier
pub fn arc_control_point(clockwise: bool, from: DVec2, to: DVec2) -> DVec2 {
    let midpoint = (from + to) * 0.5;
    let delta = to - from;
    // Perpendicular vector: (dy, -dx) rotates CW in standard math (Y-up),
    // which appears as CCW in SVG coords (Y-down).
    // C pikchr uses Y-up internally and flips on render, so we need the
    // opposite perpendicular direction to match visually.
    let perp = DVec2::new(delta.y, -delta.x);

    if clockwise {
        midpoint + perp * 0.5
    } else {
        midpoint - perp * 0.5
    }
}

/// Create arc path using quadratic bezier (matching C pikchr output).
///
/// C pikchr renders arcs as quadratic bezier curves, NOT as SVG arc commands.
/// This gives more predictable curves that match the original implementation.
pub fn create_arc_path(start: DVec2, end: DVec2, clockwise: bool) -> PathData {
    let control = arc_control_point(clockwise, start, end);

    PathData::new()
        .m(start.x, start.y)
        .q(control.x, control.y, end.x, end.y)
}
