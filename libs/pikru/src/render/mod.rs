//! SVG rendering for pikchr diagrams
//!
//! This module is organized into submodules:
//! - `defaults`: Default sizes and settings
//! - `types`: Core types like Value, PositionedText, RenderedObject, ClassName, ObjectStyle
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
pub use shapes::Shape;
pub use types::*;

use crate::ast::*;
use crate::types::{EvalValue, Length as Inches, OffsetIn, Point};
use eval::{
    endpoint_object_from_position, eval_color, eval_expr, eval_len, eval_position, eval_rvalue,
    eval_scalar, resolve_object,
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
// cref: pik_text_length (pikchr.c:6368)
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
            // cref: pik_set_direction (pikchr.c:5746)
            // When direction changes, update the cursor to the previous object's
            // exit point in the NEW direction. This makes "arrow; circle; down; arrow"
            // work correctly - the second arrow starts from circle's south edge.
            ctx.direction = *dir;
            if let Some(last_obj) = ctx.object_list.last() {
                use crate::types::UnitVec;
                let unit_dir = match dir {
                    Direction::Right => UnitVec::EAST,
                    Direction::Left => UnitVec::WEST,
                    Direction::Up => UnitVec::NORTH,
                    Direction::Down => UnitVec::SOUTH,
                };
                ctx.position = last_obj.edge_point(unit_dir);
            }
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
// cref: pik_bbox_add_elist (pikchr.c:7206) - iterates objects
pub fn expand_object_bounds(bounds: &mut BoundingBox, obj: &RenderedObject) {
    // Delegate to the shape's expand_bounds method via enum_dispatch
    obj.shape.expand_bounds(bounds);
}

/// Vertical slot assignment for text
/// cref: pik_txt_vertical_layout (pikchr.c:4984)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextVSlot {
    Above2,
    Above,
    Center,
    Below,
    Below2,
}

/// Compute vertical slot assignments for text lines
/// cref: pik_txt_vertical_layout (pikchr.c:4984)
pub fn compute_text_vslots(texts: &[PositionedText]) -> Vec<TextVSlot> {
    let n = texts.len();
    if n == 0 {
        return vec![];
    }

    // First, check what slots are explicitly assigned
    let mut slots: Vec<Option<TextVSlot>> = texts
        .iter()
        .map(|t| {
            if t.above {
                Some(TextVSlot::Above)
            } else if t.below {
                Some(TextVSlot::Below)
            } else {
                None // unassigned
            }
        })
        .collect();

    if n == 1 {
        // Single text defaults to center
        if slots[0].is_none() {
            slots[0] = Some(TextVSlot::Center);
        }
        return slots.into_iter().map(|s| s.unwrap()).collect();
    }

    // Build list of free slots (from top to bottom)
    let all_slots_mask: u8 = slots
        .iter()
        .map(|s| match s {
            Some(TextVSlot::Above2) => 1,
            Some(TextVSlot::Above) => 2,
            Some(TextVSlot::Center) => 4,
            Some(TextVSlot::Below) => 8,
            Some(TextVSlot::Below2) => 16,
            None => 0,
        })
        .fold(0, |a, b| a | b);

    let mut free_slots = Vec::new();
    if n >= 4 && (all_slots_mask & 1) == 0 {
        free_slots.push(TextVSlot::Above2);
    }
    if (all_slots_mask & 2) == 0 {
        free_slots.push(TextVSlot::Above);
    }
    if (n & 1) != 0 {
        // odd number of texts: include center slot
        free_slots.push(TextVSlot::Center);
    }
    if (all_slots_mask & 8) == 0 {
        free_slots.push(TextVSlot::Below);
    }
    if n >= 4 && (all_slots_mask & 16) == 0 {
        free_slots.push(TextVSlot::Below2);
    }

    // Assign free slots to unassigned texts
    let mut free_iter = free_slots.into_iter();
    for slot in &mut slots {
        if slot.is_none() {
            *slot = free_iter.next();
        }
    }

    slots
        .into_iter()
        .map(|s| s.unwrap_or(TextVSlot::Center))
        .collect()
}

/// Count text labels above and below for lines
pub fn count_text_above_below(texts: &[PositionedText]) -> (usize, usize) {
    let slots = compute_text_vslots(texts);
    let above = slots
        .iter()
        .filter(|s| matches!(s, TextVSlot::Above | TextVSlot::Above2))
        .count();
    let below = slots
        .iter()
        .filter(|s| matches!(s, TextVSlot::Below | TextVSlot::Below2))
        .count();
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
    class_name: Option<ClassName>,
    width: Inches,
    height: Inches,
    style: &ObjectStyle,
) -> RenderedObject {
    use shapes::*;

    let center = pin(0.0, 0.0);

    // Create a simple shape for the partial object based on class
    let shape = match class_name {
        Some(ClassName::Circle) => ShapeEnum::Circle(CircleShape {
            center,
            radius: width / 2.0,
            style: style.clone(),
            text: Vec::new(),
        }),
        Some(ClassName::Line) | Some(ClassName::Arrow) => ShapeEnum::Line(LineShape {
            waypoints: vec![center, pin(width.0, 0.0)],
            style: style.clone(),
            text: Vec::new(),
        }),
        // For most other shapes, use a box
        _ => ShapeEnum::Box(BoxShape {
            center,
            width,
            height,
            corner_radius: style.corner_radius,
            style: style.clone(),
            text: Vec::new(),
        }),
    };

    RenderedObject {
        name: None,
        shape,
        start_attachment: None,
        end_attachment: None,
    }
}

/// Update the current_object in context with latest dimensions
fn update_current_object(
    ctx: &mut RenderContext,
    class_name: Option<ClassName>,
    width: Inches,
    height: Inches,
    style: &ObjectStyle,
) {
    ctx.current_object = Some(make_partial_object(class_name, width, height, style));
}

fn render_object_stmt(
    ctx: &mut RenderContext,
    obj_stmt: &ObjectStatement,
    name: Option<String>,
) -> Result<RenderedObject, miette::Report> {
    // Extract class name for shapes that have one
    let class_name: Option<ClassName> = match &obj_stmt.basetype {
        BaseType::Class(cn) => Some(*cn),
        BaseType::Text(_, _) => Some(ClassName::Text),
        BaseType::Sublist(_) => Some(ClassName::Sublist),
    };
    // Unwrap for use in fit/position logic - Sublist uses default Box-like behavior
    let class = class_name.unwrap_or(ClassName::Box);

    // Determine base object properties from context variables (like C pikchr's pik_value)
    let (mut width, mut height) = match &obj_stmt.basetype {
        BaseType::Class(cn) => match cn {
            ClassName::Box => (
                ctx.get_length("boxwid", 0.75),
                ctx.get_length("boxht", 0.5),
            ),
            ClassName::Circle => {
                let rad = ctx.get_length("circlerad", 0.25);
                (rad * 2.0, rad * 2.0)
            }
            ClassName::Ellipse => (
                ctx.get_length("ellipsewid", 0.75),
                ctx.get_length("ellipseht", 0.5),
            ),
            ClassName::Oval => (
                ctx.get_length("ovalwid", 1.0),
                ctx.get_length("ovalht", 0.5),
            ),
            ClassName::Cylinder => (
                ctx.get_length("cylwid", 0.75),
                ctx.get_length("cylht", 0.5),
            ),
            ClassName::Diamond => (
                ctx.get_length("diamondwid", 1.0),
                ctx.get_length("diamondht", 0.75),
            ),
            ClassName::File => (
                ctx.get_length("filewid", 0.5),
                ctx.get_length("fileht", 0.75),
            ),
            ClassName::Line => (
                ctx.get_length("linewid", 0.5),
                ctx.get_length("lineht", 0.5),
            ),
            ClassName::Arrow => (
                ctx.get_length("linewid", 0.5),
                ctx.get_length("lineht", 0.5),
            ),
            ClassName::Spline => (
                ctx.get_length("linewid", 0.5),
                ctx.get_length("lineht", 0.5),
            ),
            ClassName::Arc => {
                let arcrad = ctx.get_length("arcrad", 0.25);
                (arcrad, arcrad)
            }
            ClassName::Move => (
                ctx.get_length("movewid", 0.5),
                Inches::ZERO,
            ),
            ClassName::Dot => {
                let dotrad = ctx.get_length("dotrad", 0.025);
                (dotrad * 2.0, dotrad * 2.0)
            }
            ClassName::Text => {
                // Default dimensions - will be overridden by actual text content later
                // Use charht for height to match C pikchr's text sizing
                let charht = ctx.get_scalar("charht", 0.14);
                (
                    ctx.get_length("textwid", 0.75),
                    Inches(charht),
                )
            }
            // Sublist is handled via BaseType::Sublist, not BaseType::Class(Sublist)
            ClassName::Sublist => (Inches::ZERO, Inches::ZERO),
        },
        BaseType::Text(s, _) => {
            // Use proportional character widths like C pikchr
            let charwid = ctx.get_scalar("charwid", 0.08);
            let charht = ctx.get_scalar("charht", 0.14);
            let w = text_width_inches(&s.value, charwid);
            let h = charht;
            (Inches(w), Inches(h))
        }
        BaseType::Sublist(_) => {
            // Placeholder - will be computed from rendered children below
            (Inches::ZERO, Inches::ZERO)
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
    if class_name == Some(ClassName::Arrow) {
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
        // Apply fit sizing - SET size to fit text (can shrink below default)
        // This matches C pikchr's pik_size_to_fit() function
        if !fit_text.is_empty() {
            let charwid = ctx.get_scalar("charwid", defaults::CHARWID);
            let charht = ctx.get_scalar("charht", defaults::FONT_SIZE);

            // Calculate text bounding box like C pikchr's pik_append_txt
            // For centered text: bbox extends from -cw/2 to +cw/2 horizontally
            // and -ch to +ch vertically (where ch = charht * 0.5)
            let max_text_width = fit_text
                .iter()
                .map(|t| text_width_inches(&t.value, charwid))
                .fold(0.0_f64, |a, b| a.max(b));

            // C pikchr fit: w = (bbox.ne.x - bbox.sw.x) + charWidth
            // For centered text, bbox width = text_width, so w = text_width + charwid
            let fit_width = Inches(max_text_width + charwid);

            // C pikchr fit: h = 2.0 * max(h1, h2) + 0.5 * charHeight
            // For single centered line: h1 = h2 = charht * 0.5, so h = charht + 0.5*charht = 1.5*charht
            let center_lines = fit_text.iter().filter(|t| !t.above && !t.below).count();
            let half_height = charht * 0.5 * center_lines.max(1) as f64;
            let fit_height = Inches(2.0 * half_height + 0.5 * charht);

            // Apply shape-specific fit logic matching C pikchr's xFit callbacks
            match class_name {
                Some(ClassName::Circle) => {
                    // cref: circleFit (pikchr.c:3940) - uses hypot(w,h) when both are positive
                    let w = fit_width.raw();
                    let h = fit_height.raw();
                    let mut mx = w.max(h);
                    if w > 0.0 && h > 0.0 && (w * w + h * h) > mx * mx {
                        mx = w.hypot(h);
                    }
                    let diameter = Inches(mx);
                    width = diameter;
                    height = diameter;
                }
                Some(ClassName::Cylinder) => {
                    // cref: cylinderFit (pikchr.c:3976) - height = h + 0.25*rad + stroke_width
                    let rad = ctx.get_length("cylrad", 0.075);
                    width = fit_width;
                    height = fit_height + rad * 0.25 + style.stroke_width;
                }
                Some(ClassName::Diamond) => {
                    // cref: diamondFit (pikchr.c:4096) - 1.5x initial scale, then proportional expansion
                    let mut w = fit_width.raw() * 1.5;
                    let mut h = fit_height.raw() * 1.5;
                    if w > 0.0 && h > 0.0 {
                        let x = w * fit_height.raw() / h + fit_width.raw();
                        let y = h * x / w;
                        w = x;
                        h = y;
                    }
                    width = Inches(w);
                    height = Inches(h);
                }
                Some(ClassName::File) => {
                    // cref: fileFit (pikchr.c:4214) - height = h + 2*rad (corner fold padding)
                    let rad = ctx.get_length("filerad", 0.15);
                    width = fit_width;
                    height = fit_height + rad * 2.0;
                }
                Some(ClassName::Oval) => {
                    // cref: ovalFit (pikchr.c:4320) - enforce width >= height
                    width = fit_width.max(fit_height);
                    height = fit_height;
                }
                _ => {
                    // cref: boxFit (pikchr.c:3845) - Box, Ellipse, Text: direct assignment
                    width = fit_width;
                    height = fit_height;
                }
            }
            style.fit = true;
        }
    }

    // Initialize current_object for `this` keyword support
    update_current_object(ctx, class_name, width, height, &style);

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
                            match class_name {
                                Some(ClassName::Circle) | Some(ClassName::Ellipse) | Some(ClassName::Arc) => {
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
                        update_current_object(ctx, class_name, width, height, &style);
                    }
                    NumProperty::Height => {
                        height = val;
                        update_current_object(ctx, class_name, width, height, &style);
                    }
                    NumProperty::Radius => {
                        // For circles/ellipses, radius sets size (diameter = 2 * radius)
                        // For boxes, radius sets corner rounding
                        match class_name {
                            Some(ClassName::Circle) | Some(ClassName::Ellipse) | Some(ClassName::Arc) => {
                                width = val * 2.0;
                                height = val * 2.0;
                                update_current_object(ctx, class_name, width, height, &style);
                            }
                            _ => {
                                style.corner_radius = val;
                            }
                        }
                    }
                    NumProperty::Diameter => {
                        width = val;
                        height = val;
                        update_current_object(ctx, class_name, width, height, &style);
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
                BoolProperty::Clockwise => style.clockwise = true,
                BoolProperty::CounterClockwise => style.clockwise = false,
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
                            width = source.width();
                            height = source.height();
                            style = source.style().clone();
                        }
                    }
                    None => {
                        // "same" without object - use last object of same class
                        if let Some(source) = ctx.get_last_object(Some(class)) {
                            width = source.width();
                            height = source.height();
                            style = source.style().clone();
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

    // Apply fit: auto-size shape to fit text content
    // cref: pik_size_to_fit (pikchr.c:6438)
    // cref: textOffset (pikchr.c:4416) - text objects always get auto-fitted
    let should_fit = style.fit || class == ClassName::Text;
    if should_fit && !text.is_empty() {
        let charwid = ctx.get_scalar("charwid", defaults::CHARWID);
        let charht = ctx.get_scalar("charht", defaults::FONT_SIZE);

        // Calculate text bounding box like C pikchr's pik_append_txt
        let max_text_width = text
            .iter()
            .map(|t| text_width_inches(&t.value, charwid))
            .fold(0.0_f64, |a, b| a.max(b));

        // C pikchr fit: w = (bbox.ne.x - bbox.sw.x) + charWidth
        let fit_width = Inches(max_text_width + charwid);

        // C pikchr fit: h = 2.0 * max(h1, h2) + 0.5 * charHeight
        // cref: pik_size_to_fit (pikchr.c:6461-6466) - computes h1, h2 from actual text bbox
        // cref: pik_append_txt (pikchr.c:5104-5143) - computes region heights
        //
        // We need to calculate the actual text bbox considering above/below positioning.
        // C assigns text to slots (CENTER, ABOVE, ABOVE2, BELOW, BELOW2) and computes
        // the max height in each slot, then positions text accordingly.
        let vslots = compute_text_vslots(&text);

        // Compute heights for each region (matching C's logic in pik_append_txt)
        let mut hc = 0.0_f64;  // center height
        let mut ha1 = 0.0_f64; // above height
        let mut ha2 = 0.0_f64; // above2 height
        let mut hb1 = 0.0_f64; // below height
        let mut hb2 = 0.0_f64; // below2 height

        for slot in &vslots {
            match slot {
                TextVSlot::Center => hc = hc.max(charht),
                TextVSlot::Above => ha1 = ha1.max(charht),
                TextVSlot::Above2 => ha2 = ha2.max(charht),
                TextVSlot::Below => hb1 = hb1.max(charht),
                TextVSlot::Below2 => hb2 = hb2.max(charht),
            }
        }

        // Calculate h1 (max extent above center) and h2 (max extent below center)
        // cref: pik_append_txt lines 5155-5158 for Y positioning
        // Y positions relative to center (ptAt):
        //   ABOVE2: 0.5*hc + ha1 + 0.5*ha2 + 0.5*charht (top of text)
        //   ABOVE:  0.5*hc + 0.5*ha1 + 0.5*charht
        //   CENTER: 0.5*charht (half height above center)
        //   BELOW:  -(0.5*hc + 0.5*hb1) - 0.5*charht (bottom of text)
        //   BELOW2: -(0.5*hc + hb1 + 0.5*hb2) - 0.5*charht
        let h1 = if ha2 > 0.0 {
            0.5 * hc + ha1 + 0.5 * ha2 + 0.5 * charht
        } else if ha1 > 0.0 {
            0.5 * hc + 0.5 * ha1 + 0.5 * charht
        } else {
            0.5 * hc.max(charht)
        };

        let h2 = if hb2 > 0.0 {
            0.5 * hc + hb1 + 0.5 * hb2 + 0.5 * charht
        } else if hb1 > 0.0 {
            0.5 * hc + 0.5 * hb1 + 0.5 * charht
        } else {
            0.5 * hc.max(charht)
        };

        let fit_height = Inches(2.0 * h1.max(h2) + 0.5 * charht);

        // Apply shape-specific fit logic matching C pikchr's xFit callbacks
        match class {
            ClassName::Circle => {
                // cref: circleFit (pikchr.c:3940)
                let w = fit_width.raw();
                let h = fit_height.raw();
                let mut mx = w.max(h);
                tracing::debug!(w, h, mx, "circleFit initial");
                if w > 0.0 && h > 0.0 && (w * w + h * h) > mx * mx {
                    mx = w.hypot(h);
                    tracing::debug!(mx, "circleFit using hypot");
                }
                let diameter = Inches(mx);
                let radius = diameter / 2.0;
                tracing::debug!(rad_inches = radius.raw(), rad_px = radius.raw() * 144.0, "circleFit final");
                width = diameter;
                height = diameter;
            }
            ClassName::Cylinder => {
                // cref: cylinderFit (pikchr.c:3976)
                let rad = ctx.get_length("cylrad", 0.075);
                width = fit_width;
                height = fit_height + rad * 0.25 + style.stroke_width;
            }
            ClassName::Diamond => {
                // cref: diamondFit (pikchr.c:4096)
                let mut w = fit_width.raw() * 1.5;
                let mut h = fit_height.raw() * 1.5;
                if w > 0.0 && h > 0.0 {
                    let x = w * fit_height.raw() / h + fit_width.raw();
                    let y = h * x / w;
                    w = x;
                    h = y;
                }
                width = Inches(w);
                height = Inches(h);
            }
            ClassName::File => {
                // cref: fileFit (pikchr.c:4214)
                let rad = ctx.get_length("filerad", 0.15);
                width = fit_width;
                height = fit_height + rad * 2.0;
            }
            ClassName::Oval => {
                // cref: ovalFit (pikchr.c:4320)
                width = fit_width.max(fit_height);
                height = fit_height;
            }
            _ => {
                // cref: boxFit (pikchr.c:3845) - Box, Ellipse, Text: direct assignment
                width = fit_width;
                height = fit_height;
            }
        }
        if class == ClassName::Text {
            tracing::debug!(
                fit_width = fit_width.raw(),
                fit_height = fit_height.raw(),
                width = width.raw(),
                height = height.raw(),
                "textFit"
            );
        }
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
                match last_obj.class() {
                    ClassName::Line
                    | ClassName::Arrow
                    | ClassName::Spline
                    | ClassName::Arc
                    | ClassName::Move => last_obj.end(),
                    _ => {
                        // For box-like objects, calculate exit edge based on direction
                        let (hw, hh) = (last_obj.width() / 2.0, last_obj.height() / 2.0);
                        let c = last_obj.center();
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
                let offset = center - local_center;
                for child in children.iter_mut() {
                    child.translate(offset);
                }
            }
        }
    }

    // If no explicit name and there's text, use the first text value as implicit name
    // This matches C pikchr behavior where `circle "C2"` can be referenced as C2
    // BUT: don't overwrite an existing named object - C pikchr doesn't allow that
    let final_name = name.or_else(|| {
        text.first()
            .map(|t| t.value.clone())
            .filter(|n| ctx.get_object(n).is_none())
    });

    // Clear current_object now that we're done building this object
    ctx.current_object = None;

    // Create the appropriate shape based on class
    use shapes::*;
    let shape = match class {
        ClassName::Box => ShapeEnum::Box(BoxShape {
            center,
            width,
            height,
            corner_radius: style.corner_radius,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Circle => ShapeEnum::Circle(CircleShape {
            center,
            radius: width / 2.0,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Ellipse => ShapeEnum::Ellipse(EllipseShape {
            center,
            width,
            height,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Oval => ShapeEnum::Oval(OvalShape {
            center,
            width,
            height,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Diamond => ShapeEnum::Diamond(DiamondShape {
            center,
            width,
            height,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Cylinder => ShapeEnum::Cylinder(CylinderShape {
            center,
            width,
            height,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::File => ShapeEnum::File(FileShape {
            center,
            width,
            height,
            fold_radius: defaults::FILE_RAD,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Line | ClassName::Arrow => {
            let mut line_style = style.clone();
            if class == ClassName::Arrow {
                line_style.arrow_end = true;
            }
            ShapeEnum::Line(LineShape {
                waypoints: waypoints.clone(),
                style: line_style,
                text: text.clone(),
            })
        }
        ClassName::Spline => ShapeEnum::Spline(SplineShape {
            waypoints: waypoints.clone(),
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Arc => ShapeEnum::Arc(ArcShape {
            start,
            end,
            style: style.clone(),
            text: text.clone(),
            clockwise: style.clockwise,
        }),
        ClassName::Move => ShapeEnum::Move(MoveShape {
            start,
            end,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Dot => ShapeEnum::Dot(DotShape {
            center,
            radius: width / 2.0,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Text => ShapeEnum::Text(TextShape {
            center,
            width,
            height,
            style: style.clone(),
            text: text.clone(),
        }),
        ClassName::Sublist => ShapeEnum::Sublist(SublistShape {
            center,
            width,
            height,
            style: style.clone(),
            text: text.clone(),
            children,
        }),
    };

    Ok(RenderedObject {
        name: final_name,
        shape,
        start_attachment: from_attachment,
        end_attachment: to_attachment,
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

/// Calculate center position given that a specific edge should be at target
fn calculate_center_from_edge(
    edge: EdgePoint,
    target: PointIn,
    width: Inches,
    height: Inches,
    class: ClassName,
) -> PointIn {
    match edge {
        EdgePoint::Center | EdgePoint::C | EdgePoint::Start | EdgePoint::End => return target,
        _ => {}
    }

    let hw = width / 2.0;
    let hh = height / 2.0;
    let diag = class.diagonal_factor();

    // Use UnitVec for direction, then negate to go from edge back to center
    let offset = edge.to_unit_vec().scale_xy(hw * diag, hh * diag);

    // Edge point = center + offset, so center = edge point - offset
    target - offset
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
            let dir = edge.to_unit_vec();
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
    class: ClassName,
    width: Inches,
    height: Inches,
) -> (PointIn, PointIn, PointIn) {
    // cref: pik_elem_new (pikchr.c:5615) - initial positioning logic
    // First object: centered at (0,0) with eWith=CP_C
    // Subsequent objects: entry edge at previous exit, with eWith=CP_W/E/N/S

    let is_first_object = ctx.object_list.is_empty();

    // For line-like objects, start is at cursor, end is cursor + length in direction
    let (start, end, center) = match class {
        ClassName::Line | ClassName::Arrow | ClassName::Spline | ClassName::Move => {
            let start = ctx.position;
            let end = start + ctx.direction.offset(width);
            let mid = start.midpoint(end);
            (start, end, mid)
        }
        _ => {
            let (half_w, half_h) = (width / 2.0, height / 2.0);

            if is_first_object {
                // First object: center at cursor (which is 0,0)
                // cref: pik_elem_new line 5632: pNew->eWith = CP_C
                let center = ctx.position;
                let half_dim = match ctx.direction {
                    Direction::Right | Direction::Left => half_w,
                    Direction::Up | Direction::Down => half_h,
                };
                let start = center + ctx.direction.opposite().offset(half_dim);
                let end = center + ctx.direction.offset(half_dim);
                (start, end, center)
            } else {
                // Subsequent objects: entry edge at cursor
                // cref: pik_elem_new line 5637: pNew->eWith = CP_W/E/N/S
                let half_dim = match ctx.direction {
                    Direction::Right | Direction::Left => half_w,
                    Direction::Up | Direction::Down => half_h,
                };
                let center = ctx.position + ctx.direction.offset(half_dim);
                let start = center + ctx.direction.opposite().offset(half_dim);
                let end = center + ctx.direction.offset(half_dim);
                (start, end, center)
            }
        }
    };

    tracing::debug!(
        ?class,
        is_first_object,
        cursor_x = ctx.position.x.0,
        cursor_y = ctx.position.y.0,
        dir = ?ctx.direction,
        w = width.0,
        h = height.0,
        center_x = center.x.0,
        center_y = center.y.0,
        start_x = start.x.0,
        start_y = start.y.0,
        end_x = end.x.0,
        end_y = end.y.0,
        "calculate_object_position"
    );

    (center, start, end)
}
