//! SVG rendering for pikchr diagrams
//!
//! This module is organized into submodules:
//! - `defaults`: Default sizes and settings
//! - `types`: Core types like Value, PositionedText, RenderedObject, ObjectClass, ObjectStyle
//! - `context`: RenderContext for tracking state during rendering
//! - `eval`: Expression evaluation functions
//! - `geometry`: Chop functions and path creation
//! - `svg`: SVG generation

pub mod context;
pub mod defaults;
pub mod eval;
pub mod geometry;
pub mod shapes;
pub mod svg;
pub mod types;

// Re-export commonly used items
pub use context::RenderContext;
pub use types::*;

use crate::ast::*;
use crate::types::{EvalValue, Length as Inches, OffsetIn, Point, Size};
use eval::{
    edge_point_offset, endpoint_object_from_position, eval_color, eval_expr, eval_len,
    eval_position, eval_rvalue, eval_scalar, resolve_object,
};
use svg::generate_svg;

// TODO: Move these to appropriate submodules

/// Proportional character widths from C pikchr's awChar table.
#[rustfmt::skip]
pub const AW_CHAR: [u8; 95] = [
    45,  55,  62, 115,  90, 132, 125,  40,
    55,  55,  71, 115,  45,  48,  45,  50,
    91,  91,  91,  91,  91,  91,  91,  91,
    91,  91,  50,  50, 120, 120, 120,  78,
   142, 102, 105, 110, 115, 105,  98, 105,
   125,  58,  58, 107,  95, 145, 125, 115,
    95, 115, 107,  95,  97, 118, 102, 150,
   100,  93, 100,  58,  50,  58, 119,  72,
    72,  86,  92,  80,  92,  85,  52,  92,
    92,  47,  47,  88,  48, 135,  92,  86,
    92,  92,  69,  75,  58,  92,  80, 121,
    81,  80,  76,  91,  49,  91, 118,
];

/// Calculate text width using proportional character widths like C pikchr.
pub fn pik_text_length(text: &str) -> u32 {
    let mut cnt: u32 = 0;
    for c in text.chars() {
        if c >= ' ' && c <= '~' {
            cnt += AW_CHAR[(c as usize) - 0x20] as u32;
        } else {
            cnt += 100;
        }
    }
    cnt
}

/// Calculate text width in inches using proportional character widths.
pub fn text_width_inches(text: &str, charwid: f64) -> f64 {
    let length_hundredths = pik_text_length(text);
    length_hundredths as f64 * charwid * 0.01
}

/// Render a pikchr program to SVG
pub fn render(program: &Program) -> Result<String, miette::Report> {
    let mut ctx = RenderContext::new();
    let mut print_lines: Vec<String> = Vec::new();

    // Process all statements
    for stmt in &program.statements {
        render_statement(&mut ctx, stmt, &mut print_lines)?;
    }

    // If there are print lines and no drawables, emit print output (HTML with <br>)
    if ctx.object_list.is_empty() && !print_lines.is_empty() {
        let mut out = String::new();
        for line in print_lines {
            out.push_str(&line);
            out.push_str("<br>\n");
        }
        return Ok(out);
    }

    // If nothing was drawn and no prints, emit empty comment like C
    if ctx.object_list.is_empty() {
        return Ok("<!-- empty pikchr diagram -->\n".to_string());
    }

    // Generate SVG
    generate_svg(&ctx)
}

fn render_statement(
    ctx: &mut RenderContext,
    stmt: &Statement,
    print_lines: &mut Vec<String>,
) -> Result<(), miette::Report> {
    match stmt {
        Statement::Direction(dir) => {
            ctx.direction = *dir;
        }
        Statement::Assignment(assign) => {
            let value = eval_rvalue(ctx, &assign.rvalue)?;
            match &assign.lvalue {
                LValue::Variable(name) => {
                    ctx.variables.insert(name.clone(), EvalValue::from(value));
                }
                _ => {
                    // fill, color, thickness - global settings
                }
            }
        }
        Statement::Object(obj_stmt) => {
            let obj = render_object_stmt(ctx, obj_stmt, None)?;
            ctx.add_object(obj);
        }
        Statement::Labeled(labeled) => {
            match &labeled.content {
                LabeledContent::Object(obj_stmt) => {
                    let obj = render_object_stmt(ctx, obj_stmt, Some(labeled.label.clone()))?;
                    ctx.add_object(obj);
                }
                LabeledContent::Position(_pos) => {
                    // Named position - just record it
                }
            }
        }
        Statement::Print(p) => {
            let mut parts = Vec::new();
            for arg in &p.args {
                let s = match arg {
                    PrintArg::String(s) => s.clone(),
                    PrintArg::Expr(e) => {
                        let val = eval_expr(ctx, e)?;
                        match val {
                            Value::Scalar(v) => format!("{}", v),
                            Value::Len(l) => format!("{}", l.0),
                        }
                    }
                    PrintArg::PlaceName(name) => name.clone(),
                };
                parts.push(s);
            }
            print_lines.push(parts.join(" "));
        }
        Statement::Assert(_) => {
            // Not rendered
        }
        Statement::Define(def) => {
            // Store macro definition (later definitions override earlier ones)
            // Strip the surrounding braces from the body
            let body = def.body.trim();
            let body = if body.starts_with('{') && body.ends_with('}') {
                &body[1..body.len() - 1]
            } else {
                body
            };
            ctx.macros.insert(def.name.clone(), body.to_string());
        }
        Statement::MacroCall(call) => {
            // Expand and render macro
            if let Some(body) = ctx.macros.get(&call.name).cloned() {
                // Parse and render the macro body
                let parsed = crate::parse::parse(&body)?;
                for inner_stmt in &parsed.statements {
                    render_statement(ctx, inner_stmt, print_lines)?;
                }
            }
            // If macro not found, treat as custom object type (ignore for now)
        }
        Statement::Error(e) => {
            // Error statement produces an intentional error
            return Err(miette::miette!("error: {}", e.message));
        }
    }
    Ok(())
}

/// Expand a bounding box to include a rendered object (recursing into sublists)
pub fn expand_object_bounds(bounds: &mut BoundingBox, obj: &RenderedObject) {
    match obj.class {
        ObjectClass::Line | ObjectClass::Arrow | ObjectClass::Spline | ObjectClass::Arc => {
            // C pikchr does not enlarge the bounding box by stroke width for line-like objects.
            // Using the raw waypoints here keeps the computed viewBox identical to the C output.
            for pt in &obj.waypoints {
                bounds.expand_point(Point::new(pt.x, pt.y));
            }
            // Include text labels for lines (they extend above and below)
            if !obj.text.is_empty() {
                let charht = Inches(defaults::FONT_SIZE);
                let (above_count, below_count) = count_text_above_below(&obj.text);
                // Text extends above and below the line center
                let text_above = charht * above_count as f64;
                let text_below = charht * below_count as f64;
                bounds.expand_point(Point::new(obj.center.x, obj.center.y - text_above));
                bounds.expand_point(Point::new(obj.center.x, obj.center.y + text_below));
            }
        }
        ObjectClass::Sublist => {
            for child in &obj.children {
                expand_object_bounds(bounds, child);
            }
        }
        ObjectClass::Text => {
            // For text objects, iterate ALL text spans and expand bounds for each.
            // Handle ljust/rjust - text extends in one direction from anchor point.
            let charht = Inches(defaults::FONT_SIZE);
            let charwid = defaults::CHARWID;

            if obj.text.is_empty() {
                // No text - just use object dimensions
                bounds.expand_rect(
                    obj.center,
                    Size {
                        w: obj.width,
                        h: obj.height,
                    },
                );
            } else {
                for text in &obj.text {
                    let text_w = Inches(text_width_inches(&text.value, charwid));
                    let hh = charht / 2.0;

                    if text.rjust {
                        // rjust: text extends to the LEFT of center (anchor at right edge)
                        bounds.expand_point(Point::new(obj.center.x - text_w, obj.center.y - hh));
                        bounds.expand_point(Point::new(obj.center.x, obj.center.y + hh));
                    } else if text.ljust {
                        // ljust: text extends to the RIGHT of center (anchor at left edge)
                        bounds.expand_point(Point::new(obj.center.x, obj.center.y - hh));
                        bounds.expand_point(Point::new(obj.center.x + text_w, obj.center.y + hh));
                    } else {
                        // Centered text
                        bounds.expand_rect(
                            obj.center,
                            Size {
                                w: text_w,
                                h: charht,
                            },
                        );
                    }
                }
            }
        }
        _ => {
            // For invisible objects, only include text bounds, not shape bounds
            if obj.style.invisible && !obj.text.is_empty() {
                let charht = Inches(defaults::FONT_SIZE);
                let charwid = defaults::CHARWID;
                for text in &obj.text {
                    let text_w = Inches(text_width_inches(&text.value, charwid));
                    let hh = charht / 2.0;
                    let hw = text_w / 2.0;
                    bounds.expand_point(Point::new(obj.center.x - hw, obj.center.y - hh));
                    bounds.expand_point(Point::new(obj.center.x + hw, obj.center.y + hh));
                }
            } else if !obj.style.invisible {
                bounds.expand_rect(
                    obj.center,
                    Size {
                        w: obj.width,
                        h: obj.height,
                    },
                )
            }
        }
    }
}

/// Count text labels above and below for lines
fn count_text_above_below(texts: &[PositionedText]) -> (usize, usize) {
    let mut above = 0;
    let mut below = 0;
    let mut center = 0;

    for text in texts {
        if text.above {
            above += 1;
        } else if text.below {
            below += 1;
        } else {
            center += 1;
        }
    }

    // For lines, center text is distributed: first half above, second half below
    // If only center texts (no explicit above/below), distribute evenly
    if above == 0 && below == 0 && center > 0 {
        // C pikchr: first text above, rest below (for 2 texts: 1 above, 1 below)
        above = (center + 1) / 2; // ceiling division
        below = center / 2;
    }

    (above, below)
}

/// Compute the bounding box of a list of rendered objects (in local coordinates)
fn compute_children_bounds(children: &[RenderedObject]) -> BoundingBox {
    let mut bounds = BoundingBox::new();
    for child in children {
        expand_object_bounds(&mut bounds, child);
    }
    bounds
}

/// Create a partial RenderedObject for `this` keyword resolution during attribute processing.
/// This allows expressions like `ht this.wid` to access the current object's computed properties.
fn make_partial_object(
    class: ObjectClass,
    width: Inches,
    height: Inches,
    style: &ObjectStyle,
) -> RenderedObject {
    RenderedObject {
        name: None,
        class,
        center: pin(0.0, 0.0),
        width,
        height,
        start: pin(0.0, 0.0),
        end: pin(0.0, 0.0),
        start_attachment: None,
        end_attachment: None,
        waypoints: Vec::new(),
        text: Vec::new(),
        style: style.clone(),
        children: Vec::new(),
    }
}

/// Update the current_object in context with latest dimensions
fn update_current_object(
    ctx: &mut RenderContext,
    class: ObjectClass,
    width: Inches,
    height: Inches,
    style: &ObjectStyle,
) {
    ctx.current_object = Some(make_partial_object(class, width, height, style));
}

fn render_object_stmt(
    ctx: &mut RenderContext,
    obj_stmt: &ObjectStatement,
    name: Option<String>,
) -> Result<RenderedObject, miette::Report> {
    // Determine base object properties
    let (class, mut width, mut height) = match &obj_stmt.basetype {
        BaseType::Class(cn) => match cn {
            ClassName::Box => (ObjectClass::Box, defaults::BOX_WIDTH, defaults::BOX_HEIGHT),
            ClassName::Circle => (
                ObjectClass::Circle,
                defaults::CIRCLE_RADIUS * 2.0,
                defaults::CIRCLE_RADIUS * 2.0,
            ),
            ClassName::Ellipse => (
                ObjectClass::Ellipse,
                defaults::BOX_WIDTH,
                defaults::BOX_HEIGHT,
            ),
            ClassName::Oval => (
                ObjectClass::Oval,
                defaults::OVAL_WIDTH,
                defaults::OVAL_HEIGHT,
            ),
            ClassName::Cylinder => (
                ObjectClass::Cylinder,
                defaults::BOX_WIDTH,
                defaults::BOX_HEIGHT,
            ),
            ClassName::Diamond => (
                ObjectClass::Diamond,
                defaults::DIAMOND_WIDTH,
                defaults::DIAMOND_HEIGHT,
            ),
            ClassName::File => (
                ObjectClass::File,
                defaults::FILE_WIDTH,
                defaults::FILE_HEIGHT,
            ),
            ClassName::Line => (ObjectClass::Line, defaults::LINE_WIDTH, Inches::ZERO),
            ClassName::Arrow => (ObjectClass::Arrow, defaults::LINE_WIDTH, Inches::ZERO),
            ClassName::Spline => (ObjectClass::Spline, defaults::LINE_WIDTH, Inches::ZERO),
            ClassName::Arc => (ObjectClass::Arc, defaults::LINE_WIDTH, defaults::LINE_WIDTH),
            ClassName::Move => (ObjectClass::Move, defaults::LINE_WIDTH, Inches::ZERO),
            ClassName::Dot => (ObjectClass::Dot, Inches(0.03), Inches(0.03)),
            ClassName::Text => (ObjectClass::Text, Inches::ZERO, Inches::ZERO),
        },
        BaseType::Text(s, _) => {
            // Use proportional character widths like C pikchr
            let charwid = defaults::CHARWID;
            let charht = defaults::FONT_SIZE;
            let w = text_width_inches(&s.value, charwid);
            let h = charht;
            (ObjectClass::Text, Inches(w), Inches(h))
        }
        BaseType::Sublist(_) => {
            // Placeholder - will be computed from rendered children below
            (ObjectClass::Sublist, Inches::ZERO, Inches::ZERO)
        }
    };

    // For sublists, render children early to compute their actual bounds
    let (sublist_children, sublist_bounds) =
        if let BaseType::Sublist(statements) = &obj_stmt.basetype {
            let children = render_sublist(ctx, statements)?;
            let bounds = compute_children_bounds(&children);
            (Some(children), Some(bounds))
        } else {
            (None, None)
        };

    // Update width/height for sublists based on computed bounds
    if let Some(ref bounds) = sublist_bounds {
        if !bounds.is_empty() {
            width = bounds.width();
            height = bounds.height();
        } else {
            width = defaults::BOX_WIDTH;
            height = defaults::BOX_HEIGHT;
        }
    }

    let mut style = ObjectStyle::default();
    let mut text = Vec::new();
    let mut explicit_position: Option<PointIn> = None;
    let mut from_position: Option<PointIn> = None;
    let mut to_position: Option<PointIn> = None;
    let mut from_attachment: Option<EndpointObject> = None;
    let mut to_attachment: Option<EndpointObject> = None;
    // Accumulated direction offsets for compound moves like "up 1 right 2"
    let mut direction_offset = OffsetIn::ZERO;
    let mut has_direction_move: bool = false;
    let mut even_clause: Option<(Direction, Position)> = None;
    let mut then_clauses: Vec<ThenClause> = Vec::new();
    let mut with_clause: Option<(EdgePoint, PointIn)> = None; // (edge, target_position)

    // Extract text from basetype
    if let BaseType::Text(s, pos) = &obj_stmt.basetype {
        text.push(PositionedText::from_textposition(
            s.value.clone(),
            pos.as_ref(),
        ));
    }

    // Default arrow style for arrows
    if class == ObjectClass::Arrow {
        style.arrow_end = true;
    }

    // Pre-scan for `fit` and string attributes to apply fit sizing early.
    // This allows `this.wid` to access the fit-computed width during attribute processing.
    let has_fit = obj_stmt
        .attributes
        .iter()
        .any(|a| matches!(a, Attribute::Fit));
    if has_fit {
        // Collect all text (from basetype and string attributes)
        let mut fit_text = text.clone();
        for attr in &obj_stmt.attributes {
            if let Attribute::StringAttr(s, pos) = attr {
                fit_text.push(PositionedText::from_textposition(
                    s.value.clone(),
                    pos.as_ref(),
                ));
            }
        }
        // Apply fit sizing
        if !fit_text.is_empty() {
            let char_width = defaults::FONT_SIZE * 0.6;
            let padding = defaults::FONT_SIZE;
            let max_text_width = fit_text
                .iter()
                .map(|t| t.value.len() as f64 * char_width)
                .fold(0.0_f64, |a, b| a.max(b));
            let center_lines = fit_text.iter().filter(|t| !t.above && !t.below).count();
            let fit_width = Inches(max_text_width + padding * 2.0);
            let fit_height = Inches((center_lines as f64 * defaults::FONT_SIZE) + padding * 2.0);
            width = width.max(fit_width);
            height = height.max(fit_height);
            style.fit = true;
        }
    }

    // Initialize current_object for `this` keyword support
    update_current_object(ctx, class, width, height, &style);

    // Process attributes
    for attr in &obj_stmt.attributes {
        match attr {
            Attribute::NumProperty(prop, relexpr) => {
                let raw_val = eval_len(ctx, &relexpr.expr)?;
                // If percent, multiply by current value (or default) to get actual value
                let val = if relexpr.is_percent {
                    let base = match prop {
                        NumProperty::Width => width,
                        NumProperty::Height => height,
                        NumProperty::Radius => {
                            match class {
                                ObjectClass::Circle | ObjectClass::Ellipse | ObjectClass::Arc => {
                                    width / 2.0 // current radius
                                }
                                _ => style.corner_radius,
                            }
                        }
                        NumProperty::Diameter => width,
                        NumProperty::Thickness => style.stroke_width,
                    };
                    // raw_val is the percentage as a number (e.g., 50 for 50%)
                    // Convert to fraction and multiply by base
                    base * (raw_val.raw() / 100.0)
                } else {
                    raw_val
                };
                match prop {
                    NumProperty::Width => {
                        width = val;
                        update_current_object(ctx, class, width, height, &style);
                    }
                    NumProperty::Height => {
                        height = val;
                        update_current_object(ctx, class, width, height, &style);
                    }
                    NumProperty::Radius => {
                        // For circles/ellipses, radius sets size (diameter = 2 * radius)
                        // For boxes, radius sets corner rounding
                        match class {
                            ObjectClass::Circle | ObjectClass::Ellipse | ObjectClass::Arc => {
                                width = val * 2.0;
                                height = val * 2.0;
                                update_current_object(ctx, class, width, height, &style);
                            }
                            _ => {
                                style.corner_radius = val;
                            }
                        }
                    }
                    NumProperty::Diameter => {
                        width = val;
                        height = val;
                        update_current_object(ctx, class, width, height, &style);
                    }
                    NumProperty::Thickness => style.stroke_width = val,
                }
            }
            Attribute::DashProperty(prop, _) => match prop {
                DashProperty::Dashed => style.dashed = true,
                DashProperty::Dotted => style.dotted = true,
            },
            Attribute::ColorProperty(prop, rvalue) => {
                let color = eval_color(rvalue);
                match prop {
                    ColorProperty::Fill => style.fill = color,
                    ColorProperty::Color => style.stroke = color,
                }
            }
            Attribute::BoolProperty(prop) => match prop {
                BoolProperty::Invisible => style.invisible = true,
                BoolProperty::ArrowRight => style.arrow_end = true,
                BoolProperty::ArrowLeft => style.arrow_start = true,
                BoolProperty::ArrowBoth => {
                    style.arrow_start = true;
                    style.arrow_end = true;
                }
                BoolProperty::Thick => style.stroke_width = defaults::STROKE_WIDTH * 2.0,
                BoolProperty::Thin => style.stroke_width = defaults::STROKE_WIDTH * 0.5,
                _ => {}
            },
            Attribute::StringAttr(s, pos) => {
                text.push(PositionedText::from_textposition(
                    s.value.clone(),
                    pos.as_ref(),
                ));
            }
            Attribute::At(pos) => {
                tracing::debug!(?pos, "Attribute::At position");
                if let Ok(p) = eval_position(ctx, pos) {
                    tracing::debug!(x = p.x.0, y = p.y.0, "Attribute::At evaluated");
                    explicit_position = Some(p);
                }
            }
            Attribute::From(pos) => {
                if let Ok(p) = eval_position(ctx, pos) {
                    from_position = Some(p);
                    if from_attachment.is_none() {
                        from_attachment = endpoint_object_from_position(ctx, pos);
                    }
                }
            }
            Attribute::To(pos) => {
                if let Ok(p) = eval_position(ctx, pos) {
                    to_position = Some(p);
                    if to_attachment.is_none() {
                        to_attachment = endpoint_object_from_position(ctx, pos);
                    }
                }
            }
            Attribute::DirectionMove(_go, dir, dist) => {
                has_direction_move = true;
                let distance = if let Some(relexpr) = dist {
                    if let Ok(d) = eval_len(ctx, &relexpr.expr) {
                        // Handle percent: 40% means 40% of the default line width
                        if relexpr.is_percent {
                            width * (d.raw() / 100.0)
                        } else {
                            d
                        }
                    } else {
                        width // default distance
                    }
                } else {
                    width // default distance
                };
                // Accumulate offset based on direction
                direction_offset += dir.offset(distance);
            }
            Attribute::DirectionEven(_go, dir, pos) => {
                even_clause = Some((*dir, pos.clone()));
            }
            Attribute::DirectionUntilEven(_go, dir, pos) => {
                even_clause = Some((*dir, pos.clone()));
            }
            Attribute::BareExpr(relexpr) => {
                // A bare expression is typically a distance applied in ctx.direction
                if let Ok(d) = eval_len(ctx, &relexpr.expr) {
                    // Handle percent: 40% means 40% of the default line width
                    let val = if relexpr.is_percent {
                        width * (d.raw() / 100.0)
                    } else {
                        d
                    };
                    has_direction_move = true;
                    // Apply in context direction
                    direction_offset += ctx.direction.offset(val);
                }
            }
            Attribute::Then(Some(clause)) => {
                then_clauses.push(clause.clone());
            }
            Attribute::Chop => {
                style.chop = true;
            }
            Attribute::Fit => {
                style.fit = true;
            }
            Attribute::Same(obj_ref) => {
                // Copy properties from referenced object
                match obj_ref {
                    Some(obj) => {
                        if let Some(source) = resolve_object(ctx, obj) {
                            width = source.width;
                            height = source.height;
                            style = source.style.clone();
                        }
                    }
                    None => {
                        // "same" without object - use last object of same class
                        if let Some(source) = ctx.get_last_object(Some(class)) {
                            width = source.width;
                            height = source.height;
                            style = source.style.clone();
                        }
                    }
                }
            }
            Attribute::Close => {
                style.close_path = true;
            }
            Attribute::With(clause) => {
                // Store the edge and target position for later center calculation
                let edge = match &clause.edge {
                    WithEdge::DotEdge(ep) | WithEdge::EdgePoint(ep) => *ep,
                };
                if let Ok(target) = eval_position(ctx, &clause.position) {
                    with_clause = Some((edge, target));
                }
            }
            _ => {}
        }
    }

    // Apply fit: auto-size box to fit text content
    if style.fit && !text.is_empty() {
        // Estimate text width: ~7 pixels per character for a 12pt font
        let char_width = defaults::FONT_SIZE * 0.6;
        let padding = defaults::FONT_SIZE; // Padding around text

        // Find the widest text line
        let max_text_width = text
            .iter()
            .map(|t| t.value.len() as f64 * char_width)
            .fold(0.0_f64, |a, b| a.max(b));

        // Count lines (excluding above/below positioned text)
        let center_lines = text.iter().filter(|t| !t.above && !t.below).count();

        let fit_width = Inches(max_text_width + padding * 2.0);
        let fit_height = Inches((center_lines as f64 * defaults::FONT_SIZE) + padding * 2.0);

        // Only expand, don't shrink
        width = width.max(fit_width);
        height = height.max(fit_height);
    }

    // Calculate position based on object type
    let (center, start, end, waypoints) = if from_position.is_some()
        || to_position.is_some()
        || has_direction_move
        || !then_clauses.is_empty()
        || even_clause.is_some()
    {
        // Line-like objects with explicit from/to, direction moves, or then clauses
        // Determine start position based on direction of movement
        let start = if let Some(pos) = from_position {
            pos
        } else if has_direction_move {
            // Get exit edge of last object based on accumulated offset direction
            if let Some(last_obj) = ctx.last_object() {
                // For line-like objects, use the end point directly
                match last_obj.class {
                    ObjectClass::Line
                    | ObjectClass::Arrow
                    | ObjectClass::Spline
                    | ObjectClass::Arc
                    | ObjectClass::Move => last_obj.end,
                    _ => {
                        // For box-like objects, calculate exit edge based on direction
                        let (hw, hh) = (last_obj.width / 2.0, last_obj.height / 2.0);
                        let c = last_obj.center;
                        let exit_x = if direction_offset.dx > Inches::ZERO {
                            c.x + hw // moving right, exit from right edge
                        } else if direction_offset.dx < Inches::ZERO {
                            c.x - hw // moving left, exit from left edge
                        } else {
                            c.x // no horizontal movement, use center
                        };
                        // In SVG coordinates: positive Y offset = down = bottom edge
                        let exit_y = if direction_offset.dy > Inches::ZERO {
                            c.y + hh // moving down (positive Y), exit from bottom edge
                        } else if direction_offset.dy < Inches::ZERO {
                            c.y - hh // moving up (negative Y), exit from top edge
                        } else {
                            c.y // no vertical movement, use center
                        };
                        Point::new(exit_x, exit_y)
                    }
                }
            } else {
                ctx.position
            }
        } else {
            ctx.position
        };

        if let Some((dir, pos_expr)) = even_clause.as_ref() {
            // Single-segment move until even with target
            let target = eval_position(ctx, pos_expr)?;
            let end = match dir {
                Direction::Right | Direction::Left => Point::new(target.x, start.y),
                Direction::Up | Direction::Down => Point::new(start.x, target.y),
            };
            let center = start.midpoint(end);
            (center, start, end, vec![start, end])
        } else {
            // Build waypoints starting from start
            let mut points = vec![start];
            let mut current_pos = start;

            if to_position.is_some() && then_clauses.is_empty() && !has_direction_move {
                // from X to Y - just two points
                points.push(to_position.unwrap());
            } else if has_direction_move {
                // Apply accumulated offsets as single diagonal move (C pikchr behavior)
                let next = current_pos + direction_offset;
                points.push(next);
                current_pos = next;

                // Then process any then clauses after direction moves
                let mut current_dir = ctx.direction;
                for clause in &then_clauses {
                    let (next_point, next_dir) =
                        eval_then_clause(ctx, clause, current_pos, current_dir, width)?;
                    points.push(next_point);
                    current_pos = next_point;
                    current_dir = next_dir;
                }
            } else if !then_clauses.is_empty() {
                // No direction moves but has then clauses - start with default move
                let next = move_in_direction(current_pos, ctx.direction, width);
                points.push(next);
                current_pos = next;
                let mut current_dir = ctx.direction;

                for clause in &then_clauses {
                    let (next_point, next_dir) =
                        eval_then_clause(ctx, clause, current_pos, current_dir, width)?;
                    points.push(next_point);
                    current_pos = next_point;
                    current_dir = next_dir;
                }
            } else {
                // No direction moves, no then clauses - default single segment
                let next = move_in_direction(current_pos, ctx.direction, width);
                points.push(next);
            }

            let end = *points.last().unwrap_or(&start);
            let center = start.midpoint(end);
            (center, start, end, points)
        }
    } else if let Some((edge, target)) = with_clause {
        // Position object so that specified edge is at target position
        let center = calculate_center_from_edge(edge, target, width, height, class);
        let (_, s, e) = calculate_object_position_at(ctx.direction, center, width, height);
        (center, s, e, vec![s, e])
    } else if let Some(pos) = explicit_position {
        // Box-like objects with explicit "at" position
        let (_, s, e) = calculate_object_position_at(ctx.direction, pos, width, height);
        (pos, s, e, vec![s, e])
    } else {
        let (c, s, e) = calculate_object_position(ctx, class, width, height);
        (c, s, e, vec![s, e])
    };

    // Handle sublist: use pre-rendered children and translate to final position
    let mut children = sublist_children.unwrap_or_default();

    if !children.is_empty() {
        // Children were rendered in local coords starting at (0,0).
        // We need to translate them so their center aligns with the sublist's center.
        if let Some(ref bounds) = sublist_bounds {
            if !bounds.is_empty() {
                // The children's center in local coords
                let local_center = bounds.center();
                // Offset from local center to final center
                let offset = Point::new(center.x - local_center.x, center.y - local_center.y);
                translate_children(&mut children, offset);
            }
        }
    }

    // If no explicit name and there's text, use the first text value as implicit name
    // This matches C pikchr behavior where `circle "C2"` can be referenced as C2
    let final_name = name.or_else(|| text.first().map(|t| t.value.clone()));

    // Clear current_object now that we're done building this object
    ctx.current_object = None;

    Ok(RenderedObject {
        name: final_name,
        class,
        center,
        width,
        height,
        start,
        end,
        start_attachment: from_attachment,
        end_attachment: to_attachment,
        waypoints,
        text,
        style,
        children,
    })
}

/// Render a sublist of statements with local coordinates and return children (still local)
fn render_sublist(
    parent_ctx: &RenderContext,
    statements: &[Statement],
) -> Result<Vec<RenderedObject>, miette::Report> {
    // Local context: starts at (0,0) but inherits variables and direction
    let mut ctx = RenderContext::new();
    ctx.direction = parent_ctx.direction;
    ctx.variables = parent_ctx.variables.clone();

    for stmt in statements {
        match stmt {
            Statement::Object(obj_stmt) => {
                let obj = render_object_stmt(&mut ctx, obj_stmt, None)?;
                ctx.add_object(obj);
            }
            Statement::Labeled(labeled) => {
                if let LabeledContent::Object(obj_stmt) = &labeled.content {
                    let obj = render_object_stmt(&mut ctx, obj_stmt, Some(labeled.label.clone()))?;
                    ctx.add_object(obj);
                }
            }
            _ => {
                // Skip other statement types in sublists for now
            }
        }
    }

    Ok(ctx.object_list)
}

/// Translate children from local sublist space to a given offset (usually parent center)
fn translate_children(children: &mut [RenderedObject], offset: PointIn) {
    for child in children.iter_mut() {
        translate_object(child, offset);
    }
}

fn translate_object(obj: &mut RenderedObject, offset: PointIn) {
    obj.center.x += offset.x;
    obj.center.y += offset.y;
    obj.start.x += offset.x;
    obj.start.y += offset.y;
    obj.end.x += offset.x;
    obj.end.y += offset.y;

    for pt in obj.waypoints.iter_mut() {
        pt.x += offset.x;
        pt.y += offset.y;
    }

    for child in obj.children.iter_mut() {
        translate_object(child, offset);
    }
}

/// Calculate center position given that a specific edge should be at target
fn calculate_center_from_edge(
    edge: EdgePoint,
    target: PointIn,
    width: Inches,
    height: Inches,
    class: ObjectClass,
) -> PointIn {
    let hw = width / 2.0;
    let hh = height / 2.0;

    // For circles/ellipses, diagonal edge points use the actual point on the
    // perimeter at 45 degrees, not the bounding box corner.
    let is_round = matches!(
        class,
        ObjectClass::Circle | ObjectClass::Ellipse | ObjectClass::Oval
    );
    let diag = if is_round {
        std::f64::consts::FRAC_1_SQRT_2
    } else {
        1.0
    };

    match edge {
        EdgePoint::North | EdgePoint::N | EdgePoint::Top | EdgePoint::T => {
            Point::new(target.x, target.y + hh)
        }
        EdgePoint::South | EdgePoint::S | EdgePoint::Bottom => Point::new(target.x, target.y - hh),
        EdgePoint::East | EdgePoint::E | EdgePoint::Right => Point::new(target.x - hw, target.y),
        EdgePoint::West | EdgePoint::W | EdgePoint::Left => Point::new(target.x + hw, target.y),
        EdgePoint::NorthEast => Point::new(target.x - hw * diag, target.y + hh * diag),
        EdgePoint::NorthWest => Point::new(target.x + hw * diag, target.y + hh * diag),
        EdgePoint::SouthEast => Point::new(target.x - hw * diag, target.y - hh * diag),
        EdgePoint::SouthWest => Point::new(target.x + hw * diag, target.y - hh * diag),
        EdgePoint::Center | EdgePoint::C => target,
        EdgePoint::Start | EdgePoint::End => target, // For lines, just use target
    }
}

/// Move a point in a direction by a distance
/// Note: SVG Y increases downward, so Up subtracts and Down adds
fn move_in_direction(pos: PointIn, dir: Direction, distance: Inches) -> PointIn {
    pos + dir.offset(distance)
}

/// Evaluate a then clause and return the next point and direction
fn eval_then_clause(
    ctx: &RenderContext,
    clause: &ThenClause,
    current_pos: PointIn,
    current_dir: Direction,
    default_distance: Inches,
) -> Result<(PointIn, Direction), miette::Report> {
    match clause {
        ThenClause::To(pos) => {
            let target = eval_position(ctx, pos)?;
            Ok((target, current_dir))
        }
        ThenClause::DirectionMove(dir, dist) => {
            let distance = if let Some(relexpr) = dist {
                eval_len(ctx, &relexpr.expr)?
            } else {
                default_distance
            };
            let next = move_in_direction(current_pos, *dir, distance);
            Ok((next, *dir))
        }
        ThenClause::DirectionEven(dir, pos) => {
            // Move in direction until even with position
            let target = eval_position(ctx, pos)?;
            let next = match dir {
                Direction::Right | Direction::Left => Point::new(target.x, current_pos.y),
                Direction::Up | Direction::Down => Point::new(current_pos.x, target.y),
            };
            Ok((next, *dir))
        }
        ThenClause::DirectionUntilEven(dir, pos) => {
            // Same as DirectionEven
            let target = eval_position(ctx, pos)?;
            let next = match dir {
                Direction::Right | Direction::Left => Point::new(target.x, current_pos.y),
                Direction::Up | Direction::Down => Point::new(current_pos.x, target.y),
            };
            Ok((next, *dir))
        }
        ThenClause::Heading(dist, angle_expr) => {
            let distance = if let Some(relexpr) = dist {
                eval_len(ctx, &relexpr.expr)?
            } else {
                default_distance
            };
            let angle = eval_scalar(ctx, angle_expr)?;
            // Convert angle (degrees, 0 = north/up, clockwise) to radians
            let rad = (90.0 - angle).to_radians();
            let next = Point::new(
                current_pos.x + Inches(distance.0 * rad.cos()),
                current_pos.y - Inches(distance.0 * rad.sin()),
            );
            Ok((next, current_dir))
        }
        ThenClause::EdgePoint(dist, edge) => {
            let distance = if let Some(relexpr) = dist {
                eval_len(ctx, &relexpr.expr)?
            } else {
                default_distance
            };
            // Get direction from edge point and compute displacement
            let dir = edge_point_offset(edge);
            let displacement = dir * distance;
            let next = current_pos + displacement;
            Ok((next, current_dir))
        }
    }
}

/// Calculate start/end points for an object at a specific center position
fn calculate_object_position_at(
    direction: Direction,
    center: PointIn,
    width: Inches,
    height: Inches,
) -> (PointIn, PointIn, PointIn) {
    let half_dim = match direction {
        Direction::Right | Direction::Left => width / 2.0,
        Direction::Up | Direction::Down => height / 2.0,
    };
    // Start is opposite to travel direction (entry edge)
    let start = center + direction.opposite().offset(half_dim);
    // End is in travel direction (exit edge)
    let end = center + direction.offset(half_dim);
    (center, start, end)
}

fn calculate_object_position(
    ctx: &RenderContext,
    class: ObjectClass,
    width: Inches,
    height: Inches,
) -> (PointIn, PointIn, PointIn) {
    // For line-like objects, start is at cursor, end is cursor + length in direction
    let (start, end, center) = match class {
        ObjectClass::Line | ObjectClass::Arrow | ObjectClass::Spline | ObjectClass::Move => {
            let start = ctx.position;
            let end = start + ctx.direction.offset(width);
            let mid = start.midpoint(end);
            (start, end, mid)
        }
        _ => {
            // For shaped objects (box, circle, etc.):
            // The entry edge is placed at the current cursor, not the center.
            // This matches C pikchr behavior where objects chain edge-to-edge.
            let (half_w, half_h) = (width / 2.0, height / 2.0);
            // Center is half-dimension in the direction of travel from cursor
            let half_dim = match ctx.direction {
                Direction::Right | Direction::Left => half_w,
                Direction::Up | Direction::Down => half_h,
            };
            let center = ctx.position + ctx.direction.offset(half_dim);
            // Start is the entry edge (opposite to travel direction)
            let start = center + ctx.direction.opposite().offset(half_dim);
            // End is the exit edge (in travel direction)
            let end = center + ctx.direction.offset(half_dim);
            (start, end, center)
        }
    };

    (center, start, end)
}
