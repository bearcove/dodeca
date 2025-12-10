//! SVG generation

use super::shapes::Shape;
use crate::types::{Length as Inches, Scaler};
use facet_svg::facet_xml::SerializeOptions;
use facet_svg::{Color, Polygon, Svg, SvgNode, SvgStyle, Text, facet_xml, fmt_num};
use glam::{DVec2, dvec2};
use time::{OffsetDateTime, format_description};

use super::context::RenderContext;
use super::defaults;
use super::eval::{get_length, get_scalar};
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

        // Render shape using the Shape trait's render_svg method
        if !obj.style().invisible {
            // Use the shape's render_svg method which properly handles Y-flipping
            let shape = &obj.shape;
            let shape_nodes = shape.render_svg(
                obj,
                &scaler,
                offset_x,
                max_y,
                dashwid,
                arrow_ht,
                arrow_wid,
            );
            svg_children.extend(shape_nodes);
        }

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
