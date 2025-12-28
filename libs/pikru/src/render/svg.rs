//! SVG generation

use super::shapes::{Shape, ShapeRenderContext, svg_style_from_entries};
use super::{TextVSlot, compute_text_vslots};
use crate::types::{Length as Inches, Scaler};
use facet_svg::facet_xml::SerializeOptions;
use facet_svg::{Circle as SvgCircle, Points, Polygon, Style, Svg, SvgNode, Text, facet_xml};
use glam::{DVec2, dvec2};

use super::context::RenderContext;
use super::defaults;
use super::eval::{get_length, get_scalar};
use super::types::*;

/// Convert a color name to rgb() format like C pikchr
pub fn color_to_rgb(color: &str) -> String {
    color
        .parse::<crate::types::Color>()
        .unwrap()
        .to_rgb_string()
}

/// Convert a color to either CSS variable reference or rgb() format
pub fn color_to_string(color: &str, use_css_vars: bool) -> String {
    if use_css_vars {
        // Try to extract the color name for the CSS variable
        let name = color.to_lowercase();
        let normalized = match name.as_str() {
            "rgb(0,0,0)" | "black" => "black",
            "rgb(255,255,255)" | "white" => "white",
            "rgb(255,0,0)" | "red" => "red",
            "rgb(0,128,0)" | "green" => "green",
            "rgb(0,0,255)" | "blue" => "blue",
            "rgb(255,255,0)" | "yellow" => "yellow",
            "rgb(0,255,255)" | "cyan" => "cyan",
            "rgb(255,0,255)" | "magenta" => "magenta",
            "rgb(255,165,0)" | "orange" => "orange",
            "rgb(128,0,128)" | "purple" => "purple",
            "rgb(165,42,42)" | "brown" => "brown",
            "rgb(255,192,203)" | "pink" => "pink",
            "rgb(128,128,128)" | "gray" | "grey" => "gray",
            "rgb(211,211,211)" | "lightgray" | "lightgrey" => "lightgray",
            "rgb(169,169,169)" | "darkgray" | "darkgrey" => "darkgray",
            "rgb(192,192,192)" | "silver" => "silver",
            "none" | "off" => return "none".to_string(),
            _ => {
                // Unknown color, fall back to direct value
                return color_to_rgb(color);
            }
        };
        format!("var(--pik-{})", normalized)
    } else {
        color_to_rgb(color)
    }
}

/// Process backslash escape sequences in text content.
///
/// C pikchr treats backslash as an escape character:
/// - `\\` becomes a literal backslash character (which will render as `\`)
/// - `\x` (where x is any other char) removes the backslash and keeps x
///
/// This means `"\\a"` becomes `"a"` and `"\\\\"` becomes `"\"` (one backslash).
/// Note: This is NOT standard C escape processing - `\n` becomes `n`, not newline.
///
/// cref: pik_append_txt (pikchr.c:5271-5281) - processes backslashes in text output
fn process_backslash_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Find next backslash
        let mut j = i;
        while j < bytes.len() && bytes[j] != b'\\' {
            j += 1;
        }

        // Append all text before the backslash
        if j > i {
            result.push_str(&s[i..j]);
        }

        // Handle backslash if found
        if j < bytes.len() {
            // We're at a backslash
            if j + 1 == bytes.len() {
                // Backslash at end of string -> output literal backslash
                // cref: pikchr.c:5275-5277 - C outputs &#92; but facet-xml will escape it
                result.push('\\');
                break;
            } else if bytes[j + 1] == b'\\' {
                // Double backslash -> output literal backslash, skip both
                // cref: pikchr.c:5275-5277
                result.push('\\');
                i = j + 2;
            } else {
                // Backslash followed by other char -> skip backslash, output next char on next iteration
                i = j + 1;
            }
        } else {
            // No more backslashes
            break;
        }
    }

    result
}

/// Process HTML entities in text content for SVG output.
///
/// C pikchr behavior (pik_append_text in pikchr.c:2066-2110):
/// - If text contains `&` followed by alphanumerics and `;`, it's treated as an entity
///   and passed through unchanged (e.g., `&amp;` stays as `&amp;`, `&rightarrow;` stays as-is)
/// - Only `<` and `>` are always escaped to `&lt;` and `&gt;`
/// - Bare `&` not part of an entity is escaped to `&amp;`
///
/// Since facet-xml with preserve_entities:true will pass through entity-like sequences,
/// we just need to escape bare `&` that aren't part of entities, plus `<` and `>`.
///
/// cref: pik_isentity (pikchr.c:2043) - checks if text starts with HTML entity
/// cref: pik_append_text (pikchr.c:2066) - handles entity pass-through
fn process_entities_for_svg(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'<' => {
                result.push_str("&lt;");
                i += 1;
            }
            b'>' => {
                result.push_str("&gt;");
                i += 1;
            }
            b'&' => {
                // Check if this looks like an entity: &[#]?[a-zA-Z0-9]+;
                if is_entity_at(bytes, i) {
                    // Pass through the entity unchanged
                    result.push('&');
                    i += 1;
                } else {
                    // Bare & - escape it
                    result.push_str("&amp;");
                    i += 1;
                }
            }
            _ => {
                // Safe to push byte as char for ASCII, but we need to handle UTF-8
                if c < 128 {
                    result.push(c as char);
                    i += 1;
                } else {
                    // UTF-8 multibyte - find the full character
                    let remaining = &s[i..];
                    if let Some(ch) = remaining.chars().next() {
                        result.push(ch);
                        i += ch.len_utf8();
                    } else {
                        i += 1;
                    }
                }
            }
        }
    }

    result
}

/// Check if position i in bytes starts an HTML entity.
/// Matches: &[#]?[a-zA-Z0-9]+;
/// cref: pik_isentity (pikchr.c:2043)
fn is_entity_at(bytes: &[u8], i: usize) -> bool {
    if i >= bytes.len() || bytes[i] != b'&' {
        return false;
    }

    let mut j = i + 1;
    if j >= bytes.len() {
        return false;
    }

    // Optional # for numeric entities
    if bytes[j] == b'#' {
        j += 1;
        if j >= bytes.len() {
            return false;
        }
    }

    // Must have at least one alphanumeric
    let start = j;
    while j < bytes.len() {
        let c = bytes[j];
        if c == b';' {
            // Valid entity if we have at least 2 chars between & and ;
            // (e.g., &lt; is valid, &; is not)
            return j > start + 1 || (j > start && bytes[i + 1] != b'#');
        } else if c.is_ascii_alphanumeric() {
            j += 1;
        } else {
            return false;
        }
    }

    false
}

/// Generate CSS color definitions for light-dark mode
fn generate_color_css() -> Style {
    let colors = [
        ("black", "rgb(0,0,0)", "rgb(255,255,255)"),
        ("white", "rgb(255,255,255)", "rgb(0,0,0)"),
        ("red", "rgb(255,0,0)", "rgb(255,100,100)"),
        ("green", "rgb(0,128,0)", "rgb(100,255,100)"),
        ("blue", "rgb(0,0,255)", "rgb(100,100,255)"),
        ("yellow", "rgb(255,255,0)", "rgb(255,255,150)"),
        ("cyan", "rgb(0,255,255)", "rgb(150,255,255)"),
        ("magenta", "rgb(255,0,255)", "rgb(255,150,255)"),
        ("orange", "rgb(255,165,0)", "rgb(255,200,100)"),
        ("purple", "rgb(128,0,128)", "rgb(200,100,200)"),
        ("brown", "rgb(165,42,42)", "rgb(210,150,150)"),
        ("pink", "rgb(255,192,203)", "rgb(255,220,230)"),
        ("gray", "rgb(128,128,128)", "rgb(160,160,160)"),
        ("grey", "rgb(128,128,128)", "rgb(160,160,160)"),
        ("lightgray", "rgb(211,211,211)", "rgb(100,100,100)"),
        ("lightgrey", "rgb(211,211,211)", "rgb(100,100,100)"),
        ("darkgray", "rgb(169,169,169)", "rgb(200,200,200)"),
        ("darkgrey", "rgb(169,169,169)", "rgb(200,200,200)"),
        ("silver", "rgb(192,192,192)", "rgb(128,128,128)"),
        ("none", "none", "none"),
    ];

    let mut css = String::from(":root {\n");
    for (name, light, dark) in &colors {
        css.push_str(&format!(
            "  --pik-{}: light-dark({}, {});\n",
            name, light, dark
        ));
    }
    css.push_str("}\n");

    Style {
        type_: Some("text/css".to_string()),
        content: css,
    }
}

/// Generate SVG from render context
// cref: pik_render (pikchr.c:7253) - main SVG output function
pub fn generate_svg(
    ctx: &RenderContext,
    options: &super::RenderOptions,
) -> Result<String, miette::Report> {
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
    crate::log::debug!(
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

    crate::log::debug!(
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

    // Add CSS variables style block if enabled
    if options.css_variables {
        svg_children.push(SvgNode::Style(generate_color_css()));
    }

    // SVG header - C pikchr only adds width/height when scale != 1.0
    let viewbox_width = scaler.px(view_width);
    let viewbox_height = scaler.px(view_height);

    // Create the main SVG element
    let viewbox = format!("0 0 {} {}", fmt_num(viewbox_width), fmt_num(viewbox_height));
    let mut svg = Svg {
        width: None,
        height: None,
        view_box: Some(viewbox),
        children: Vec::new(),
    };

    // C pikchr: when scale != 1.0, display width = viewBox width * scale
    // cref: pik_render (pikchr.c:4626-4633) - C rounds viewbox to int first, then scales and rounds again
    // This matches the two-step rounding: wSVG = pik_round(rScale*w), then wSVG = pik_round(wSVG*pikScale)
    let is_scaled = !(0.99..=1.01).contains(&scale);
    if is_scaled {
        let viewbox_width_int = viewbox_width as i32;
        let viewbox_height_int = viewbox_height as i32;
        let display_width = ((viewbox_width_int as f64) * scale) as i32;
        let display_height = ((viewbox_height_int as f64) * scale) as i32;
        svg.width = Some(display_width.to_string());
        svg.height = Some(display_height.to_string());
    }

    // Arrowheads are now rendered inline as polygon elements (matching C pikchr)

    // Helper to render text for an object (and recursively for sublist children)
    #[allow(clippy::too_many_arguments)]
    fn render_object_text(
        obj: &RenderedObject,
        scaler: &Scaler,
        offset_x: Inches,
        max_y: Inches,
        charht: f64,
        charwid: f64,
        _thickness: f64,
        fontscale: f64,
        use_css_vars: bool,
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
                color_to_string("black", use_css_vars)
            } else {
                color_to_string(&obj.style().stroke, use_css_vars)
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

                let uses_box_justification = matches!(
                    obj.class(),
                    ClassName::Box | ClassName::Cylinder | ClassName::File | ClassName::Oval
                );
                // cref: pik_append_txt (pikchr.c:2467) - jw uses object's sw, not global thickness
                let jw_inches = if uses_box_justification {
                    0.5 * (obj.width().0 - 0.5 * (charwid + obj_sw))
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
                    style: String::new(),
                    font_family,
                    font_style,
                    font_weight,
                    font_size,
                    text_anchor: Some(anchor.to_string()),
                    dominant_baseline: Some("central".to_string()),
                    content: {
                        // Process in order: backslash escapes first, then entities for SVG, then spaces
                        let text = process_backslash_escapes(&positioned_text.value);
                        let text = process_entities_for_svg(&text);
                        text.replace(' ', "\u{00A0}")
                    },
                };
                svg_children.push(SvgNode::Text(text_element));
            }
        }

        // Recursively render text for sublist children
        if let Some(children) = obj.shape.children() {
            for child in children {
                render_object_text(
                    child,
                    scaler,
                    offset_x,
                    max_y,
                    charht,
                    charwid,
                    _thickness,
                    fontscale,
                    use_css_vars,
                    svg_children,
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
    #[allow(clippy::too_many_arguments)]
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
        use_css_vars: bool,
        svg_children: &mut Vec<SvgNode>,
    ) {
        // For sublists, render each child (shape + text) sorted by layer
        // cref: pik_render (pikchr.c:5619) - renders by layer, applies to sublist children too
        if let Some(children) = obj.children() {
            // Sort children by layer so "behind" attribute works correctly
            let mut sorted_children: Vec<_> = children.iter().collect();
            sorted_children.sort_by_key(|c| c.layer);
            for child in sorted_children {
                render_object_full(
                    child,
                    scaler,
                    offset_x,
                    max_y,
                    dashwid,
                    arrow_ht,
                    arrow_wid,
                    charht,
                    charwid,
                    thickness,
                    fontscale,
                    use_css_vars,
                    svg_children,
                );
            }
        } else {
            // Non-sublist: render shape then text immediately after
            if !obj.style().invisible {
                let shape = &obj.shape;
                let ctx = ShapeRenderContext {
                    scaler,
                    offset_x,
                    max_y,
                    dashwid,
                    arrow_len: arrow_ht,
                    arrow_wid,
                    thickness: Inches(thickness),
                    use_css_vars,
                };
                let shape_nodes = shape.render_svg(obj, &ctx);
                svg_children.extend(shape_nodes);
            }
            // Render text for this object immediately after its shape
            render_object_text(
                obj,
                scaler,
                offset_x,
                max_y,
                charht,
                charwid,
                thickness,
                fontscale,
                use_css_vars,
                svg_children,
            );
        }
    }

    // Render each object (shape + text together), sorted by layer
    // cref: pikchr.c:7289-7290 - charht and charwid are scaled by fontscale
    let charht = get_length(ctx, "charht", 0.14) * fontscale;
    let charwid = get_length(ctx, "charwid", 0.08) * fontscale;
    for obj in sorted_objects.iter() {
        render_object_full(
            obj,
            &scaler,
            offset_x,
            max_y,
            dashwid,
            arrow_ht,
            arrow_wid,
            charht,
            charwid,
            thickness,
            fontscale,
            options.css_variables,
            &mut svg_children,
        );
    }

    // cref: pik_elist_render (pikchr.c:4497-4518) - render debug labels if debug_label_color is set
    // If debug_label_color is defined and non-negative, render a dot + label at each named object/position
    if let Some(crate::types::EvalValue::Color(color_val)) = ctx.variables.get("debug_label_color")
    {
        // Convert u32 color value to rgb() string
        let r = ((*color_val >> 16) & 0xFF) as u8;
        let g = ((*color_val >> 8) & 0xFF) as u8;
        let b = (*color_val & 0xFF) as u8;
        let color_str = format!("rgb({},{},{})", r, g, b);

        let dot_rad = 0.015; // Same as C: dot.rad = 0.015
        let dot_rad_px = scaler.px(Inches(dot_rad));
        let sw_px = fmt_num(scaler.px(Inches(0.015))); // Same as C: dot.sw = 0.015

        // Helper to render a debug label at a position
        let mut render_debug_label = |name: &str, center: DVec2| {
            // Render dot (filled circle)
            let circle = SvgCircle {
                cx: Some(center.x),
                cy: Some(center.y),
                r: Some(dot_rad_px),
                fill: Some(color_str.clone()),
                stroke: Some(color_str.clone()),
                stroke_width: Some(sw_px.clone()),
                stroke_dasharray: None,
                style: String::new(),
            };
            svg_children.push(SvgNode::Circle(circle));

            // Render label text above the dot
            // cref: pik_elist_render (pikchr.c:4509) - aTxt[0].eCode = TP_ABOVE
            let text_y = center.y - scaler.px(Inches(charht * 0.5));
            let text_element = Text {
                x: Some(center.x),
                y: Some(text_y),
                transform: None,
                fill: Some(color_str.clone()),
                stroke: None,
                stroke_width: None,
                style: String::new(),
                font_family: None,
                font_style: None,
                font_weight: None,
                font_size: None,
                text_anchor: Some("middle".to_string()),
                dominant_baseline: Some("auto".to_string()),
                content: name.to_string(),
            };
            svg_children.push(SvgNode::Text(text_element));
        };

        // Render debug labels for objects with explicit names
        for obj in sorted_objects.iter() {
            if let Some(ref name) = obj.name {
                if obj.name_is_explicit {
                    let center = obj.center().to_svg(&scaler, offset_x, max_y);
                    render_debug_label(name, center);
                }
            }
        }

        // Render debug labels for named positions (e.g., `OUT: 6.3in right of previous.e`)
        for (name, pos) in ctx.named_positions.iter() {
            let center = pos.to_svg(&scaler, offset_x, max_y);
            render_debug_label(name, center);
        }
    }

    // Set children on the SVG element
    svg.children = svg_children;

    // Create wrapper function for fmt_num to match the expected signature
    fn format_float(value: f64, writer: &mut dyn std::io::Write) -> Result<(), std::io::Error> {
        write!(writer, "{}", fmt_num(value))
    }

    // Serialize to string using facet_xml with custom f64 formatter to match C pikchr precision
    let options_ser = SerializeOptions {
        float_formatter: Some(format_float),
        preserve_entities: true,
        ..Default::default()
    };
    facet_xml::to_string_with_options(&svg, &options_ser)
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
    use_css_vars: bool,
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

    let fill_color = color_to_string(&style.stroke, use_css_vars);

    Some(Polygon {
        points,
        fill: None,
        stroke: None,
        stroke_width: None,
        stroke_dasharray: None,
        style: svg_style_from_entries(vec![("fill", fill_color)]),
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
