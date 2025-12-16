//! SVG generation

use super::shapes::{Shape, svg_style_from_entries};
use super::{TextVSlot, compute_text_vslots};
use crate::types::{Length as Inches, Scaler};
use facet_svg::facet_xml::SerializeOptions;
use facet_svg::{Color, Points, Polygon, Svg, SvgNode, SvgStyle, Text, facet_xml};
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
    color
        .parse::<crate::types::Color>()
        .unwrap()
        .to_rgb_string()
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
            let shape_nodes =
                shape.render_svg(obj, &scaler, offset_x, max_y, dashwid, arrow_ht, arrow_wid);
            svg_children.extend(shape_nodes);
        }

        // Render text labels inside objects (always rendered, even for invisible shapes)
        // cref: pik_append_txt (pikchr.c:5077)
        if obj.class() != ClassName::Text && !obj.text().is_empty() {
            let texts = obj.text();
            let charht = get_length(ctx, "charht", 0.14);

            // For cylinders, C pikchr shifts text down by 0.75 * cylrad
            // This accounts for the top ellipse taking up space
            let y_base = if obj.class() == ClassName::Cylinder {
                let cylrad = get_length(ctx, "cylrad", 0.075);
                -0.75 * cylrad // In pikchr coordinates (positive Y is up)
            } else {
                0.0
            };

            // Compute slot assignments for all text lines
            let slots = compute_text_vslots(texts);

            // Helper to compute font scale factor
            fn font_scale(text: &PositionedText) -> f64 {
                if text.big { 1.25 } else if text.small { 0.8 } else { 1.0 }
            }

            // Compute max height for each slot type (like C's ha2, ha1, hc, hb1, hb2)
            // cref: pik_append_txt (pikchr.c:5104-5143)
            let mut ha2: f64 = 0.0; // Height of Above2 row
            let mut ha1: f64 = 0.0; // Height of Above row
            let mut hc: f64 = 0.0;  // Height of Center row
            let mut hb1: f64 = 0.0; // Height of Below row
            let mut hb2: f64 = 0.0; // Height of Below2 row

            for (text, slot) in texts.iter().zip(slots.iter()) {
                let h = font_scale(text) * charht;
                tracing::debug!(text = %text.value, ?slot, h, "slot assignment");
                match slot {
                    TextVSlot::Above2 => ha2 = ha2.max(h),
                    TextVSlot::Above => ha1 = ha1.max(h),
                    TextVSlot::Center => hc = hc.max(h),
                    TextVSlot::Below => hb1 = hb1.max(h),
                    TextVSlot::Below2 => hb2 = hb2.max(h),
                }
            }
            tracing::debug!(ha2, ha1, hc, hb1, hb2, "slot heights");

            // Text color comes from stroke ("color" attribute in pikchr)
            let text_color = if obj.style().stroke == "black" || obj.style().stroke == "none" {
                "rgb(0,0,0)".to_string()
            } else {
                color_to_rgb(&obj.style().stroke)
            };

            for (positioned_text, slot) in texts.iter().zip(slots.iter()) {
                // Compute y offset based on slot assignment
                // cref: pik_append_txt (pikchr.c:5155-5158)
                let mut y_offset = y_base;
                match slot {
                    TextVSlot::Above2 => y_offset += 0.5 * hc + ha1 + 0.5 * ha2,
                    TextVSlot::Above => y_offset += 0.5 * hc + 0.5 * ha1,
                    TextVSlot::Center => {} // No offset
                    TextVSlot::Below => y_offset -= 0.5 * hc + 0.5 * hb1,
                    TextVSlot::Below2 => y_offset -= 0.5 * hc + hb1 + 0.5 * hb2,
                }

                // Convert y offset to SVG pixels (negative because SVG Y is flipped)
                let svg_y_offset = scaler.px(Inches::inches(-y_offset));
                tracing::debug!(
                    text = %positioned_text.value,
                    ?slot,
                    y_offset,
                    svg_y_offset,
                    center_y = center.y,
                    final_y = center.y + svg_y_offset,
                    "text y position"
                );

                // Determine text anchor and x position based on ljust/rjust
                // cref: pik_append_txt (pikchr.c:5144-5160)
                // For box-style shapes (eJust=1), calculate jw padding
                let uses_box_justification =
                    matches!(obj.class(), ClassName::Box | ClassName::Cylinder);
                let charwid = get_length(ctx, "charwid", 0.08);
                let jw_inches = if uses_box_justification {
                    // jw = 0.5 * (obj.width - 0.5 * (charWidth + strokeWidth))
                    0.5 * (obj.width().0 - 0.5 * (charwid + thickness))
                } else {
                    0.0
                };
                let jw = scaler.px(Inches(jw_inches));

                let (anchor, text_x) = if positioned_text.rjust {
                    ("end", center.x + jw)
                } else if positioned_text.ljust {
                    ("start", center.x - jw)
                } else {
                    ("middle", center.x)
                };

                // Determine font styling based on text attributes
                // cref: pik_append_txt (pikchr.c:5180-5200)
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
                // C pikchr uses percentage-based font sizes: 125% for big, 80% for small
                // cref: pik_append_txt (pikchr.c:5183)
                let font_size = if positioned_text.big {
                    Some("125%".to_string())
                } else if positioned_text.small {
                    Some("80%".to_string())
                } else {
                    None
                };

                let text_element = Text {
                    x: Some(text_x),
                    y: Some(center.y + svg_y_offset),
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

    let points = Points::new()
        .push(end.x, end.y)
        .push(p1.x, p1.y)
        .push(p2.x, p2.y);

    let fill_color = Color::parse(&style.stroke);

    Some(Polygon {
        points,
        fill: None,
        stroke: None,
        stroke_width: None,
        stroke_dasharray: None,
        style: svg_style_from_entries(vec![("fill", fill_color.to_string())]),
    })
}

/// Format a number with up to 3 decimal places (sufficient for SVG coordinates).
/// Trims trailing zeros and decimal point.
fn fmt_num(value: f64) -> String {
    let s = format!("{:.3}", value);
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    s.to_string()
}
