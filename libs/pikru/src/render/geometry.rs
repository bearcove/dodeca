//! Geometry functions: chop calculations and path creation

use crate::types::{Length as Inches, Point, Scaler};
use facet_format_svg::PathData;
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
    ///   Should be pre-normalized for aspect ratio if the shape isn't square.
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
        0.292_893_218_813_452_54 * rad
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
        ClassName::Circle => {
            // circleChop - continuous ray intersection with circle
            chop_against_ellipse(center, half_size, toward)
        }
        ClassName::Ellipse => {
            // ellipseChop - continuous ray intersection with ellipse
            chop_against_ellipse(center, half_size, toward)
        }
        ClassName::Box => {
            // boxChop - discrete compass points
            chop_against_box_compass_point(center, half_size, corner_radius, toward)
        }
        ClassName::File => {
            // fileOffset - like box but NE corner is inset for the fold
            let filerad = scaler.px(defaults::FILE_RAD);
            chop_against_file_compass_point(center, half_size, filerad, toward)
        }
        ClassName::Cylinder => {
            // cylinderOffset - special compass points with ellipse inset
            let cylrad = scaler.px(Inches::inches(0.075)); // default cylrad
            chop_against_cylinder_compass_point(center, half_size, cylrad, toward)
        }
        ClassName::Oval => {
            // boxChop with corner radius = half of smaller dimension
            let oval_radius = half_size.x.min(half_size.y);
            chop_against_box_compass_point(center, half_size, oval_radius, toward)
        }
        ClassName::Diamond => {
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
/// cref: boxRender (oval uses same render function as box with rad > 0)
pub fn create_oval_path(x1: f64, y1: f64, x2: f64, y2: f64, rad: f64) -> PathData {
    // IMPORTANT: The path must go COUNTER-CLOCKWISE with sweep-flag=0 for arcs
    // to curve inward. C starts at bottom-left and goes: right along bottom,
    // up right side, left along top, down left side.
    //
    // SVG coordinates: y1 = top (smaller y), y2 = bottom (larger y)
    // C variable mapping (after Y-flip to SVG coords):
    //   x0 = x1 (left edge), x3 = x2 (right edge)
    //   xi1 = x1 + rad (inner left), xi2 = x2 - rad (inner right)
    //   y0_svg = y2 (bottom in SVG), y3_svg = y1 (top in SVG)
    //   yi1 = y2 - rad (inner bottom), yi2 = y1 + rad (inner top)
    let xi1 = x1 + rad; // inner left x
    let xi2 = x2 - rad; // inner right x
    let yi_bottom = y2 - rad; // inner bottom y
    let yi_top = y1 + rad; // inner top y

    // C pikchr uses `>` comparisons (e.g., `if(x2>x1)`) to decide whether to emit
    // line commands between arcs. Due to floating-point precision issues in C,
    // these comparisons can return true even when the values are mathematically equal,
    // resulting in zero-length lines being emitted (e.g., L103.306,171.792 right after
    // an arc that ends at the same point). To match C's output exactly, we use the
    // same comparison logic without an epsilon tolerance.
    // cref: boxRender (pikchr.y:1211-1222)

    let mut path = PathData::new();
    path = path.m(xi1, y2); // Start at bottom-left inner corner

    // Bottom edge (horizontal line) - only if x2 > x1
    if xi2 > xi1 {
        path = path.l(xi2, y2);
    }

    // Bottom-right corner arc (going up)
    path = path.a(rad, rad, 0.0, false, false, x2, yi_bottom);

    // Right edge (vertical line going up) - only if y2 > y1
    // Note: C's y2>y1 becomes yi_bottom>yi_top in our coordinate system
    if yi_bottom > yi_top {
        path = path.l(x2, yi_top);
    }

    // Top-right corner arc (going left)
    path = path.a(rad, rad, 0.0, false, false, xi2, y1);

    // Top edge (horizontal line going left) - only if x2 > x1
    if xi2 > xi1 {
        path = path.l(xi1, y1);
    }

    // Top-left corner arc (going down)
    path = path.a(rad, rad, 0.0, false, false, x1, yi_top);

    // Left edge (vertical line going down) - only if y2 > y1
    if yi_bottom > yi_top {
        path = path.l(x1, yi_bottom);
    }

    // Bottom-left corner arc back to start
    path = path.a(rad, rad, 0.0, false, false, xi1, y2);
    path = path.z();

    path
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

/// Create a simple line path from waypoints.
/// cref: lineRender fallback for splines with < 3 waypoints (pikchr.c:1717)
pub fn create_line_path(
    waypoints: &[Point<Inches>],
    scaler: &Scaler,
    offset_x: Inches,
    max_y: Inches,
) -> PathData {
    if waypoints.is_empty() {
        return PathData::new();
    }

    let points: Vec<DVec2> = waypoints
        .iter()
        .map(|p| p.to_svg(scaler, offset_x, max_y))
        .collect();

    let mut path = PathData::new().m(points[0].x, points[0].y);
    for p in points.iter().skip(1) {
        path = path.l(p.x, p.y);
    }
    path
}

/// Calculate a point along the line from `from` to `to` that is `r` units
/// prior to reaching `to`, except if the path is less than 2*r total,
/// return the midpoint.
/// cref: radiusMidpoint (pikchr.c:1662-1679)
///
/// Returns (midpoint, is_mid) where is_mid=true if radius was clamped to midpoint.
fn radius_midpoint(from: DVec2, to: DVec2, r: f64) -> (DVec2, bool) {
    let delta = to - from;
    let dist = delta.length();

    if dist <= 0.0 {
        return (to, false);
    }

    let dir = delta / dist;

    if r > 0.5 * dist {
        // Radius is too large - clamp to midpoint
        let mid = (from + to) * 0.5;
        (mid, true)
    } else {
        // Go from `to` back toward `from` by distance r
        let m = to - dir * r;
        (m, false)
    }
}

/// Create spline path using the C pikchr radiusPath algorithm.
/// cref: radiusPath (pikchr.c:1680-1711)
///
/// The algorithm:
/// 1. Move to first point
/// 2. Line to a point `r` before the second point (first midpoint)
/// 3. For each interior vertex:
///    - Quadratic bezier with vertex as control point, next midpoint as end
///    - If the radius didn't clamp to midpoint, add a line segment
/// 4. Line to the last point
pub fn create_spline_path(
    waypoints: &[Point<Inches>],
    scaler: &Scaler,
    offset_x: Inches,
    max_y: Inches,
    radius: Inches,
) -> PathData {
    if waypoints.is_empty() {
        return PathData::new();
    }

    // Convert waypoints to SVG coordinates (Y-flipped)
    let a: Vec<DVec2> = waypoints
        .iter()
        .map(|p| p.to_svg(scaler, offset_x, max_y))
        .collect();

    let n = a.len();
    // cref: radiusPath uses pObj->rad which is in inches, convert to pixels
    let r = scaler.px(radius);

    // cref: radiusPath (pikchr.c:1689) - M a[0]
    let mut path = PathData::new().m(a[0].x, a[0].y);

    // cref: radiusPath (pikchr.c:1690-1691) - L to first midpoint
    let (m, _) = radius_midpoint(a[0], a[1], r);
    path = path.l(m.x, m.y);

    // cref: radiusPath (pikchr.c:1692-1701) - loop through interior vertices
    // Note: C uses iLast = bClose ? n : n-1, we don't support bClose for splines
    let i_last = n - 1;

    for i in 1..i_last {
        // an = next point (wrapping for closed paths, but we don't close splines)
        let an = a[i + 1];

        // cref: radiusPath (pikchr.c:1694-1696) - Q with vertex as control, midpoint as end
        let (m, is_mid) = radius_midpoint(an, a[i], r);
        path = path.q(a[i].x, a[i].y, m.x, m.y);

        // cref: radiusPath (pikchr.c:1697-1700) - if radius didn't clamp, add line to next midpoint
        if !is_mid {
            let (m2, _) = radius_midpoint(a[i], an, r);
            path = path.l(m2.x, m2.y);
        }
    }

    // cref: radiusPath (pikchr.c:1702) - L to final point
    path = path.l(a[n - 1].x, a[n - 1].y);

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

/// Create arc path with a pre-calculated control point.
/// cref: arcRender (pikchr.c:1077-1079) - uses original control point with chopped endpoints
///
/// Use this when endpoints have been chopped for arrows but the control point
/// should remain calculated from the original (unchopped) endpoints.
pub fn create_arc_path_with_control(start: DVec2, control: DVec2, end: DVec2) -> PathData {
    PathData::new()
        .m(start.x, start.y)
        .q(control.x, control.y, end.x, end.y)
}

// ============================================================================
// Construction-time chopping (inches, pikchr coordinates)
// ============================================================================
//
// These functions implement chopping during object construction, matching C pikchr's
// pik_autochop behavior. They work in pikchr coordinates (Y-up, inches) rather than
// SVG coordinates (Y-down, pixels).
//
// cref: pik_autochop (pikchr.c:4272-4279)
// cref: boxChop (pikchr.c:1132-1169)
// cref: circleChop (pikchr.c:1254-1264)
// cref: ellipseChop (pikchr.c:1451-1466)

/// Chop a line endpoint against an attached object.
///
/// This is the inch-based equivalent of C's `pik_autochop`. It modifies the
/// endpoint to be on the edge of the attached object rather than at its center.
///
/// # Arguments
/// * `from` - The other endpoint of the line (used to determine direction)
/// * `to` - The endpoint to chop (will be modified if chopping succeeds)
/// * `endpoint` - Information about the object to chop against
///
/// # Returns
/// The chopped point, or `to` unchanged if chopping is not applicable.
///
/// cref: pik_autochop (pikchr.c:4272-4279)
pub fn autochop_inches(from: PointIn, to: PointIn, endpoint: &EndpointObject) -> PointIn {
    // Convert to DVec2 for math (pikchr coords: Y-up)
    let from_vec = dvec2(from.x.raw(), from.y.raw());

    let chopped = match endpoint.class {
        // boxChop is used by: box, cylinder, diamond, file, oval, text
        ClassName::Box
        | ClassName::Cylinder
        | ClassName::Diamond
        | ClassName::File
        | ClassName::Oval
        | ClassName::Text => box_chop_inches(endpoint, from_vec),
        // circleChop is used by: circle, dot
        ClassName::Circle | ClassName::Dot => circle_chop_inches(endpoint, from_vec),
        // ellipseChop is used by: ellipse
        ClassName::Ellipse => ellipse_chop_inches(endpoint, from_vec),
        // Lines, arrows, splines, moves, arcs, sublists have no xChop
        _ => None,
    };

    match chopped {
        Some(pt) => PointIn::new(Inches(pt.x), Inches(pt.y)),
        None => to,
    }
}

/// Box chopping in inches (pikchr coordinates).
///
/// Uses discrete compass points to find the edge point, matching C's boxChop.
/// The direction is determined by the angle from the object center to `toward`,
/// normalized by aspect ratio.
///
/// cref: boxChop (pikchr.c:1132-1169)
fn box_chop_inches(obj: &EndpointObject, toward: DVec2) -> Option<DVec2> {
    let center = dvec2(obj.center.x.raw(), obj.center.y.raw());
    let w = obj.width.raw();
    let h = obj.height.raw();

    if w <= 0.0 || h <= 0.0 {
        return Some(center);
    }

    // C pikchr normalizes dx by h/w for aspect ratio
    // cref: boxChop line 1138: dx = (pPt->x - pObj->ptAt.x)*pObj->h/pObj->w
    let dx = (toward.x - center.x) * h / w;
    let dy = toward.y - center.y;

    // Determine compass point using the normalized direction
    let cp = CompassPoint::from_direction(dvec2(dx, dy));

    // Get offset for this compass point using the appropriate xOffset function
    let offset = match obj.class {
        ClassName::Box | ClassName::Text => box_offset_inches(obj, cp),
        ClassName::Cylinder => cylinder_offset_inches(obj, cp),
        ClassName::Diamond => diamond_offset_inches(obj, cp),
        ClassName::File => file_offset_inches(obj, cp),
        ClassName::Oval => oval_offset_inches(obj, cp),
        _ => box_offset_inches(obj, cp), // Default to box
    };

    Some(center + offset)
}

/// Circle chopping in inches (pikchr coordinates).
///
/// Uses ray intersection with the circle to find the edge point.
/// This is a continuous calculation, not discrete like boxChop.
///
/// cref: circleChop (pikchr.c:1254-1264)
fn circle_chop_inches(obj: &EndpointObject, toward: DVec2) -> Option<DVec2> {
    let center = dvec2(obj.center.x.raw(), obj.center.y.raw());
    // Circle uses width/2 as radius (w = h = 2*rad for circles)
    let rad = obj.width.raw() / 2.0;

    let dx = toward.x - center.x;
    let dy = toward.y - center.y;
    let dist = (dx * dx + dy * dy).sqrt();

    // cref: circleChop line 1259: if( dist<pObj->rad || dist<=0 ) return pObj->ptAt
    if dist < rad || dist <= 0.0 {
        return Some(center);
    }

    // cref: circleChop lines 1260-1261
    Some(dvec2(
        center.x + dx * rad / dist,
        center.y + dy * rad / dist,
    ))
}

/// Ellipse chopping in inches (pikchr coordinates).
///
/// Uses ray intersection with the ellipse to find the edge point.
/// The calculation normalizes by aspect ratio to handle non-circular ellipses.
///
/// cref: ellipseChop (pikchr.c:1451-1466)
fn ellipse_chop_inches(obj: &EndpointObject, toward: DVec2) -> Option<DVec2> {
    let center = dvec2(obj.center.x.raw(), obj.center.y.raw());
    let w = obj.width.raw();
    let h = obj.height.raw();

    if w <= 0.0 || h <= 0.0 {
        return Some(center);
    }

    let dx = toward.x - center.x;
    let dy = toward.y - center.y;

    // cref: ellipseChop lines 1458-1460
    let s = h / w;
    let dq = dx * s;
    let dist = (dq * dq + dy * dy).sqrt();

    // cref: ellipseChop line 1461: if( dist<pObj->h ) return pObj->ptAt
    if dist < h {
        return Some(center);
    }

    // cref: ellipseChop lines 1462-1463
    Some(dvec2(
        center.x + 0.5 * dq * h / (dist * s),
        center.y + 0.5 * dy * h / dist,
    ))
}

/// Box offset for compass point (inches).
/// cref: boxOffset (pikchr.c:1178-1213)
fn box_offset_inches(obj: &EndpointObject, cp: CompassPoint) -> DVec2 {
    let w2 = obj.width.raw() / 2.0;
    let h2 = obj.height.raw() / 2.0;
    let rad = obj.corner_radius.raw();

    // cref: boxOffset lines 1181-1183 - rx for rounded corners
    // rx = (1 - cos(45°)) * rad ≈ 0.29289 * rad
    let mn = w2.min(h2);
    let rad_clamped = rad.min(mn);
    let rx = if rad_clamped > 0.0 {
        0.292_893_218_813_452_54 * rad_clamped
    } else {
        0.0
    };

    // cref: boxOffset lines 1184-1212
    match cp {
        CompassPoint::North => dvec2(0.0, h2),
        CompassPoint::NorthEast => dvec2(w2 - rx, h2 - rx),
        CompassPoint::East => dvec2(w2, 0.0),
        CompassPoint::SouthEast => dvec2(w2 - rx, -h2 + rx),
        CompassPoint::South => dvec2(0.0, -h2),
        CompassPoint::SouthWest => dvec2(-w2 + rx, -h2 + rx),
        CompassPoint::West => dvec2(-w2, 0.0),
        CompassPoint::NorthWest => dvec2(-w2 + rx, h2 - rx),
    }
}

/// Cylinder offset for compass point (inches).
/// cref: cylinderOffset (pikchr.c:1378-1417)
fn cylinder_offset_inches(obj: &EndpointObject, cp: CompassPoint) -> DVec2 {
    let w2 = obj.width.raw() / 2.0;
    let h2 = obj.height.raw() / 2.0;
    // cref: cylinderOffset line 1380: rad = pObj->rad (default cylrad = 0.075)
    // Default cylrad is 0.075 inches
    let default_cylrad = 0.075;
    let rad = obj.corner_radius.raw().max(default_cylrad);

    // cref: cylinderOffset - h2_inner = h2 - rad (diagonal corners are inset)
    let h2_inner = h2 - rad;

    match cp {
        CompassPoint::North => dvec2(0.0, h2),
        CompassPoint::NorthEast => dvec2(w2, h2_inner),
        CompassPoint::East => dvec2(w2, 0.0),
        CompassPoint::SouthEast => dvec2(w2, -h2_inner),
        CompassPoint::South => dvec2(0.0, -h2),
        CompassPoint::SouthWest => dvec2(-w2, -h2_inner),
        CompassPoint::West => dvec2(-w2, 0.0),
        CompassPoint::NorthWest => dvec2(-w2, h2_inner),
    }
}

/// Diamond offset for compass point (inches).
/// cref: diamondOffset (pikchr.c:1432-1449)
fn diamond_offset_inches(obj: &EndpointObject, cp: CompassPoint) -> DVec2 {
    let w2 = obj.width.raw() / 2.0;
    let h2 = obj.height.raw() / 2.0;

    // cref: diamondOffset - diagonal points at quarter width/height
    let w4 = w2 / 2.0;
    let h4 = h2 / 2.0;

    match cp {
        CompassPoint::North => dvec2(0.0, h2),
        CompassPoint::NorthEast => dvec2(w4, h4),
        CompassPoint::East => dvec2(w2, 0.0),
        CompassPoint::SouthEast => dvec2(w4, -h4),
        CompassPoint::South => dvec2(0.0, -h2),
        CompassPoint::SouthWest => dvec2(-w4, -h4),
        CompassPoint::West => dvec2(-w2, 0.0),
        CompassPoint::NorthWest => dvec2(-w4, h4),
    }
}

/// File offset for compass point (inches).
/// cref: fileOffset (pikchr.c:1491-1540)
fn file_offset_inches(obj: &EndpointObject, cp: CompassPoint) -> DVec2 {
    let w2 = obj.width.raw() / 2.0;
    let h2 = obj.height.raw() / 2.0;

    // cref: fileOffset lines 1493-1500
    // rx = 0.5 * rad, clamped to [mn*0.25, mn] where mn = min(w2, h2)
    let mn = w2.min(h2);
    let mut rx = defaults::FILE_RAD.raw();
    if rx > mn {
        rx = mn;
    }
    if rx < mn * 0.25 {
        rx = mn * 0.25;
    }
    rx *= 0.5;

    // cref: fileOffset - only NE is inset for the fold
    match cp {
        CompassPoint::North => dvec2(0.0, h2),
        CompassPoint::NorthEast => dvec2(w2 - rx, h2 - rx), // NE: inset for fold
        CompassPoint::East => dvec2(w2, 0.0),
        CompassPoint::SouthEast => dvec2(w2, -h2), // SE: no inset
        CompassPoint::South => dvec2(0.0, -h2),
        CompassPoint::SouthWest => dvec2(-w2, -h2),
        CompassPoint::West => dvec2(-w2, 0.0),
        CompassPoint::NorthWest => dvec2(-w2, h2),
    }
}

/// Oval offset for compass point (inches).
/// cref: boxOffset with rad = min(w2, h2) (oval uses boxOffset in C)
fn oval_offset_inches(obj: &EndpointObject, cp: CompassPoint) -> DVec2 {
    let w2 = obj.width.raw() / 2.0;
    let h2 = obj.height.raw() / 2.0;
    // Oval uses full rounding radius = min of half dimensions
    let rad = w2.min(h2);

    // rx = (1 - cos(45°)) * rad ≈ 0.29289 * rad
    let rx = 0.292_893_218_813_452_54 * rad;

    match cp {
        CompassPoint::North => dvec2(0.0, h2),
        CompassPoint::NorthEast => dvec2(w2 - rx, h2 - rx),
        CompassPoint::East => dvec2(w2, 0.0),
        CompassPoint::SouthEast => dvec2(w2 - rx, -h2 + rx),
        CompassPoint::South => dvec2(0.0, -h2),
        CompassPoint::SouthWest => dvec2(-w2 + rx, -h2 + rx),
        CompassPoint::West => dvec2(-w2, 0.0),
        CompassPoint::NorthWest => dvec2(-w2 + rx, h2 - rx),
    }
}
