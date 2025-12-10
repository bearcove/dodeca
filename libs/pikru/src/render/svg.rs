//! SVG generation

use super::shapes::Shape;
use crate::types::{Length as Inches, Point, Scaler};
use facet_svg::facet_xml::SerializeOptions;
use facet_svg::{
    Circle, Color, Ellipse, Path, PathData, Polygon, Svg, SvgNode, SvgStyle, Text, facet_xml,
    fmt_num,
};
use glam::{DVec2, dvec2};
use time::{OffsetDateTime, format_description};

use super::context::RenderContext;
use super::defaults;
use super::eval::{get_length, get_scalar};
use super::geometry::{
    apply_auto_chop_simple_line, arc_control_point, chop_line, create_arc_path,
    create_cylinder_paths_with_rad, create_file_paths, create_oval_path, create_rounded_box_path,
    create_spline_path,
};
use super::types::*;

/// Generate a UTC timestamp in YYYYMMDDhhmmss format for data-pikchr-date attribute
fn utc_timestamp() -> String {
    let now = OffsetDateTime::now_utc();
    let format = format_description::parse("[year][month][day][hour][minute][second]")
        .expect("valid format");
    now.format(&format).unwrap_or_default()
}

/// Convert a color name to rgb() format like C pikchr
pub fn color_to_rgb(color: &str) -> String {
    color.parse::<crate::types::Color>().unwrap().to_rgb_string()
}

/// Build a polyline path from a series of points.
/// Converts points to SVG coordinates and creates M/L commands.
fn build_polyline_path(
    points: &[Point<Inches>],
    scaler: &Scaler,
    offset_x: Inches,
    max_y: Inches,
) -> PathData {
    let mut path_data = PathData::new();
    for (i, p) in points.iter().enumerate() {
        let svg_pt = p.to_svg(scaler, offset_x, max_y);
        if i == 0 {
            path_data = path_data.m(svg_pt.x, svg_pt.y);
        } else {
            path_data = path_data.l(svg_pt.x, svg_pt.y);
        }
    }
    path_data
}

/// Generate SVG from render context
// cref: pik_render (pikchr.c:7253) - main SVG output function
pub fn generate_svg(ctx: &RenderContext) -> Result<String, miette::Report> {
    let margin_base = get_length(ctx, "margin", defaults::MARGIN);
    let left_margin = get_length(ctx, "leftmargin", 0.0);
    let right_margin = get_length(ctx, "rightmargin", 0.0);
    let top_margin = get_length(ctx, "topmargin", 0.0);
    let bottom_margin = get_length(ctx, "bottommargin", 0.0);
    let thickness = get_length(ctx, "thickness", defaults::STROKE_WIDTH.raw());

    let margin = margin_base + thickness;
    let scale = get_scalar(ctx, "scale", 1.0);
    let r_scale = scale * 144.0; // match pikchr.c rScale - use scale factor for coordinates
    // C pikchr uses r_scale for coordinate conversion
    let scaler = Scaler::try_new(r_scale)
        .map_err(|e| miette::miette!("invalid scale value {}: {}", r_scale, e))?;
    let arrow_ht = Inches(get_length(ctx, "arrowht", 0.08));
    let arrow_wid = Inches(get_length(ctx, "arrowwid", 0.06));
    let dashwid = Inches(get_length(ctx, "dashwid", 0.05));
    let arrow_len_px = scaler.len(arrow_ht);
    let arrow_wid_px = scaler.len(arrow_wid);
    let mut bounds = ctx.bounds;

    // Debug: compare with C bbox
    tracing::debug!(
        sw_x = bounds.min.x.0,
        sw_y = bounds.min.y.0,
        ne_x = bounds.max.x.0,
        ne_y = bounds.max.y.0,
        "bbox before margin"
    );

    // Expand bounds with margins first (before checking for zero dimensions)
    bounds.max.x += Inches(margin + right_margin);
    bounds.max.y += Inches(margin + top_margin);
    bounds.min.x -= Inches(margin + left_margin);
    bounds.min.y -= Inches(margin + bottom_margin);

    // Use a small epsilon for zero-dimension check (thin lines are valid)
    let min_dim = Inches(0.01);
    let view_width = bounds.width().max(min_dim);
    let view_height = bounds.height().max(min_dim);
    let offset_x = -bounds.min.x;
    // Y-flip: C pikchr uses `y = bbox.ne.y - y` to flip from Y-up to SVG Y-down
    // For to_svg(), we pass max_y = bounds.max.y so that:
    //   svg_y = scaler.px(max_y - point.y) which gives the correct flipped coordinate
    let max_y = bounds.max.y;

    tracing::debug!(
        bounds_min_x = bounds.min.x.0,
        bounds_min_y = bounds.min.y.0,
        bounds_max_x = bounds.max.x.0,
        bounds_max_y = bounds.max.y.0,
        offset_x = offset_x.0,
        max_y = max_y.0,
        "generate_svg bounds"
    );

    // Build SVG DOM
    let mut svg_children: Vec<SvgNode> = Vec::new();

    // SVG header - C pikchr only adds width/height when scale != 1.0
    let viewbox_width = scaler.px(view_width);
    let viewbox_height = scaler.px(view_height);
    let _timestamp = utc_timestamp();

    // Create the main SVG element
    let viewbox = format!("0 0 {} {}", fmt_num(viewbox_width), fmt_num(viewbox_height));
    let mut svg = Svg {
        width: None,
        height: None,
        view_box: Some(viewbox),
        children: Vec::new(),
    };

    // C pikchr: when scale != 1.0, display width = viewBox width * scale
    let is_scaled = scale < 0.99 || scale > 1.01;
    if is_scaled {
        let display_width = (viewbox_width * scale).round();
        let display_height = (viewbox_height * scale).round();
        svg.width = Some(display_width.to_string());
        svg.height = Some(display_height.to_string());
    }

    // Arrowheads are now rendered inline as polygon elements (matching C pikchr)

    // Render each object
    for obj in &ctx.object_list {
        // Convert from pikchr coordinates (Y-up) to SVG pixels (Y-down)
        let center = obj.center().to_svg(&scaler, offset_x, max_y);
        let start = obj.start().to_svg(&scaler, offset_x, max_y);
        let end = obj.end().to_svg(&scaler, offset_x, max_y);

        let svg_style = create_svg_style(&obj.style(), &scaler, dashwid);

        // Render shape (skip if invisible, but still render text below)
        if !obj.style().invisible {
            match obj.class() {
                ClassName::Box => {
                    let half_w = scaler.px(obj.width() / 2.0);
                    let half_h = scaler.px(obj.height() / 2.0);
                    let x1 = center.x - half_w;
                    let x2 = center.x + half_w;
                    let y1 = center.y - half_h;
                    let y2 = center.y + half_h;

                    let path_data = if obj.style().corner_radius > Inches::ZERO {
                        // Rounded box using proper path with arcs
                        let r = scaler.px(obj.style().corner_radius);
                        create_rounded_box_path(x1, y1, x2, y2, r)
                    } else {
                        // Regular box: C pikchr renders boxes as path: M x1,y2 L x2,y2 L x2,y1 L x1,y1 Z
                        // (starts at bottom-left, goes clockwise)
                        PathData::new()
                            .m(x1, y2) // Start at bottom-left
                            .l(x2, y2) // Line to bottom-right
                            .l(x2, y1) // Line to top-right
                            .l(x1, y1) // Line to top-left
                            .z() // Close path
                    };

                    let path = Path {
                        d: Some(path_data),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style,
                    };
                    svg_children.push(SvgNode::Path(path));
                }
                ClassName::Circle => {
                    let r = scaler.px(obj.width() / 2.0);
                    let circle = Circle {
                        cx: Some(center.x),
                        cy: Some(center.y),
                        r: Some(r),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style,
                    };
                    svg_children.push(SvgNode::Circle(circle));
                }
                ClassName::Dot => {
                    // Dot is a small filled circle
                    let r = scaler.px(obj.width() / 2.0);
                    let fill = if obj.style().fill == "none" {
                        &obj.style().stroke
                    } else {
                        &obj.style().fill
                    };
                    let mut dot_style = SvgStyle::new();
                    dot_style
                        .properties
                        .insert("fill".to_string(), fill.to_string());
                    let circle = Circle {
                        cx: Some(center.x),
                        cy: Some(center.y),
                        r: Some(r),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: dot_style,
                    };
                    svg_children.push(SvgNode::Circle(circle));
                }
                ClassName::Ellipse => {
                    let rx = scaler.px(obj.width() / 2.0);
                    let ry = scaler.px(obj.height() / 2.0);
                    let ellipse = Ellipse {
                        cx: Some(center.x),
                        cy: Some(center.y),
                        rx: Some(rx),
                        ry: Some(ry),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style.clone(),
                    };
                    svg_children.push(SvgNode::Ellipse(ellipse));
                }
                ClassName::Oval => {
                    // Oval is a pill shape (rounded rectangle with fully rounded ends)
                    // C pikchr renders as a path, not a rect
                    let width = scaler.px(obj.width());
                    let height = scaler.px(obj.height());
                    let rad = height.min(width) / 2.0; // radius is half of smaller dimension
                    let x1 = center.x - width / 2.0;
                    let x2 = center.x + width / 2.0;
                    let y1 = center.y - height / 2.0;
                    let y2 = center.y + height / 2.0;

                    // Create path like C pikchr: start at bottom-left inner, go clockwise
                    let path_data = create_oval_path(x1, y1, x2, y2, rad);
                    let path = Path {
                        d: Some(path_data),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style.clone(),
                    };
                    svg_children.push(SvgNode::Path(path));
                }
                ClassName::Cylinder => {
                    // Cylinder: single path with 3 arcs matching C pikchr
                    // Uses cylrad for the ellipse vertical radius (default 0.075")
                    let width = scaler.px(obj.width());
                    let height = scaler.px(obj.height());
                    let cylrad = get_length(ctx, "cylrad", 0.075);
                    let ry = scaler.px(Inches::inches(cylrad));

                    let (body_path, _) =
                        create_cylinder_paths_with_rad(center.x, center.y, width, height, ry);

                    let body = Path {
                        d: Some(body_path),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style.clone(),
                    };
                    svg_children.push(SvgNode::Path(body));
                }
                ClassName::File => {
                    // File: proper path-based rendering with fold matching C pikchr
                    let width = scaler.px(obj.width());
                    let height = scaler.px(obj.height());
                    let filerad = scaler.px(defaults::FILE_RAD);

                    let (main_path, fold_path) =
                        create_file_paths(center.x, center.y, width, height, filerad);

                    // Main outline path
                    let main = Path {
                        d: Some(main_path),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style.clone(),
                    };
                    svg_children.push(SvgNode::Path(main));

                    // Fold line (stroke only, no fill) - uses same stroke as main shape
                    let mut fold_style = svg_style.clone();
                    fold_style
                        .properties
                        .insert("fill".to_string(), "none".to_string());

                    let fold = Path {
                        d: Some(fold_path),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: fold_style,
                    };
                    svg_children.push(SvgNode::Path(fold));
                }
                ClassName::Diamond => {
                    // Diamond: rotated square/rhombus with vertices at edges
                    // C pikchr uses path: M left,cy L cx,bottom L right,cy L cx,top Z
                    let half_w = scaler.px(obj.width() / 2.0);
                    let half_h = scaler.px(obj.height() / 2.0);
                    let left = center.x - half_w;
                    let right = center.x + half_w;
                    let top = center.y - half_h;
                    let bottom = center.y + half_h;
                    let path_data = PathData::new()
                        .m(left, center.y) // Left vertex
                        .l(center.x, bottom) // Bottom vertex
                        .l(right, center.y) // Right vertex
                        .l(center.x, top) // Top vertex
                        .z(); // Close path
                    let path = Path {
                        d: Some(path_data),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style,
                    };
                    svg_children.push(SvgNode::Path(path));
                }
                ClassName::Line | ClassName::Arrow => {
                    // Auto-chop always applies for object-attached endpoints (trims to boundary)
                    // The explicit "chop" attribute is for additional user-requested shortening
                    let (draw_start, draw_end) = if obj.waypoints().unwrap_or(&[]).len() <= 2 {
                        apply_auto_chop_simple_line(
                            &scaler, obj, start, end, offset_x, max_y,
                        )
                    } else {
                        (start, end)
                    };

                    if obj.waypoints().unwrap_or(&[]).len() <= 2 {
                        // Simple line - render as <path> (matching C pikchr)
                        // First render arrowhead polygon if needed (rendered before line, like C)
                        if obj.style().arrow_end {
                            if let Some(arrowhead) = render_arrowhead_dom(
                                draw_start,
                                draw_end,
                                &obj.style(),
                                arrow_len_px.0,
                                arrow_wid_px.0,
                            ) {
                                svg_children.push(SvgNode::Polygon(arrowhead));
                            }
                        }
                        if obj.style().arrow_start {
                            if let Some(arrowhead) = render_arrowhead_dom(
                                draw_end,
                                draw_start,
                                &obj.style(),
                                arrow_len_px.0,
                                arrow_wid_px.0,
                            ) {
                                svg_children.push(SvgNode::Polygon(arrowhead));
                            }
                        }

                        // Chop line endpoints for arrowheads (by arrowht/2 as in C pikchr, in pixels)
                        let arrow_chop_px = arrow_len_px.0 / 2.0;
                        let (line_start, line_end) = {
                            let mut s = draw_start;
                            let mut e = draw_end;

                            if obj.style().arrow_start {
                                let (new_s, _) = chop_line(s, e, arrow_chop_px);
                                s = new_s;
                            }
                            if obj.style().arrow_end {
                                let (_, new_e) = chop_line(s, e, arrow_chop_px);
                                e = new_e;
                            }
                            (s, e)
                        };

                        // Render the line path (with chopped endpoints)
                        let line_path_data = PathData::new()
                            .m(line_start.x, line_start.y)
                            .l(line_end.x, line_end.y);
                        let line_path = Path {
                            d: Some(line_path_data),
                            fill: None,
                            stroke: None,
                            stroke_width: None,
                            stroke_dasharray: None,
                            style: svg_style,
                        };
                        svg_children.push(SvgNode::Path(line_path));
                    } else {
                        // Multi-segment polyline - chop first and last segments
                        // For polylines, use fixed chop amount (circle radius) since we don't
                        // have endpoint detection for multi-segment paths yet
                        let mut points: Vec<_> = obj.waypoints().unwrap_or(&[]).to_vec();
                        if obj.style().chop && points.len() >= 2 {
                            let chop_amount_px = scaler.px(defaults::CIRCLE_RADIUS);
                            // Chop start - convert to SVG coords, chop, then back
                            let p0 = points[0].to_svg(&scaler, offset_x, max_y);
                            let p1 = points[1].to_svg(&scaler, offset_x, max_y);
                            let (new_start, _) = chop_line(p0, p1, chop_amount_px);
                            // Store as SVG coords for later rendering
                            points[0] = Point::new(Inches(new_start.x / scaler.r_scale - offset_x.0),
                                                   Inches(max_y.0 - new_start.y / scaler.r_scale));

                            // Chop end
                            let n = points.len();
                            let pn1 = points[n - 2].to_svg(&scaler, offset_x, max_y);
                            let pn = points[n - 1].to_svg(&scaler, offset_x, max_y);
                            let (_, new_end) = chop_line(pn1, pn, chop_amount_px);
                            points[n - 1] = Point::new(Inches(new_end.x / scaler.r_scale - offset_x.0),
                                                       Inches(max_y.0 - new_end.y / scaler.r_scale));
                        }

                        if obj.style().close_path {
                            // Build closed path (no arrow chopping for closed paths)
                            let path_data = build_polyline_path(&points, &scaler, offset_x, max_y).z();
                            let path = Path {
                                d: Some(path_data),
                                fill: None,
                                stroke: None,
                                stroke_width: None,
                                stroke_dasharray: None,
                                style: svg_style,
                            };
                            svg_children.push(SvgNode::Path(path));
                        } else {
                            // TODO: Render arrowheads for polylines

                            // Chop endpoints for arrow heads (arrowht/2)
                            if (obj.style().arrow_end || obj.style().arrow_start) && points.len() >= 2 {
                                let chop_amount_px = arrow_len_px.0 / 2.0;

                                // Chop start if arrow_start
                                if obj.style().arrow_start {
                                    let p0 = points[0].to_svg(&scaler, offset_x, max_y);
                                    let p1 = points[1].to_svg(&scaler, offset_x, max_y);
                                    let (new_start, _) = chop_line(p0, p1, chop_amount_px);
                                    points[0] = Point::new(Inches(new_start.x / scaler.r_scale - offset_x.0),
                                                           Inches(max_y.0 - new_start.y / scaler.r_scale));
                                }

                                // Chop end if arrow_end
                                if obj.style().arrow_end {
                                    let n = points.len();
                                    let pn1 = points[n - 2].to_svg(&scaler, offset_x, max_y);
                                    let pn = points[n - 1].to_svg(&scaler, offset_x, max_y);
                                    let (_, new_end) = chop_line(pn1, pn, chop_amount_px);
                                    points[n - 1] = Point::new(Inches(new_end.x / scaler.r_scale - offset_x.0),
                                                               Inches(max_y.0 - new_end.y / scaler.r_scale));
                                }
                            }

                            // Now render the polyline
                            let path_data = build_polyline_path(&points, &scaler, offset_x, max_y);
                            let path = Path {
                                d: Some(path_data),
                                fill: None,
                                stroke: None,
                                stroke_width: None,
                                stroke_dasharray: None,
                                style: svg_style,
                            };
                            svg_children.push(SvgNode::Path(path));
                        }
                    }
                }
                ClassName::Spline => {
                    // Proper spline rendering with Bezier curves
                    let spline_path_data = create_spline_path(obj.waypoints().unwrap_or(&[]), &scaler, offset_x, max_y);

                    // Render arrowheads first (before path, like C pikchr)
                    let n = obj.waypoints().unwrap_or(&[]).len();
                    if obj.style().arrow_end && n >= 2 {
                        let p1 = obj.waypoints().unwrap_or(&[])[n - 2].to_svg(&scaler, offset_x, max_y);
                        let p2 = obj.waypoints().unwrap_or(&[])[n - 1].to_svg(&scaler, offset_x, max_y);
                        if let Some(arrowhead) = render_arrowhead_dom(
                            p1,
                            p2,
                            &obj.style(),
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        ) {
                            svg_children.push(SvgNode::Polygon(arrowhead));
                        }
                    }
                    if obj.style().arrow_start && n >= 2 {
                        let p1 = obj.waypoints().unwrap_or(&[])[0].to_svg(&scaler, offset_x, max_y);
                        let p2 = obj.waypoints().unwrap_or(&[])[1].to_svg(&scaler, offset_x, max_y);
                        if let Some(arrowhead) = render_arrowhead_dom(
                            p2,
                            p1,
                            &obj.style(),
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        ) {
                            svg_children.push(SvgNode::Polygon(arrowhead));
                        }
                    }

                    let spline_path = Path {
                        d: Some(spline_path_data),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style,
                    };
                    svg_children.push(SvgNode::Path(spline_path));
                }
                ClassName::Move => {
                    // Move: invisible - just advances the cursor, renders nothing
                }

                ClassName::Arc => {
                    // Arc rendering using quadratic bezier (matching C pikchr)
                    // start and end are already in SVG coordinates (Y-flipped)
                    let control = arc_control_point(obj.style().clockwise, start, end);
                    let arc_path_data = create_arc_path(start, end, obj.style().clockwise);

                    // Render arrowheads first (before path, like C pikchr)
                    // For arcs, arrowheads point from control point toward the endpoint
                    if obj.style().arrow_start {
                        // Arrow at start: from control point toward start
                        if let Some(arrowhead) = render_arrowhead_dom(
                            control,
                            start,
                            &obj.style(),
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        ) {
                            svg_children.push(SvgNode::Polygon(arrowhead));
                        }
                    }
                    if obj.style().arrow_end {
                        // Arrow at end: from control point toward end
                        if let Some(arrowhead) = render_arrowhead_dom(
                            control,
                            end,
                            &obj.style(),
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        ) {
                            svg_children.push(SvgNode::Polygon(arrowhead));
                        }
                    }

                    let arc_path = Path {
                        d: Some(arc_path_data),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style,
                    };
                    svg_children.push(SvgNode::Path(arc_path));
                }
                ClassName::Text => {
                    // Multi-line text: distribute lines vertically around center
                    // C pikchr stacks lines with charht spacing, centered on the object center
                    let charht_px = scaler.px(Inches(defaults::FONT_SIZE));
                    let line_count = obj.text().len();
                    // Start Y: center minus half of total height, plus half charht for first line center
                    // Total height = (line_count - 1) * charht, distributed evenly around center
                    let start_y = center.y - (line_count as f64 - 1.0) * charht_px / 2.0;

                    for (i, positioned_text) in obj.text().iter().enumerate() {
                        // Determine text anchor based on ljust/rjust
                        let anchor = if positioned_text.rjust {
                            "end"
                        } else if positioned_text.ljust {
                            "start"
                        } else {
                            "middle"
                        };
                        let line_y = start_y + i as f64 * charht_px;
                        // Text color comes from stroke ("color" attribute in pikchr)
                        let text_color = if obj.style().stroke == "black" || obj.style().stroke == "none" {
                            "rgb(0,0,0)".to_string()
                        } else {
                            color_to_rgb(&obj.style().stroke)
                        };
                        let text_element = Text {
                            x: Some(center.x),
                            y: Some(line_y),
                            fill: Some(text_color),
                            stroke: None,
                            stroke_width: None,
                            style: SvgStyle::default(),
                            text_anchor: Some(anchor.to_string()),
                            dominant_baseline: Some("central".to_string()),
                            // C pikchr uses NO-BREAK SPACE (U+00A0) in SVG text output
                            content: positioned_text.value.replace(' ', "\u{00A0}"),
                        };
                        svg_children.push(SvgNode::Text(text_element));
                    }
                }

                ClassName::Sublist => {
                    // Render sublist children with offset
                    // Children are now ShapeEnums, use their render_svg method
                    if let Some(children) = obj.children() {
                        for child in children {
                            let child_nodes = child.render_svg(&scaler, offset_x, max_y, dashwid);
                            svg_children.extend(child_nodes);
                        }
                    }
                }
            }
        } // end if !obj.style().invisible

        // Render text labels inside objects (always rendered, even for invisible shapes)
        if obj.class() != ClassName::Text && !obj.text().is_empty() {
            // For cylinders, C pikchr shifts text down by 0.75 * cylrad
            // This accounts for the top ellipse taking up space
            let text_y_offset = if obj.class() == ClassName::Cylinder {
                let cylrad = get_length(ctx, "cylrad", 0.075);
                scaler.px(Inches::inches(0.75 * cylrad))
            } else {
                0.0
            };

            // Text color comes from stroke ("color" attribute in pikchr)
            let text_color = if obj.style().stroke == "black" || obj.style().stroke == "none" {
                "rgb(0,0,0)".to_string()
            } else {
                color_to_rgb(&obj.style().stroke)
            };
            for positioned_text in obj.text() {
                // Determine text anchor based on ljust/rjust
                let anchor = if positioned_text.rjust {
                    "end"
                } else if positioned_text.ljust {
                    "start"
                } else {
                    "middle"
                };
                let text_element = Text {
                    x: Some(center.x),
                    y: Some(center.y + text_y_offset),
                    fill: Some(text_color.clone()),
                    stroke: None,
                    stroke_width: None,
                    style: SvgStyle::default(),
                    text_anchor: Some(anchor.to_string()),
                    dominant_baseline: Some("central".to_string()),
                    // C pikchr uses NO-BREAK SPACE (U+00A0) in SVG text output
                    content: positioned_text.value.replace(' ', "\u{00A0}"),
                };
                svg_children.push(SvgNode::Text(text_element));
            }
        }
    }

    // Set children on the SVG element
    svg.children = svg_children;

    // Create wrapper function for fmt_num to match the expected signature
    fn format_float(value: f64, writer: &mut dyn std::io::Write) -> Result<(), std::io::Error> {
        write!(writer, "{}", fmt_num(value))
    }

    // Serialize to string using facet_xml with custom f64 formatter to match C pikchr precision
    let options = SerializeOptions {
        float_formatter: Some(format_float),
        ..Default::default()
    };
    facet_xml::to_string_with_options(&svg, &options)
        .map_err(|e| miette::miette!("XML serialization error: {}", e))
}

fn create_svg_style(style: &ObjectStyle, scaler: &Scaler, dashwid: Inches) -> SvgStyle {
    // Build SvgStyle for DOM-based generation
    let fill = Color::parse(&style.fill);
    let stroke = Color::parse(&style.stroke);
    let stroke_width_val = scaler.len(style.stroke_width).0;

    let stroke_dasharray = if style.dashed {
        // C pikchr: stroke-dasharray: dashwid, dashwid
        let dash = scaler.len(dashwid).0;
        Some((dash, dash))
    } else if style.dotted {
        // C pikchr: stroke-dasharray: stroke_width, dashwid
        let sw = stroke_width_val.max(2.1); // min 2.1 per C source
        let dash = scaler.len(dashwid).0;
        Some((sw, dash))
    } else {
        None
    };

    let mut svg_style = SvgStyle::new();
    svg_style
        .properties
        .insert("fill".to_string(), fill.to_string());
    svg_style
        .properties
        .insert("stroke".to_string(), stroke.to_string());
    svg_style
        .properties
        .insert("stroke-width".to_string(), fmt_num(stroke_width_val));
    if let Some((a, b)) = stroke_dasharray {
        svg_style
            .properties
            .insert("stroke-dasharray".to_string(), format!("{:.2},{:.2}", a, b));
    }

    svg_style
}

/// Render an arrowhead polygon at the end of a line
/// The arrowhead points in the direction from start to end
pub fn render_arrowhead_dom(
    start: DVec2,
    end: DVec2,
    style: &ObjectStyle,
    arrow_len: f64,
    arrow_width: f64,
) -> Option<Polygon> {
    // Calculate direction vector
    let delta = end - start;
    let len = delta.length();

    if len < 0.001 {
        return None; // Zero-length line, no arrowhead
    }

    // Unit vector in direction of line
    let unit = delta / len;

    // Perpendicular unit vector
    let perp = dvec2(-unit.y, unit.x);

    // Arrow tip is at end
    // Base points are arrow_len back along the line, offset by half arrow_width perpendicular
    // Note: arrowwid is the FULL base width, so we use arrow_width/2 for the half-width
    let base = end - unit * arrow_len;
    let half_width = arrow_width / 2.0;

    let p1 = base + perp * half_width;
    let p2 = base - perp * half_width;

    let points = format!(
        "{},{} {},{} {},{}",
        fmt_num(end.x),
        fmt_num(end.y),
        fmt_num(p1.x),
        fmt_num(p1.y),
        fmt_num(p2.x),
        fmt_num(p2.y)
    );

    let fill_color = Color::parse(&style.stroke);

    Some(Polygon {
        points: Some(points),
        fill: None,
        stroke: None,
        stroke_width: None,
        stroke_dasharray: None,
        style: SvgStyle::new().add("fill", &fill_color.to_string()),
    })
}
