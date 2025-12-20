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

/// Decode HTML entities in text content.
///
/// C pikchr allows HTML entities in string literals (e.g., `&amp;` for `&`).
/// Since facet-xml will escape `&` to `&amp;` during serialization, we need to
/// decode entities first so they get re-encoded correctly.
///
/// cref: pik_isentity (pikchr.c:4728) - checks if text starts with HTML entity
/// cref: pik_append_text (pikchr.c:4766) - handles entity pass-through
fn decode_html_entities(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '&' {
            // Collect the potential entity
            let mut entity = String::from("&");
            let mut found_semicolon = false;

            while let Some(&next) = chars.peek() {
                if next == ';' {
                    entity.push(chars.next().unwrap());
                    found_semicolon = true;
                    break;
                } else if next.is_ascii_alphanumeric() || next == '#' {
                    entity.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            if found_semicolon {
                // Try to decode the entity
                match entity.as_str() {
                    "&amp;" => result.push('&'),
                    "&lt;" => result.push('<'),
                    "&gt;" => result.push('>'),
                    "&quot;" => result.push('"'),
                    "&apos;" => result.push('\''),
                    "&nbsp;" => result.push('\u{00A0}'),
                    _ if entity.starts_with("&#x") || entity.starts_with("&#X") => {
                        // Hex numeric entity: &#xHH;
                        if let Ok(code) = u32::from_str_radix(&entity[3..entity.len() - 1], 16) {
                            if let Some(ch) = char::from_u32(code) {
                                result.push(ch);
                                continue;
                            }
                        }
                        result.push_str(&entity);
                    }
                    _ if entity.starts_with("&#") => {
                        // Decimal numeric entity: &#NNN;
                        if let Ok(code) = entity[2..entity.len() - 1].parse::<u32>() {
                            if let Some(ch) = char::from_u32(code) {
                                result.push(ch);
                                continue;
                            }
                        }
                        result.push_str(&entity);
                    }
                    _ => {
                        // Unknown entity, pass through as-is
                        result.push_str(&entity);
                    }
                }
            } else {
                // Not a valid entity, output what we collected
                result.push_str(&entity);
            }
        } else {
            result.push(c);
        }
    }

    result
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
    // cref: pik_render (pikchr.c:7282) - clamp thickness to minimum 0.01
    let thickness = thickness.max(0.01);

    let margin = margin_base + thickness;
    let scale = get_scalar(ctx, "scale", 1.0);
    let fontscale = get_scalar(ctx, "fontscale", 1.0);
    // C pikchr uses constant rScale=144.0 for all coordinates
    // Scale only affects the display width/height attributes
    let r_scale = 144.0;
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
    // cref: pik_render (pikchr.c:4632-4633) - uses pik_round which truncates (int)v
    let is_scaled = scale < 0.99 || scale > 1.01;
    if is_scaled {
        let display_width = (viewbox_width * scale) as i32;
        let display_height = (viewbox_height * scale) as i32;
        svg.width = Some(display_width.to_string());
        svg.height = Some(display_height.to_string());
    }

    // Arrowheads are now rendered inline as polygon elements (matching C pikchr)

    // Helper to render text for an object (and recursively for sublist children)
    fn render_object_text(
        obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        charht: f64,
        charwid: f64,
        thickness: f64,
        fontscale: f64,
        svg_children: &mut Vec<SvgNode>,
    ) {
        // Convert from pikchr coordinates (Y-up) to SVG pixels (Y-down)
        let center = obj.center().to_svg(scaler, offset_x, max_y);

        // Render text labels inside objects (always rendered, even for invisible shapes)
        // cref: pik_append_txt (pikchr.c:5077)
        // Note: Text objects also use this path - TextShape::render_svg returns empty nodes
        if !obj.text().is_empty() {
            let texts = obj.text();

            // For cylinders, C pikchr shifts text down by 0.75 * rad
            // cref: pik_append_txt (pikchr.c:5102-5104)
            // Note: C code only applies yBase if rad > 0
            let y_base = if let super::shapes::ShapeEnum::Cylinder(cyl) = &obj.shape {
                if cyl.ellipse_rad.0 > 0.0 {
                    -0.75 * cyl.ellipse_rad.0
                } else {
                    0.0
                }
            } else {
                0.0
            };

            // Compute slot assignments for all text lines
            let slots = compute_text_vslots(texts);

            // cref: pik_append_txt (pikchr.c:2421-2422) - for lines, hc starts at sw*1.5
            // This reserves vertical space for the line stroke when positioning text
            // The value can only increase if center text is taller (via .max() below)
            let is_line = matches!(
                obj.class(),
                ClassName::Line | ClassName::Arrow | ClassName::Spline | ClassName::Move
            );

            // cref: pik_append_txt (pikchr.c:2407) - sw = pObj->sw>=0.0 ? pObj->sw : 0
            // Use object's stroke width, clamped to 0 if negative
            let obj_sw = obj.style().stroke_width.raw().max(0.0);

            let mut ha2: f64 = 0.0;
            let mut ha1: f64 = 0.0;
            let mut hc: f64 = if is_line { obj_sw * 1.5 } else { 0.0 };
            let mut hb1: f64 = 0.0;
            let mut hb2: f64 = 0.0;

            // cref: pik_append_txt (pikchr.c:5114-5147) - uses pik_font_scale for each text
            for (text, slot) in texts.iter().zip(slots.iter()) {
                let h = text.font_scale() * charht;
                match slot {
                    TextVSlot::Above2 => ha2 = ha2.max(h),
                    TextVSlot::Above => ha1 = ha1.max(h),
                    TextVSlot::Center => hc = hc.max(h),
                    TextVSlot::Below => hb1 = hb1.max(h),
                    TextVSlot::Below2 => hb2 = hb2.max(h),
                }
            }

            let text_color = if obj.style().stroke == "black" || obj.style().stroke == "none" {
                "rgb(0,0,0)".to_string()
            } else {
                color_to_rgb(&obj.style().stroke)
            };

            for (positioned_text, slot) in texts.iter().zip(slots.iter()) {
                let mut y_offset = y_base;
                match slot {
                    TextVSlot::Above2 => y_offset += 0.5 * hc + ha1 + 0.5 * ha2,
                    TextVSlot::Above => y_offset += 0.5 * hc + 0.5 * ha1,
                    TextVSlot::Center => {}
                    TextVSlot::Below => y_offset -= 0.5 * hc + 0.5 * hb1,
                    TextVSlot::Below2 => y_offset -= 0.5 * hc + hb1 + 0.5 * hb2,
                }

                let svg_y_offset = scaler.px(Inches::inches(-y_offset));


                let uses_box_justification =
                    matches!(obj.class(), ClassName::Box | ClassName::Cylinder);
                let jw_inches = if uses_box_justification {
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
                // Combined with global fontscale variable
                // cref: pik_append_txt (pikchr.c:5183) - outputs as percentage
                let total_font_scale = fontscale * positioned_text.font_scale();
                let font_size = if (total_font_scale - 1.0).abs() > 0.001 {
                    let percent = total_font_scale * 100.0;
                    // Format with appropriate precision to avoid floating point artifacts
                    Some(fmt_num(percent) + "%")
                } else {
                    None
                };

                // Compute rotation transform for aligned text on line-like objects
                // cref: pik_append_txt (pikchr.c:2559-2568)
                let transform = if positioned_text.aligned {
                    if let Some(waypoints) = obj.waypoints() {
                        if waypoints.len() >= 2 {
                            let n = waypoints.len();
                            // Use first and last waypoints to compute line direction
                            // waypoints are in pikchr Y-up coordinates
                            let dx = waypoints[n - 1].x.raw() - waypoints[0].x.raw();
                            let dy = waypoints[n - 1].y.raw() - waypoints[0].y.raw();
                            if dx != 0.0 || dy != 0.0 {
                                // Negative because SVG Y is flipped
                                let angle = dy.atan2(dx) * -180.0 / std::f64::consts::PI;
                                // Rotation center is at (text_x, center.y) in SVG coordinates
                                // Use fmt_num_hi for angle to match C's %.10g precision
                                Some(format!(
                                    "rotate({} {},{})",
                                    fmt_num_hi(angle),
                                    fmt_num(text_x),
                                    fmt_num(center.y)
                                ))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                let text_element = Text {
                    x: Some(text_x),
                    y: Some(center.y + svg_y_offset),
                    transform,
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
                    content: decode_html_entities(&positioned_text.value).replace(' ', "\u{00A0}"),
                };
                svg_children.push(SvgNode::Text(text_element));
            }
        }

        // Recursively render text for sublist children
        if let Some(children) = obj.shape.children() {
            for child in children {
                render_object_text(
                    child, scaler, offset_x, max_y, charht, charwid, thickness, fontscale, svg_children,
                );
            }
        }
    }

    // Sort objects by layer for rendering (lower layers first = behind)
    // cref: pik_render (pikchr.c:5619) - renders by layer
    let mut sorted_objects: Vec<_> = ctx.object_list.iter().collect();
    sorted_objects.sort_by_key(|obj| obj.layer);

    // Helper to render a single object (shape + text), recursing into sublist children
    // This ensures text is rendered inline with each shape, matching C pikchr order
    // cref: boxRender (pikchr.c:3850), lineRender (pikchr.c:4266) - each xRender renders shape then calls pik_append_txt
    fn render_object_full(
        obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        dashwid: Inches,
        arrow_ht: Inches,
        arrow_wid: Inches,
        charht: f64,
        charwid: f64,
        thickness: f64,
        fontscale: f64,
        svg_children: &mut Vec<SvgNode>,
    ) {
        // For sublists, render each child (shape + text) in order
        if let Some(children) = obj.children() {
            for child in children {
                render_object_full(
                    child, scaler, offset_x, max_y, dashwid, arrow_ht, arrow_wid,
                    charht, charwid, thickness, fontscale, svg_children,
                );
            }
        } else {
            // Non-sublist: render shape then text immediately after
            if !obj.style().invisible {
                let shape = &obj.shape;
                let shape_nodes =
                    shape.render_svg(obj, scaler, offset_x, max_y, dashwid, arrow_ht, arrow_wid, Inches(thickness));
                svg_children.extend(shape_nodes);
            }
            // Render text for this object immediately after its shape
            render_object_text(
                obj, scaler, offset_x, max_y, charht, charwid, thickness, fontscale, svg_children,
            );
        }
    }

    // Render each object (shape + text together), sorted by layer
    // cref: pikchr.c:7289-7290 - charht and charwid are scaled by fontscale
    let charht = get_length(ctx, "charht", 0.14) * fontscale;
    let charwid = get_length(ctx, "charwid", 0.08) * fontscale;
    for obj in sorted_objects {
        render_object_full(
            obj, &scaler, offset_x, max_y, dashwid, arrow_ht, arrow_wid,
            charht, charwid, thickness, fontscale, &mut svg_children,
        );
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

/// Format a number matching C's %g format (6 significant figures, trailing zeros trimmed).
/// cref: pik_append_dis uses snprintf with %g format
pub(crate) fn fmt_num(value: f64) -> String {
    fmt_num_precision(value, 6)
}

/// Format a number with high precision (10 significant figures) matching C's %.10g format.
/// cref: pik_append_num uses snprintf with %.10g format
pub(crate) fn fmt_num_hi(value: f64) -> String {
    fmt_num_precision(value, 10)
}

/// Format a number with specified significant figures, trailing zeros trimmed.
fn fmt_num_precision(value: f64, sig_figs: i32) -> String {
    if value == 0.0 {
        return "0".to_string();
    }

    // Round to specified significant figures
    let abs_val = value.abs();
    let magnitude = abs_val.log10().floor() as i32;
    let scale = 10_f64.powi(sig_figs - 1 - magnitude);
    let rounded = (value * scale).round() / scale;

    // Format with enough decimal places, then trim
    let decimals = (sig_figs - 1 - magnitude).max(0) as usize;
    let s = format!("{:.prec$}", rounded, prec = decimals);
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    s.to_string()
}
