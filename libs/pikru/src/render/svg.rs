//! SVG generation

use std::fmt::Write;

use crate::types::{Length as Inches, Point, Scaler};
use facet_svg::facet_xml::SerializeOptions;
use facet_svg::{
    Circle, Color, Ellipse, Path, PathData, Polygon, Svg, SvgNode, SvgStyle, Text, facet_xml,
    fmt_num,
};
use time::{OffsetDateTime, format_description};

use super::context::RenderContext;
use super::defaults;
use super::eval::{get_length, get_scalar};
use super::geometry::{
    apply_auto_chop_simple_line, chop_line, create_arc_path, create_cylinder_paths_with_rad,
    create_file_paths, create_oval_path, create_rounded_box_path, create_spline_path,
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
    match color.to_lowercase().as_str() {
        "black" => "rgb(0,0,0)".to_string(),
        "white" => "rgb(255,255,255)".to_string(),
        "red" => "rgb(255,0,0)".to_string(),
        "green" => "rgb(0,128,0)".to_string(),
        "blue" => "rgb(0,0,255)".to_string(),
        "yellow" => "rgb(255,255,0)".to_string(),
        "cyan" => "rgb(0,255,255)".to_string(),
        "magenta" => "rgb(255,0,255)".to_string(),
        "gray" | "grey" => "rgb(128,128,128)".to_string(),
        "lightgray" | "lightgrey" => "rgb(211,211,211)".to_string(),
        "darkgray" | "darkgrey" => "rgb(169,169,169)".to_string(),
        "orange" => "rgb(255,165,0)".to_string(),
        "pink" => "rgb(255,192,203)".to_string(),
        "purple" => "rgb(128,0,128)".to_string(),
        "none" => "none".to_string(),
        _ => color.to_string(),
    }
}

/// Generate SVG from render context
pub fn generate_svg(ctx: &RenderContext) -> Result<String, miette::Report> {
    let margin_base = get_length(ctx, "margin", defaults::MARGIN);
    let left_margin = get_length(ctx, "leftmargin", 0.0);
    let right_margin = get_length(ctx, "rightmargin", 0.0);
    let top_margin = get_length(ctx, "topmargin", 0.0);
    let bottom_margin = get_length(ctx, "bottommargin", 0.0);
    let thickness = get_length(ctx, "thickness", defaults::STROKE_WIDTH.raw());

    let margin = margin_base + thickness;
    let r_scale = 144.0; // match pikchr.c rScale - always use base scale for coordinates
    let scale = get_scalar(ctx, "scale", 1.0);
    // C pikchr uses r_scale for coordinate conversion, scale only affects display size
    let scaler = Scaler::try_new(r_scale)
        .map_err(|e| miette::miette!("invalid scale value {}: {}", r_scale, e))?;
    let arrow_ht = Inches(get_length(ctx, "arrowht", 0.08));
    let arrow_wid = Inches(get_length(ctx, "arrowwid", 0.06));
    let dashwid = Inches(get_length(ctx, "dashwid", 0.05));
    let arrow_len_px = scaler.len(arrow_ht);
    let arrow_wid_px = scaler.len(arrow_wid);
    let mut bounds = ctx.bounds;

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
    let offset_y = -bounds.min.y;

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
        let tx = scaler.px(obj.center.x + offset_x);
        let ty = scaler.px(obj.center.y + offset_y);
        let sx = scaler.px(obj.start.x + offset_x);
        let sy = scaler.px(obj.start.y + offset_y);
        let ex = scaler.px(obj.end.x + offset_x);
        let ey = scaler.px(obj.end.y + offset_y);

        let svg_style = create_svg_style(&obj.style, &scaler, dashwid);

        // Render shape (skip if invisible, but still render text below)
        if !obj.style.invisible {
            match obj.class {
                ObjectClass::Box => {
                    let x1 = tx - scaler.px(obj.width / 2.0);
                    let x2 = tx + scaler.px(obj.width / 2.0);
                    let y1 = ty - scaler.px(obj.height / 2.0);
                    let y2 = ty + scaler.px(obj.height / 2.0);

                    let path_data = if obj.style.corner_radius > Inches::ZERO {
                        // Rounded box using proper path with arcs
                        let r = scaler.px(obj.style.corner_radius);
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
                ObjectClass::Circle => {
                    let r = scaler.px(obj.width / 2.0);
                    let circle = Circle {
                        cx: Some(tx),
                        cy: Some(ty),
                        r: Some(r),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style,
                    };
                    svg_children.push(SvgNode::Circle(circle));
                }
                ObjectClass::Dot => {
                    // Dot is a small filled circle
                    let r = scaler.px(obj.width / 2.0);
                    let fill = if obj.style.fill == "none" {
                        &obj.style.stroke
                    } else {
                        &obj.style.fill
                    };
                    let mut dot_style = SvgStyle::new();
                    dot_style
                        .properties
                        .insert("fill".to_string(), fill.to_string());
                    let circle = Circle {
                        cx: Some(tx),
                        cy: Some(ty),
                        r: Some(r),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: dot_style,
                    };
                    svg_children.push(SvgNode::Circle(circle));
                }
                ObjectClass::Ellipse => {
                    let rx = scaler.px(obj.width / 2.0);
                    let ry = scaler.px(obj.height / 2.0);
                    let ellipse = Ellipse {
                        cx: Some(tx),
                        cy: Some(ty),
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
                ObjectClass::Oval => {
                    // Oval is a pill shape (rounded rectangle with fully rounded ends)
                    // C pikchr renders as a path, not a rect
                    let width = scaler.px(obj.width);
                    let height = scaler.px(obj.height);
                    let rad = height.min(width) / 2.0; // radius is half of smaller dimension
                    let x1 = tx - width / 2.0;
                    let x2 = tx + width / 2.0;
                    let y1 = ty - height / 2.0;
                    let y2 = ty + height / 2.0;

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
                ObjectClass::Cylinder => {
                    // Cylinder: single path with 3 arcs matching C pikchr
                    // Uses cylrad for the ellipse vertical radius (default 0.075")
                    let width = scaler.px(obj.width);
                    let height = scaler.px(obj.height);
                    let cylrad = get_length(ctx, "cylrad", 0.075);
                    let ry = scaler.px(Inches::inches(cylrad));

                    let (body_path, _) = create_cylinder_paths_with_rad(tx, ty, width, height, ry);

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
                ObjectClass::File => {
                    // File: proper path-based rendering with fold matching C pikchr
                    let width = scaler.px(obj.width);
                    let height = scaler.px(obj.height);
                    let filerad = scaler.px(defaults::FILE_RAD);

                    let (main_path, fold_path) = create_file_paths(tx, ty, width, height, filerad);

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
                ObjectClass::Diamond => {
                    // Diamond: rotated square/rhombus with vertices at edges
                    // C pikchr uses path: M left,cy L cx,bottom L right,cy L cx,top Z
                    let half_w = scaler.px(obj.width / 2.0);
                    let half_h = scaler.px(obj.height / 2.0);
                    let left = tx - half_w;
                    let right = tx + half_w;
                    let top = ty - half_h;
                    let bottom = ty + half_h;
                    let path_data = PathData::new()
                        .m(left, ty) // Left vertex
                        .l(tx, bottom) // Bottom vertex
                        .l(right, ty) // Right vertex
                        .l(tx, top) // Top vertex
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
                ObjectClass::Line | ObjectClass::Arrow => {
                    // Auto-chop always applies for object-attached endpoints (trims to boundary)
                    // The explicit "chop" attribute is for additional user-requested shortening
                    let (draw_sx, draw_sy, draw_ex, draw_ey) = if obj.waypoints.len() <= 2 {
                        apply_auto_chop_simple_line(
                            &scaler, obj, sx, sy, ex, ey, offset_x, offset_y,
                        )
                    } else {
                        (sx, sy, ex, ey)
                    };

                    if obj.waypoints.len() <= 2 {
                        // Simple line - render as <path> (matching C pikchr)
                        // First render arrowhead polygon if needed (rendered before line, like C)
                        if obj.style.arrow_end {
                            if let Some(arrowhead) = render_arrowhead_dom(
                                draw_sx,
                                draw_sy,
                                draw_ex,
                                draw_ey,
                                &obj.style,
                                arrow_len_px.0,
                                arrow_wid_px.0,
                            ) {
                                svg_children.push(SvgNode::Polygon(arrowhead));
                            }
                        }
                        if obj.style.arrow_start {
                            if let Some(arrowhead) = render_arrowhead_dom(
                                draw_ex,
                                draw_ey,
                                draw_sx,
                                draw_sy,
                                &obj.style,
                                arrow_len_px.0,
                                arrow_wid_px.0,
                            ) {
                                svg_children.push(SvgNode::Polygon(arrowhead));
                            }
                        }

                        // Chop line endpoints for arrowheads (by arrowht/2 as in C pikchr, in pixels)
                        let arrow_chop_px = arrow_len_px.0 / 2.0;
                        let (line_sx, line_sy, line_ex, line_ey) = {
                            let mut sx = draw_sx;
                            let mut sy = draw_sy;
                            let mut ex = draw_ex;
                            let mut ey = draw_ey;

                            if obj.style.arrow_start {
                                let (new_sx, new_sy, _, _) =
                                    chop_line(sx, sy, ex, ey, arrow_chop_px);
                                sx = new_sx;
                                sy = new_sy;
                            }
                            if obj.style.arrow_end {
                                let (_, _, new_ex, new_ey) =
                                    chop_line(sx, sy, ex, ey, arrow_chop_px);
                                ex = new_ex;
                                ey = new_ey;
                            }
                            (sx, sy, ex, ey)
                        };

                        // Render the line path (with chopped endpoints)
                        let line_path_data = PathData::new()
                            .m(line_sx, line_sy)
                            .l(line_ex, line_ey);
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
                        let mut points = obj.waypoints.clone();
                        if obj.style.chop && points.len() >= 2 {
                            let chop_amount_px = scaler.px(defaults::CIRCLE_RADIUS);
                            // Chop start
                            let p0 = points[0];
                            let p1 = points[1];
                            let (new_x, new_y, _, _) =
                                chop_line(p0.x.0, p0.y.0, p1.x.0, p1.y.0, chop_amount_px);
                            points[0] = Point::new(Inches(new_x), Inches(new_y));

                            // Chop end
                            let n = points.len();
                            let pn1 = points[n - 2];
                            let pn = points[n - 1];
                            let (_, _, new_x, new_y) =
                                chop_line(pn1.x.0, pn1.y.0, pn.x.0, pn.y.0, chop_amount_px);
                            points[n - 1] = Point::new(Inches(new_x), Inches(new_y));
                        }

                        if obj.style.close_path {
                            // Build path using fluent API (no arrow chopping for closed paths)
                            let mut path_data = PathData::new();
                            for (i, p) in points.iter().enumerate() {
                                let px = scaler.px(p.x + offset_x);
                                let py = scaler.px(p.y + offset_y);
                                if i == 0 {
                                    path_data = path_data.m(px, py);
                                } else {
                                    path_data = path_data.l(px, py);
                                }
                            }
                            path_data = path_data.z();
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
                            if (obj.style.arrow_end || obj.style.arrow_start) && points.len() >= 2 {
                                let chop_amount_px = arrow_len_px.0 / 2.0;

                                // Chop start if arrow_start
                                if obj.style.arrow_start {
                                    let p0 = points[0];
                                    let p1 = points[1];
                                    let (new_x, new_y, _, _) =
                                        chop_line(p0.x.0, p0.y.0, p1.x.0, p1.y.0, chop_amount_px);
                                    points[0] = Point::new(Inches(new_x), Inches(new_y));
                                }

                                // Chop end if arrow_end
                                if obj.style.arrow_end {
                                    let n = points.len();
                                    let pn1 = points[n - 2];
                                    let pn = points[n - 1];
                                    let (_, _, new_x, new_y) =
                                        chop_line(pn1.x.0, pn1.y.0, pn.x.0, pn.y.0, chop_amount_px);
                                    points[n - 1] = Point::new(Inches(new_x), Inches(new_y));
                                }
                            }

                            // Now render the polyline using fluent API
                            let mut path_data = PathData::new();
                            for (i, p) in points.iter().enumerate() {
                                let px = scaler.px(p.x + offset_x);
                                let py = scaler.px(p.y + offset_y);
                                if i == 0 {
                                    path_data = path_data.m(px, py);
                                } else {
                                    path_data = path_data.l(px, py);
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
                            svg_children.push(SvgNode::Path(path));
                        }
                    }
                }
                ObjectClass::Spline => {
                    // Proper spline rendering with Bezier curves
                    let spline_path_data = create_spline_path(&obj.waypoints, offset_x, offset_y);

                    // Render arrowheads first (before path, like C pikchr)
                    let n = obj.waypoints.len();
                    if obj.style.arrow_end && n >= 2 {
                        let p1 = obj.waypoints[n - 2];
                        let p2 = obj.waypoints[n - 1];
                        if let Some(arrowhead) = render_arrowhead_dom(
                            p1.x.0 + offset_x.0,
                            p1.y.0 + offset_y.0,
                            p2.x.0 + offset_x.0,
                            p2.y.0 + offset_y.0,
                            &obj.style,
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        ) {
                            svg_children.push(SvgNode::Polygon(arrowhead));
                        }
                    }
                    if obj.style.arrow_start && n >= 2 {
                        let p1 = obj.waypoints[0];
                        let p2 = obj.waypoints[1];
                        if let Some(arrowhead) = render_arrowhead_dom(
                            p2.x.0 + offset_x.0,
                            p2.y.0 + offset_y.0,
                            p1.x.0 + offset_x.0,
                            p1.y.0 + offset_y.0,
                            &obj.style,
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
                ObjectClass::Move => {
                    // Move: invisible - just advances the cursor, renders nothing
                }

                ObjectClass::Arc => {
                    // Proper arc rendering using curved arc paths
                    let radius = scaler.px(obj.width / 2.0); // Use width as arc radius
                    let arc_path_data = create_arc_path(sx, sy, ex, ey, radius);

                    // Render arrowheads first (before path, like C pikchr)
                    if obj.style.arrow_end {
                        if let Some(arrowhead) = render_arrowhead_dom(
                            sx,
                            sy,
                            ex,
                            ey,
                            &obj.style,
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        ) {
                            svg_children.push(SvgNode::Polygon(arrowhead));
                        }
                    }
                    if obj.style.arrow_start {
                        if let Some(arrowhead) = render_arrowhead_dom(
                            ex,
                            ey,
                            sx,
                            sy,
                            &obj.style,
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
                ObjectClass::Text => {
                    // Multi-line text: distribute lines vertically around center
                    // C pikchr stacks lines with charht spacing, centered on the object center
                    let charht_px = scaler.px(Inches(defaults::FONT_SIZE));
                    let line_count = obj.text.len();
                    // Start Y: center minus half of total height, plus half charht for first line center
                    // Total height = (line_count - 1) * charht, distributed evenly around center
                    let start_y = ty - (line_count as f64 - 1.0) * charht_px / 2.0;

                    for (i, positioned_text) in obj.text.iter().enumerate() {
                        // Determine text anchor based on ljust/rjust
                        let anchor = if positioned_text.rjust {
                            "end"
                        } else if positioned_text.ljust {
                            "start"
                        } else {
                            "middle"
                        };
                        let line_y = start_y + i as f64 * charht_px;
                        let text_element = Text {
                            x: Some(tx),
                            y: Some(line_y),
                            fill: Some("rgb(0,0,0)".to_string()),
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

                ObjectClass::Sublist => {
                    // Render sublist children with offset
                    for child in &obj.children {
                        let child_tx = scaler.px(child.center.x + offset_x);
                        let child_ty = scaler.px(child.center.y + offset_y);
                        let child_svg_style = create_svg_style(&child.style, &scaler, dashwid);

                        match child.class {
                            ObjectClass::Box => {
                                let x1 = child_tx - scaler.px(child.width / 2.0);
                                let x2 = child_tx + scaler.px(child.width / 2.0);
                                let y1 = child_ty - scaler.px(child.height / 2.0);
                                let y2 = child_ty + scaler.px(child.height / 2.0);

                                let path_data = PathData::new()
                                    .m(x1, y2)
                                    .l(x2, y2)
                                    .l(x2, y1)
                                    .l(x1, y1)
                                    .z();
                                let path = Path {
                                    d: Some(path_data),
                                    fill: None,
                                    stroke: None,
                                    stroke_width: None,
                                    stroke_dasharray: None,
                                    style: child_svg_style,
                                };
                                svg_children.push(SvgNode::Path(path));
                            }
                            ObjectClass::Circle => {
                                let r = scaler.px(child.width / 2.0);
                                let circle = Circle {
                                    cx: Some(child_tx),
                                    cy: Some(child_ty),
                                    r: Some(r),
                                    fill: None,
                                    stroke: None,
                                    stroke_width: None,
                                    stroke_dasharray: None,
                                    style: child_svg_style.clone(),
                                };
                                svg_children.push(SvgNode::Circle(circle));
                            }
                            _ => {
                                // For other shapes, just render as a simple circle placeholder
                                let r = scaler.px(child.width / 4.0);
                                let circle = Circle {
                                    cx: Some(child_tx),
                                    cy: Some(child_ty),
                                    r: Some(r),
                                    fill: None,
                                    stroke: None,
                                    stroke_width: None,
                                    stroke_dasharray: None,
                                    style: child_svg_style.clone(),
                                };
                                svg_children.push(SvgNode::Circle(circle));
                            }
                        }
                    }
                }
            }
        } // end if !obj.style.invisible

        // Render text labels inside objects (always rendered, even for invisible shapes)
        if obj.class != ObjectClass::Text && !obj.text.is_empty() {
            // For cylinders, C pikchr shifts text down by 0.75 * cylrad
            // This accounts for the top ellipse taking up space
            let text_y_offset = if obj.class == ObjectClass::Cylinder {
                let cylrad = get_length(ctx, "cylrad", 0.075);
                scaler.px(Inches::inches(0.75 * cylrad))
            } else {
                0.0
            };

            for positioned_text in &obj.text {
                // Determine text anchor based on ljust/rjust
                let anchor = if positioned_text.rjust {
                    "end"
                } else if positioned_text.ljust {
                    "start"
                } else {
                    "middle"
                };
                let text_element = Text {
                    x: Some(tx),
                    y: Some(ty + text_y_offset),
                    fill: Some("rgb(0,0,0)".to_string()),
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
/// The arrowhead points in the direction from (sx, sy) to (ex, ey)
fn render_arrowhead_dom(
    sx: f64,
    sy: f64,
    ex: f64,
    ey: f64,
    style: &ObjectStyle,
    arrow_len: f64,
    arrow_width: f64,
) -> Option<Polygon> {
    // Calculate direction vector
    let dx = ex - sx;
    let dy = ey - sy;
    let len = (dx * dx + dy * dy).sqrt();

    if len < 0.001 {
        return None; // Zero-length line, no arrowhead
    }

    // Unit vector in direction of line
    let ux = dx / len;
    let uy = dy / len;

    // Perpendicular unit vector
    let px = -uy;
    let py = ux;

    // Arrow tip is at (ex, ey)
    // Base points are arrow_len back along the line, offset by half arrow_width perpendicular
    // Note: arrowwid is the FULL base width, so we use arrow_width/2 for the half-width
    let base_x = ex - ux * arrow_len;
    let base_y = ey - uy * arrow_len;
    let half_width = arrow_width / 2.0;

    let p1_x = base_x + px * half_width;
    let p1_y = base_y + py * half_width;
    let p2_x = base_x - px * half_width;
    let p2_y = base_y - py * half_width;

    let points = format!(
        "{},{} {},{} {},{}",
        fmt_num(ex),
        fmt_num(ey),
        fmt_num(p1_x),
        fmt_num(p1_y),
        fmt_num(p2_x),
        fmt_num(p2_y)
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

/// Legacy string-based arrowhead rendering
fn render_arrowhead(
    svg: &mut String,
    sx: f64,
    sy: f64,
    ex: f64,
    ey: f64,
    style: &ObjectStyle,
    arrow_len: f64,
    arrow_width: f64,
) {
    // Calculate direction vector
    let dx = ex - sx;
    let dy = ey - sy;
    let len = (dx * dx + dy * dy).sqrt();

    if len < 0.001 {
        return; // Zero-length line, no arrowhead
    }

    // Unit vector in direction of line
    let ux = dx / len;
    let uy = dy / len;

    // Perpendicular unit vector
    let px = -uy;
    let py = ux;

    // Arrow tip is at (ex, ey)
    // Base points are arrow_len back along the line, offset by half arrow_width perpendicular
    // Note: arrowwid is the FULL base width, so we use arrow_width/2 for the half-width
    let base_x = ex - ux * arrow_len;
    let base_y = ey - uy * arrow_len;
    let half_width = arrow_width / 2.0;

    let p1_x = base_x + px * half_width;
    let p1_y = base_y + py * half_width;
    let p2_x = base_x - px * half_width;
    let p2_y = base_y - py * half_width;

    writeln!(
        svg,
        r#"  <polygon points="{},{} {},{} {},{}" style="fill:{}"/>"#,
        fmt_num(ex),
        fmt_num(ey),
        fmt_num(p1_x),
        fmt_num(p1_y),
        fmt_num(p2_x),
        fmt_num(p2_y),
        color_to_rgb(&style.stroke)
    )
    .unwrap();
}
