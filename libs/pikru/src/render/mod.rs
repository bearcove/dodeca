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

/// Character width units for proportional text (in hundredths).
/// Monospace uses constant 82 units per character.
// cref: pik_text_length (pikchr.c:6386)
fn proportional_text_length(text: &str) -> u32 {
    const STD_AVG: u32 = 100;
    let mut cnt: u32 = 0;
    for c in text.chars() {
        if c >= ' ' && c <= '~' {
            cnt += AW_CHAR[(c as usize) - 0x20] as u32;
        } else {
            cnt += STD_AVG;
        }
    }
    cnt
}

fn monospace_text_length(text: &str) -> u32 {
    const MONO_AVG: u32 = 82;
    text.chars().count() as u32 * MONO_AVG
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

    tracing::debug!(
        bounds_min_x = ctx.bounds.min.x.raw(),
        bounds_min_y = ctx.bounds.min.y.raw(),
        bounds_max_x = ctx.bounds.max.x.raw(),
        bounds_max_y = ctx.bounds.max.y.raw(),
        "Rust: final bounding box before SVG generation"
    );

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
                // For line-like objects, the cursor should stay at the line's endpoint,
                // not be recalculated from center. Lines don't have meaningful "edges"
                // in the same way shaped objects do.
                let is_line_like = matches!(
                    last_obj.class(),
                    ClassName::Line | ClassName::Arrow | ClassName::Spline | ClassName::Move
                );
                if is_line_like {
                    // Keep cursor at line endpoint - don't recalculate
                    ctx.position = last_obj.end();
                } else {
                    // For shaped objects, get edge point in new direction
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
        }
        Statement::Assignment(assign) => {
            // cref: pik_set_var (pikchr.c:6479-6511)
            // eval_rvalue now returns EvalValue directly, preserving Color type information
            let rhs_val = eval_rvalue(ctx, &assign.rvalue)?;

            // Get variable name for lookup
            let var_name = match &assign.lvalue {
                LValue::Variable(name) => name.clone(),
                LValue::Fill => "fill".to_string(),
                LValue::Color => "color".to_string(),
                LValue::Thickness => "thickness".to_string(),
            };

            // Apply compound assignment operators
            let eval_val = match assign.op {
                AssignOp::Assign => rhs_val,
                AssignOp::AddAssign | AssignOp::SubAssign | AssignOp::MulAssign | AssignOp::DivAssign => {
                    // Get current value (with default of 0)
                    let current = ctx.variables.get(&var_name).cloned().unwrap_or(EvalValue::Scalar(0.0));

                    // Apply operation based on types
                    match (current, rhs_val) {
                        (EvalValue::Length(lhs), EvalValue::Scalar(rhs)) => {
                            // Length op Scalar
                            let result = match assign.op {
                                AssignOp::AddAssign => lhs + Inches(rhs),
                                AssignOp::SubAssign => lhs - Inches(rhs),
                                AssignOp::MulAssign => lhs * rhs,
                                AssignOp::DivAssign => lhs / rhs,
                                _ => unreachable!(),
                            };
                            EvalValue::Length(result)
                        }
                        (EvalValue::Scalar(lhs), EvalValue::Scalar(rhs)) => {
                            // Scalar op Scalar
                            let result = match assign.op {
                                AssignOp::AddAssign => lhs + rhs,
                                AssignOp::SubAssign => lhs - rhs,
                                AssignOp::MulAssign => lhs * rhs,
                                AssignOp::DivAssign => if rhs == 0.0 { lhs } else { lhs / rhs },
                                _ => unreachable!(),
                            };
                            EvalValue::Scalar(result)
                        }
                        (EvalValue::Length(lhs), EvalValue::Length(rhs)) => {
                            // For *= and /=, treat RHS length as scalar (bare number context)
                            // cref: pik_set_var (pikchr.c:6496-6499) - *= and /= use raw values
                            let result = match assign.op {
                                AssignOp::AddAssign => lhs + rhs,
                                AssignOp::SubAssign => lhs - rhs,
                                AssignOp::MulAssign => lhs * rhs.raw(), // Treat as scalar for multiplication
                                AssignOp::DivAssign => lhs / rhs.raw(), // Treat as scalar for division
                                _ => lhs,
                            };
                            EvalValue::Length(result)
                        }
                        _ => rhs_val, // Fallback for other combinations
                    }
                }
            };

            match &assign.lvalue {
                LValue::Variable(name) => {
                    tracing::debug!(op = ?assign.op, "Setting variable {} to {:?}", name, eval_val);
                    ctx.variables.insert(name.clone(), eval_val);
                }
                LValue::Fill => {
                    tracing::debug!(op = ?assign.op, "Setting global fill to {:?}", eval_val);
                    ctx.variables.insert("fill".to_string(), eval_val);
                }
                LValue::Color => {
                    tracing::debug!(op = ?assign.op, "Setting global color to {:?}", eval_val);
                    ctx.variables.insert("color".to_string(), eval_val);
                }
                LValue::Thickness => {
                    tracing::debug!(op = ?assign.op, "Setting global thickness to {:?}", eval_val);
                    ctx.variables.insert("thickness".to_string(), eval_val);
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
                            Value::Color(c) => format!("#{:06x}", c),
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
// cref: pik_bbox_add_elist (pikchr.c:7243) - checks pObj->sw>=0.0 before adding bbox
// cref: pik_bbox_add_elist (pikchr.c:7251-7260) - arrowheads added regardless of sw
pub fn expand_object_bounds(bounds: &mut BoundingBox, obj: &RenderedObject) {
    // C pikchr: shape bbox only added if sw>=0, but arrowheads are always added
    // The shape's expand_bounds handles both aspects
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

    // cref: pik_txt_vertical_layout (pikchr.c:2321-2332)
    // If there is more than one TP_ABOVE, change the first to TP_ABOVE2.
    // Scan from end: first ABOVE found stays as ABOVE, second becomes ABOVE2.
    let mut found_above = false;
    for i in (0..n).rev() {
        if slots[i] == Some(TextVSlot::Above) {
            if !found_above {
                found_above = true;
            } else {
                slots[i] = Some(TextVSlot::Above2);
                break;
            }
        }
    }

    // cref: pik_txt_vertical_layout (pikchr.c:2335-2346)
    // Same logic for BELOW -> BELOW2
    let mut found_below = false;
    for i in 0..n {
        if slots[i] == Some(TextVSlot::Below) {
            if !found_below {
                found_below = true;
            } else {
                slots[i] = Some(TextVSlot::Below2);
                break;
            }
        }
    }

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

/// Sum actual text heights above and below for bbox calculation.
/// This computes the vertical extent of text relative to the object center,
/// accounting for the actual Y positions of each text slot.
///
/// cref: pik_append_txt (pikchr.c:2484-2528) - computes text bbox using y positions
pub fn sum_text_heights_above_below(texts: &[PositionedText], charht: f64) -> (f64, f64) {
    let slots = compute_text_vslots(texts);

    // First, compute the slot heights like C does (ha2, ha1, hc, hb1, hb2)
    // These are the MAX heights for each slot category
    let mut ha2 = 0.0_f64; // Above2 slot height
    let mut ha1 = 0.0_f64; // Above slot height
    let mut hc = 0.0_f64; // Center slot height
    let mut hb1 = 0.0_f64; // Below slot height
    let mut hb2 = 0.0_f64; // Below2 slot height

    for (text, slot) in texts.iter().zip(slots.iter()) {
        let h = text.height(charht);
        match slot {
            TextVSlot::Above2 => ha2 = ha2.max(h),
            TextVSlot::Above => ha1 = ha1.max(h),
            TextVSlot::Center => hc = hc.max(h),
            TextVSlot::Below => hb1 = hb1.max(h),
            TextVSlot::Below2 => hb2 = hb2.max(h),
        }
    }

    // Now compute the actual vertical extent from center (y=0)
    // Following C logic in pik_append_txt lines 2477-2480:
    //   ABOVE2: y = 0.5*hc + ha1 + 0.5*ha2  (top edge = y + ch = y + 0.5*h)
    //   ABOVE:  y = 0.5*hc + 0.5*ha1
    //   CENTER: y = 0  (extends +/- 0.5*hc from center)
    //   BELOW:  y = -(0.5*hc + 0.5*hb1)
    //   BELOW2: y = -(0.5*hc + hb1 + 0.5*hb2)
    //
    // The top-most point is ABOVE2's top edge: 0.5*hc + ha1 + ha2
    // The bottom-most point is BELOW2's bottom edge: -(0.5*hc + hb1 + hb2)

    // Total height above center line (positive Y direction)
    let above_extent = if ha2 > 0.0 {
        // ABOVE2 top edge
        0.5 * hc + ha1 + ha2
    } else if ha1 > 0.0 {
        // ABOVE top edge
        0.5 * hc + ha1
    } else {
        // Just center text
        0.5 * hc
    };

    // Total height below center line (negative Y direction, but return positive)
    let below_extent = if hb2 > 0.0 {
        // BELOW2 bottom edge
        0.5 * hc + hb1 + hb2
    } else if hb1 > 0.0 {
        // BELOW bottom edge
        0.5 * hc + hb1
    } else {
        // Just center text
        0.5 * hc
    };

    (above_extent, below_extent)
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
        layer: 1000, // Default layer for partial objects
        direction: Direction::Right, // Default direction for partial objects
        class_name: class_name.unwrap_or(ClassName::Box),
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

    // Get layer from "layer" variable, default 1000
    // cref: pik_elem_new (pikchr.c:2960)
    let mut layer = ctx.get_scalar("layer", 1000.0) as i32;

    // Determine base object properties from context variables (like C pikchr's pik_value)
    let (mut width, mut height) = match &obj_stmt.basetype {
        BaseType::Class(cn) => match cn {
            ClassName::Box => (ctx.get_length("boxwid", 0.75), ctx.get_length("boxht", 0.5)),
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
            ClassName::Cylinder => (ctx.get_length("cylwid", 0.75), ctx.get_length("cylht", 0.5)),
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
            ClassName::Move => (ctx.get_length("movewid", 0.5), Inches::ZERO),
            ClassName::Dot => {
                // cref: dotInit (pikchr.c:4026-4028)
                let dotrad = ctx.get_length("dotrad", 0.015);
                // C: pObj->w = pObj->h = pObj->rad * 6
                (dotrad * 6.0, dotrad * 6.0)
            }
            ClassName::Text => {
                // Default dimensions - will be overridden by actual text content later
                // Use charht for height to match C pikchr's text sizing
                let charht = ctx.get_scalar("charht", 0.14);
                (ctx.get_length("textwid", 0.75), Inches(charht))
            }
            // Sublist is handled via BaseType::Sublist, not BaseType::Class(Sublist)
            ClassName::Sublist => (Inches::ZERO, Inches::ZERO),
        },
        BaseType::Text(s, pos) => {
            // Use proportional character widths like C pikchr
            let charwid = ctx.get_scalar("charwid", 0.08);
            let charht = ctx.get_scalar("charht", 0.14);
            let pt = PositionedText::from_textposition(s.value.clone(), pos.as_ref());
            let w = pt.width_inches(charwid);
            let h = pt.height(charht);
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

    // Apply global fill and color settings (these can be overridden by attributes)
    // cref: pik_color_lookup, pik_render_object (pikchr.c)
    if let Some(fill_val) = ctx.variables.get("fill") {
        // fill is a color value
        style.fill = match fill_val {
            EvalValue::Color(c) => {
                let color_hex = format!("#{:06x}", c);
                tracing::debug!("Applying global fill color: {} (from {:?})", color_hex, fill_val);
                color_hex
            }
            _ => {
                tracing::debug!("Global fill is not a color: {:?}", fill_val);
                "none".to_string()
            }
        };
    } else {
        tracing::debug!("No global fill variable found");
    }
    if let Some(color_val) = ctx.variables.get("color") {
        // color/stroke is a color value
        style.stroke = match color_val {
            EvalValue::Color(c) => format!("#{:06x}", c),
            _ => "black".to_string(),
        };
    }

    // Initialize shape-specific radius values before processing attributes
    // cref: cylinderInit (pikchr.c:3974), boxInit (pikchr.c:3775), etc.
    if let Some(ref cn) = class_name {
        style.corner_radius = match cn {
            ClassName::Cylinder => ctx.get_length("cylrad", 0.075),
            ClassName::Box => ctx.get_length("boxrad", 0.0),
            ClassName::File => ctx.get_length("filerad", 0.0),
            ClassName::Line | ClassName::Arrow | ClassName::Spline => ctx.get_length("linerad", 0.0),
            _ => Inches::ZERO,
        };
    }

    let mut text = Vec::new();
    let mut explicit_position: Option<PointIn> = None;
    let mut from_position: Option<PointIn> = None;
    let mut to_positions: Vec<PointIn> = Vec::new();
    let mut from_attachment: Option<EndpointObject> = None;
    let mut to_attachment: Option<EndpointObject> = None;
    // Accumulated direction offsets for compound moves like "up 1 right 2"
    // cref: p->mTPath in C pikchr tracks horizontal/vertical movement
    let mut direction_offset = OffsetIn::ZERO;
    let mut has_direction_move: bool = false;
    let mut even_clause: Option<(Direction, Position)> = None;
    // Instead of storing ThenClauses directly, we store segments
    // Each "then" starts a new segment with accumulated direction offsets
    // cref: p->thenFlag in C pikchr - when set, next direction creates new point
    enum Segment {
        /// Relative offset from previous position (accumulated directions)
        Offset(OffsetIn, Direction),
        /// Absolute position (from "then to position")
        AbsolutePosition(PointIn),
    }
    let mut segments: Vec<Segment> = Vec::new();
    let mut current_segment_offset = OffsetIn::ZERO;
    let mut current_segment_direction: Option<Direction> = None;
    let mut in_then_segment = false;
    let mut with_clause: Option<(EdgePoint, PointIn)> = None; // (edge, target_position)
    // The object's direction - starts as ctx.direction, updated by DirectionMove attributes
    // cref: pObj->outDir in C pikchr
    let mut object_direction = ctx.direction;

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

    // For dots, initialize fill to match stroke color
    // cref: dotInit (pikchr.c:4026-4030) - pObj->fill = pObj->color
    // Dots render with both fill and stroke in the same color (stroke_width is NOT 0)
    if class_name == Some(ClassName::Dot) {
        style.fill = style.stroke.clone();
        tracing::debug!(
            fill = %style.fill,
            "[Rust dot init] Set fill = stroke"
        );
    }

    // Initialize current_object for `this` keyword support
    update_current_object(ctx, class_name, width, height, &style);

    // Process attributes in order, just like C does
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
                                Some(ClassName::Circle)
                                | Some(ClassName::Ellipse)
                                | Some(ClassName::Arc) => {
                                    width / 2.0 // current radius
                                }
                                Some(ClassName::Cylinder) => {
                                    // For cylinders, corner_radius was initialized to cylrad before attributes
                                    style.corner_radius
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
                            Some(ClassName::Circle)
                            | Some(ClassName::Ellipse)
                            | Some(ClassName::Arc) => {
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
            Attribute::DashProperty(prop, opt_expr) => {
                // Get the dash/dot width: use explicit value or fall back to dashwid default
                // cref: pik_set_dashed (pikchr.c:3205)
                let width = if let Some(expr) = opt_expr {
                    eval_len(ctx, expr)?
                } else {
                    Inches(eval::get_length(ctx, "dashwid", 0.05))
                };

                match prop {
                    DashProperty::Dashed => {
                        style.dashed = Some(width);
                        style.dotted = None; // Clear dotted if setting dashed
                    }
                    DashProperty::Dotted => {
                        style.dotted = Some(width);
                        style.dashed = None; // Clear dashed if setting dotted
                    }
                }
            }
            Attribute::ColorProperty(prop, rvalue) => {
                let color = eval_color(ctx, rvalue);
                // cref: dotNumProp (pikchr.c:1353-1363) - dots keep fill and stroke synchronized
                match prop {
                    ColorProperty::Fill => {
                        style.fill = color.clone();
                        // For dots, when fill is set, also update stroke to match
                        if class_name == Some(ClassName::Dot) {
                            style.stroke = color.clone();
                            tracing::debug!(
                                color = %color,
                                "[Rust dot] Fill set, updating stroke to match"
                            );
                        }
                    }
                    ColorProperty::Color => {
                        style.stroke = color.clone();
                        // For dots, when color (stroke) is set, also update fill to match
                        if class_name == Some(ClassName::Dot) {
                            style.fill = color.clone();
                            tracing::debug!(
                                color = %color,
                                "[Rust dot] Color set, updating fill to match"
                            );
                        }
                    }
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
                // cref: pikchr.y:694-697 - thick/thin multiply, solid resets stroke width
                BoolProperty::Thick => style.stroke_width = style.stroke_width * 1.5,
                BoolProperty::Thin => style.stroke_width = style.stroke_width * 0.67,
                BoolProperty::Solid => {
                    style.stroke_width = ctx
                        .variables
                        .get("thickness")
                        .and_then(|v| match v {
                            EvalValue::Length(l) => Some(*l),
                            _ => None,
                        })
                        .unwrap_or(defaults::STROKE_WIDTH);
                }
                BoolProperty::Clockwise => style.clockwise = true,
                BoolProperty::CounterClockwise => style.clockwise = false,
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
                    tracing::debug!(
                        x = p.x.0,
                        y = p.y.0,
                        "Attribute::To evaluated position"
                    );
                    to_positions.push(p);
                    if to_attachment.is_none() {
                        to_attachment = endpoint_object_from_position(ctx, pos);
                    }
                }
            }
            Attribute::DirectionMove(_go, dir, dist) => {
                has_direction_move = true;
                // Update object's direction - this will become the new global direction
                // cref: pik_after_adding_element sets p->eDir = pObj->outDir
                object_direction = *dir;
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
                // cref: pik_add_direction (pikchr.c:3272) - accumulates directions
                // If we're in a then segment, accumulate to current segment
                // Otherwise accumulate to the initial direction_offset
                if in_then_segment {
                    current_segment_offset += dir.offset(distance);
                    current_segment_direction = Some(*dir);
                } else {
                    direction_offset += dir.offset(distance);
                }
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
                    // Apply in context direction or current segment
                    if in_then_segment {
                        current_segment_offset += ctx.direction.offset(val);
                    } else {
                        direction_offset += ctx.direction.offset(val);
                    }
                }
            }
            Attribute::Then(Some(clause)) => {
                // cref: pik_then (pikchr.c:3240) - "then" starts a new segment
                // First, save any pending then segment
                if in_then_segment && current_segment_direction.is_some() {
                    segments.push(Segment::Offset(
                        current_segment_offset,
                        current_segment_direction.unwrap(),
                    ));
                    current_segment_offset = OffsetIn::ZERO;
                    current_segment_direction = None;
                }
                in_then_segment = true;

                // Process the then clause's direction if it has one
                match clause {
                    ThenClause::DirectionMove(dir, dist) => {
                        let distance = if let Some(relexpr) = dist {
                            if let Ok(d) = eval_len(ctx, &relexpr.expr) {
                                if relexpr.is_percent {
                                    width * (d.raw() / 100.0)
                                } else {
                                    d
                                }
                            } else {
                                width
                            }
                        } else {
                            width
                        };
                        current_segment_offset += dir.offset(distance);
                        current_segment_direction = Some(*dir);
                        object_direction = *dir;
                    }
                    ThenClause::EdgePoint(dist, edge) => {
                        // EdgePoint like "nw" specifies a diagonal direction
                        let distance = if let Some(relexpr) = dist {
                            if let Ok(d) = eval_len(ctx, &relexpr.expr) {
                                if relexpr.is_percent {
                                    width * (d.raw() / 100.0)
                                } else {
                                    d
                                }
                            } else {
                                width
                            }
                        } else {
                            width
                        };
                        let unit_vec = edge.to_unit_vec();
                        current_segment_offset += unit_vec * distance;
                        // Direction is determined by the edge point
                        current_segment_direction =
                            Some(Direction::from_edge_point(edge).unwrap_or(ctx.direction));
                    }
                    ThenClause::To(pos) => {
                        // "then to position" - save current segment if any, then add absolute position
                        if current_segment_direction.is_some() {
                            segments.push(Segment::Offset(
                                current_segment_offset,
                                current_segment_direction.unwrap(),
                            ));
                            current_segment_offset = OffsetIn::ZERO;
                            current_segment_direction = None;
                        }
                        if let Ok(p) = eval_position(ctx, pos) {
                            segments.push(Segment::AbsolutePosition(p));
                        }
                        in_then_segment = false;
                    }
                    _ => {
                        // Other clause types - handle as before by storing position targets
                    }
                }
            }
            Attribute::Then(None) => {
                // Bare "then" - just sets then flag for next direction
                // cref: pik_then (pikchr.c:3251) - p->thenFlag = 1
                if in_then_segment && current_segment_direction.is_some() {
                    segments.push(Segment::Offset(
                        current_segment_offset,
                        current_segment_direction.unwrap(),
                    ));
                    current_segment_offset = OffsetIn::ZERO;
                    current_segment_direction = None;
                }
                in_then_segment = true;
            }
            Attribute::Chop => {
                style.chop = true;
            }
            Attribute::Fit => {
                // cref: pik_size_to_fit (pikchr.c:3754-3782)
                // Compute fit using current state (text, width, height) just like C does
                style.fit = true;

                if !text.is_empty() {
                    let charwid = ctx.get_scalar("charwid", defaults::CHARWID);
                    let charht = ctx.get_scalar("charht", defaults::FONT_SIZE);
                    let sw = style.stroke_width.raw();

                    // Calculate text bounding box width using jw offset like C does
                    // cref: pik_append_txt (pikchr.c:2466-2508)
                    // For shapes with eJust==1 (box, cylinder, file, oval), ljust/rjust
                    // text is offset inward from edges by jw
                    let has_ejust = matches!(
                        class_name,
                        Some(ClassName::Box)
                            | Some(ClassName::Cylinder)
                            | Some(ClassName::File)
                            | Some(ClassName::Oval)
                    );
                    let jw = if has_ejust {
                        0.5 * (width.raw() - 0.5 * (charwid + sw))
                    } else {
                        0.0
                    };

                    // Compute bbox x-range for all text lines
                    let mut bbox_min_x = 0.0_f64;
                    let mut bbox_max_x = 0.0_f64;
                    for t in &text {
                        let cw = t.width_inches(charwid);
                        let nx = if t.ljust {
                            -jw
                        } else if t.rjust {
                            jw
                        } else {
                            0.0
                        };
                        // Text extent depends on justification
                        let (x0, x1) = if t.rjust {
                            (nx, nx - cw) // text extends left from anchor
                        } else if t.ljust {
                            (nx, nx + cw) // text extends right from anchor
                        } else {
                            (nx - cw / 2.0, nx + cw / 2.0) // centered
                        };
                        bbox_min_x = bbox_min_x.min(x0).min(x1);
                        bbox_max_x = bbox_max_x.max(x0).max(x1);
                    }
                    let bbox_width = bbox_max_x - bbox_min_x;
                    let fit_width = Inches(bbox_width + charwid);

                    // Calculate text bounding box height using vertical slots
                    let y_base = match class_name {
                        Some(ClassName::Cylinder) => {
                            // corner_radius was initialized to cylrad before attributes
                            // C code only applies yBase if rad > 0
                            if style.corner_radius.raw() > 0.0 {
                                -0.75 * style.corner_radius.raw()
                            } else {
                                0.0
                            }
                        }
                        _ => 0.0,
                    };

                    let vslots = compute_text_vslots(&text);
                    let mut hc = 0.0_f64;
                    let mut ha1 = 0.0_f64;
                    let mut ha2 = 0.0_f64;
                    let mut hb1 = 0.0_f64;
                    let mut hb2 = 0.0_f64;

                    for (i, t) in text.iter().enumerate() {
                        let h = t.height(charht);
                        match vslots.get(i).unwrap_or(&TextVSlot::Center) {
                            TextVSlot::Center => hc = hc.max(h),
                            TextVSlot::Above => ha1 = ha1.max(h),
                            TextVSlot::Above2 => ha2 = ha2.max(h),
                            TextVSlot::Below => hb1 = hb1.max(h),
                            TextVSlot::Below2 => hb2 = hb2.max(h),
                        }
                    }

                    let mut bbox_min_y = f64::MAX;
                    let mut bbox_max_y = f64::MIN;
                    for (i, t) in text.iter().enumerate() {
                        let slot = vslots.get(i).unwrap_or(&TextVSlot::Center);
                        let y_offset = match slot {
                            TextVSlot::Above2 => 0.5 * hc + ha1 + 0.5 * ha2,
                            TextVSlot::Above => 0.5 * hc + 0.5 * ha1,
                            TextVSlot::Center => 0.0,
                            TextVSlot::Below => -(0.5 * hc + 0.5 * hb1),
                            TextVSlot::Below2 => -(0.5 * hc + hb1 + 0.5 * hb2),
                        };
                        let y = y_base + y_offset;
                        let ch = charht * 0.5 * t.font_scale();
                        bbox_min_y = bbox_min_y.min(y - ch);
                        bbox_max_y = bbox_max_y.max(y + ch);
                    }

                    let h1 = bbox_max_y;
                    let h2 = -bbox_min_y;
                    let fit_height = Inches(2.0 * h1.max(h2) + 0.5 * charht);

                    tracing::debug!(
                        bbox_min_y = bbox_min_y,
                        bbox_max_y = bbox_max_y,
                        h1 = h1,
                        h2 = h2,
                        charht = charht,
                        fit_height = fit_height.raw(),
                        "[Rust fit height calculation]"
                    );

                    // Apply shape-specific fit logic
                    match class_name {
                        Some(ClassName::Circle) => {
                            let w = fit_width.raw();
                            let h = fit_height.raw();
                            let mut mx = w.max(h);
                            if w > 0.0 && h > 0.0 && (w * w + h * h) > mx * mx {
                                mx = w.hypot(h);
                            }
                            width = Inches(mx);
                            height = Inches(mx);
                        }
                        Some(ClassName::Cylinder) => {
                            // corner_radius was initialized to cylrad before attributes
                            width = fit_width;
                            height = fit_height + style.corner_radius * 0.25 + style.stroke_width;
                        }
                        Some(ClassName::Diamond) => {
                            // cref: diamondFit (pikchr.c:1418-1430)
                            // Use current width/height (set by earlier attributes, or defaults)
                            let mut w = width.raw();
                            let mut h = height.raw();
                            if w <= 0.0 {
                                w = fit_width.raw() * 1.5;
                            }
                            if h <= 0.0 {
                                h = fit_height.raw() * 1.5;
                            }
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
                            let rad = ctx.get_length("filerad", 0.15);
                            width = fit_width;
                            height = fit_height + rad * 2.0;
                        }
                        Some(ClassName::Oval) => {
                            width = fit_width.max(fit_height);
                            height = fit_height;
                        }
                        _ => {
                            width = fit_width;
                            height = fit_height;
                        }
                    }
                    update_current_object(ctx, class_name, width, height, &style);
                }
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
            Attribute::Behind(obj_ref) => {
                // Lower the layer of the current object so that it is behind the given object
                // cref: pik_behind (pikchr.c:3500-3505)
                if let Some(other) = resolve_object(ctx, obj_ref) {
                    // Set our layer to one less than the other object's layer
                    // We'll apply this after creating the object
                    layer = other.layer - 1;
                }
            }
            _ => {}
        }
    }

    // Apply auto-fit for Text class objects (they always get auto-fitted)
    // cref: textOffset (pikchr.c:4416) - text objects always get auto-fitted
    // Normal fit is handled inline when Attribute::Fit is encountered
    let should_fit = class == ClassName::Text && !style.fit;
    if should_fit && !text.is_empty() {
        let charwid = ctx.get_scalar("charwid", defaults::CHARWID);
        let charht = ctx.get_scalar("charht", defaults::FONT_SIZE);

        // For box-style shapes (eJust=1), C computes bbox with jw-based offsets
        // jw is computed from the CURRENT object width (default boxwid/cylwid)
        // cref: pik_append_txt (pikchr.c:5144-5187)
        let uses_box_justification = matches!(class, ClassName::Box | ClassName::Cylinder);
        let current_width = if uses_box_justification {
            match class {
                ClassName::Box => eval::get_length(ctx, "boxwid", 0.75),
                ClassName::Cylinder => eval::get_length(ctx, "cylwid", 1.0),
                _ => 0.0,
            }
        } else {
            0.0
        };
        let sw = style.stroke_width.0;
        let jw = if uses_box_justification {
            0.5 * (current_width - 0.5 * (charwid + sw))
        } else {
            0.0
        };

        // Calculate text bounding box including jw-based position offsets
        // cref: pik_append_txt (pikchr.c:5173-5187)
        let mut bbox_min_x = f64::MAX;
        let mut bbox_max_x = f64::MIN;
        for t in &text {
            let cw = t.width_inches(charwid);
            let nx = if t.ljust {
                -jw // ljust shifts text left from center
            } else if t.rjust {
                jw // rjust shifts text right from center
            } else {
                0.0
            };

            // Compute x extent based on alignment
            let (x0, x1) = if t.rjust {
                (nx, nx - cw) // text extends left from anchor
            } else if t.ljust {
                (nx, nx + cw) // text extends right from anchor
            } else {
                (nx + cw / 2.0, nx - cw / 2.0) // centered
            };

            bbox_min_x = bbox_min_x.min(x0).min(x1);
            bbox_max_x = bbox_max_x.max(x0).max(x1);
        }
        let bbox_width = bbox_max_x - bbox_min_x;

        // C pikchr fit: w = (bbox.ne.x - bbox.sw.x) + charWidth
        let fit_width = Inches(bbox_width + charwid);

        // C pikchr fit: h = 2.0 * max(h1, h2) + 0.5 * charHeight
        // cref: pik_size_to_fit (pikchr.c:6461-6466) - computes h1, h2 from actual text bbox
        // cref: pik_append_txt (pikchr.c:5104-5143) - computes region heights
        //
        // Compute heights for each slot using each text's font scale
        let vslots = compute_text_vslots(&text);

        let mut hc = 0.0_f64; // center height
        let mut ha1 = 0.0_f64; // above height
        let mut ha2 = 0.0_f64; // above2 height
        let mut hb1 = 0.0_f64; // below height
        let mut hb2 = 0.0_f64; // below2 height

        for (t, slot) in text.iter().zip(vslots.iter()) {
            let h = t.height(charht);
            match slot {
                TextVSlot::Center => hc = hc.max(h),
                TextVSlot::Above => ha1 = ha1.max(h),
                TextVSlot::Above2 => ha2 = ha2.max(h),
                TextVSlot::Below => hb1 = hb1.max(h),
                TextVSlot::Below2 => hb2 = hb2.max(h),
            }
        }

        // Compute actual bbox y-extents like C's pik_append_txt
        // For each text: y_position +/- (0.5 * charht * fontScale)
        // cref: pik_append_txt (pikchr.c:5162-5188)
        //
        // For shapes with yBase offset (like cylinder), text is shifted from center.
        // cref: pik_append_txt (pikchr.c:5102-5104) - cylinder yBase = -0.75 * rad
        let y_base = match class {
            ClassName::Cylinder => {
                // corner_radius was initialized to cylrad before attributes
                -0.75 * style.corner_radius.raw()
            }
            _ => 0.0,
        };

        let mut bbox_max_y = f64::MIN;
        let mut bbox_min_y = f64::MAX;

        for (t, slot) in text.iter().zip(vslots.iter()) {
            // Compute y position offset (same as C's y calculation)
            // Start from yBase, then add slot-specific offset
            let y = y_base
                + match slot {
                    TextVSlot::Above2 => 0.5 * hc + ha1 + 0.5 * ha2,
                    TextVSlot::Above => 0.5 * hc + 0.5 * ha1,
                    TextVSlot::Center => 0.0,
                    TextVSlot::Below => -(0.5 * hc + 0.5 * hb1),
                    TextVSlot::Below2 => -(0.5 * hc + hb1 + 0.5 * hb2),
                };
            // Character half-height, scaled by font scale
            let ch = charht * 0.5 * t.font_scale();
            // Text extends from y-ch to y+ch
            bbox_max_y = bbox_max_y.max(y + ch);
            bbox_min_y = bbox_min_y.min(y - ch);
        }

        // h1 = extent above center, h2 = extent below center
        let h1 = bbox_max_y; // max y relative to center
        let h2 = -bbox_min_y; // min y (negative) relative to center

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
                tracing::debug!(
                    rad_inches = radius.raw(),
                    rad_px = radius.raw() * 144.0,
                    "circleFit final"
                );
                width = diameter;
                height = diameter;
            }
            ClassName::Cylinder => {
                // cref: cylinderFit (pikchr.c:3976)
                // corner_radius was initialized to cylrad before attributes
                width = fit_width;
                height = fit_height + style.corner_radius * 0.25 + style.stroke_width;
                tracing::debug!(
                    fit_height = fit_height.raw(),
                    rad = style.corner_radius.raw(),
                    sw = style.stroke_width.raw(),
                    result_height = height.raw(),
                    "cylinderFit calculation"
                );
            }
            ClassName::Diamond => {
                // cref: diamondFit (pikchr.c:4096)
                // Use default shape dimensions (diamondwid=1.0, diamondht=0.75) in formula
                let mut w = width.raw(); // Already set to diamondwid default
                let mut h = height.raw(); // Already set to diamondht default
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

    // Save final pending then segment if there is one
    // cref: C pikchr saves the current path point when processing completes
    if in_then_segment && current_segment_direction.is_some() {
        segments.push(Segment::Offset(
            current_segment_offset,
            current_segment_direction.unwrap(),
        ));
    }

    // Calculate position based on object type
    tracing::debug!(
        ?class,
        from_position = from_position.is_some(),
        to_positions_count = to_positions.len(),
        has_direction_move,
        segments_count = segments.len(),
        even_clause = even_clause.is_some(),
        with_clause = with_clause.is_some(),
        "position branch conditions"
    );
    let (center, start, end, waypoints) = if from_position.is_some()
        || !to_positions.is_empty()
        || has_direction_move
        || !segments.is_empty()
        || even_clause.is_some()
    {
        // Line-like objects with explicit from/to, direction moves, or then clauses
        // Determine start position based on direction of movement
        let start = if let Some(pos) = from_position {
            tracing::debug!(
                from_x = pos.x.raw(),
                from_y = pos.y.raw(),
                "start: from explicit from_position"
            );
            pos
        } else if !to_positions.is_empty() && has_direction_move {
            // "move to X down Y" - start FROM the to_position, not from current cursor
            // The direction offset will be applied from this point
            tracing::debug!(
                to_x = to_positions[0].x.raw(),
                to_y = to_positions[0].y.raw(),
                "start: using to_position as start (move to X down Y case)"
            );
            to_positions[0]
        } else if has_direction_move && to_positions.is_empty() {
            // Only use last object's edge when we have direction_move WITHOUT to_positions
            // (e.g., "arrow right 2in" uses last object's edge)
            if let Some(last_obj) = ctx.last_object() {
                // For line-like objects, use the end point directly
                match last_obj.class() {
                    ClassName::Line
                    | ClassName::Arrow
                    | ClassName::Spline
                    | ClassName::Arc
                    | ClassName::Move
                    | ClassName::Dot => last_obj.end(),
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

            if !to_positions.is_empty() && segments.is_empty() && !has_direction_move {
                // from X to Y [to Z...] - add all to_positions as waypoints
                for pos in &to_positions {
                    points.push(*pos);
                }
            } else if has_direction_move || !segments.is_empty() {
                // cref: C pikchr accumulates directions per segment
                // direction_offset = initial segment (before first "then")
                // segments = accumulated offsets for each "then" segment

                // If we have both to_positions and direction moves, the direction offset
                // should be applied AFTER reaching the to_position (e.g., "move to X down 1in")
                // cref: C pikchr handles this in pik_elem_new
                if !to_positions.is_empty() {
                    // Note: if start was set to to_positions[0], we've already pushed it
                    // Skip the first to_position if it equals start to avoid duplicate
                    for (i, pos) in to_positions.iter().enumerate() {
                        if i == 0 && *pos == start {
                            current_pos = *pos;
                            continue; // Skip - already in points as start
                        }
                        points.push(*pos);
                        current_pos = *pos;
                    }
                }

                // Apply initial direction offset (segment before first "then")
                if direction_offset != OffsetIn::ZERO {
                    let next = current_pos + direction_offset;
                    tracing::debug!(
                        start_x = start.x.raw(),
                        start_y = start.y.raw(),
                        current_pos_x = current_pos.x.raw(),
                        current_pos_y = current_pos.y.raw(),
                        direction_offset_dx = direction_offset.dx.raw(),
                        direction_offset_dy = direction_offset.dy.raw(),
                        next_x = next.x.raw(),
                        next_y = next.y.raw(),
                        "Rust: applying initial direction offset"
                    );
                    points.push(next);
                    current_pos = next;
                }

                // Apply each "then" segment's accumulated offset or absolute position
                // cref: each segment is like calling pik_add_direction after a "then"
                for (i, segment) in segments.iter().enumerate() {
                    let next = match segment {
                        Segment::Offset(segment_offset, _segment_dir) => {
                            let next = current_pos + *segment_offset;
                            tracing::debug!(
                                segment_index = i,
                                current_pos_x = current_pos.x.raw(),
                                current_pos_y = current_pos.y.raw(),
                                segment_offset_dx = segment_offset.dx.raw(),
                                segment_offset_dy = segment_offset.dy.raw(),
                                next_x = next.x.raw(),
                                next_y = next.y.raw(),
                                "Rust: applying then segment offset"
                            );
                            next
                        }
                        Segment::AbsolutePosition(pos) => {
                            tracing::debug!(
                                segment_index = i,
                                current_pos_x = current_pos.x.raw(),
                                current_pos_y = current_pos.y.raw(),
                                absolute_x = pos.x.raw(),
                                absolute_y = pos.y.raw(),
                                "Rust: applying then absolute position"
                            );
                            *pos
                        }
                    };
                    points.push(next);
                    current_pos = next;
                }
            } else {
                // No direction moves, no then clauses - default single segment
                let next = move_in_direction(current_pos, ctx.direction, width);
                tracing::debug!(
                    start_x = start.x.raw(),
                    start_y = start.y.raw(),
                    ctx_direction = ?ctx.direction,
                    width = width.raw(),
                    next_x = next.x.raw(),
                    next_y = next.y.raw(),
                    "[Rust line_default_path]"
                );
                points.push(next);
            }

            let end = *points.last().unwrap_or(&start);
            // Compute line center as center of bounding box over all path points
            // cref: pikchr.y:4381-4391 - "the center of a line is the center of its bounding box"
            // This differs from PIC where center is midpoint between start and end
            let mut min_x = f64::MAX;
            let mut max_x = f64::MIN;
            let mut min_y = f64::MAX;
            let mut max_y = f64::MIN;
            for pt in &points {
                min_x = min_x.min(pt.x.raw());
                max_x = max_x.max(pt.x.raw());
                min_y = min_y.min(pt.y.raw());
                max_y = max_y.max(pt.y.raw());
            }
            let center = Point {
                x: Inches((min_x + max_x) / 2.0),
                y: Inches((min_y + max_y) / 2.0),
            };
            tracing::debug!(
                center_x = center.x.raw(),
                center_y = center.y.raw(),
                start_x = start.x.raw(),
                start_y = start.y.raw(),
                end_x = end.x.raw(),
                end_y = end.y.raw(),
                waypoints_len = points.len(),
                "[Rust line_final]"
            );
            (center, start, end, points)
        }
    } else if let Some((edge, target)) = with_clause {
        // Position object so that specified edge is at target position
        let center = calculate_center_from_edge(edge, target, width, height, class, ctx.direction);
        let (_, s, e) = calculate_object_position_at(ctx.direction, center, width, height);
        (center, s, e, vec![s, e])
    } else if let Some(pos) = explicit_position {
        // Box-like objects with explicit "at" position
        let (_, s, e) = calculate_object_position_at(ctx.direction, pos, width, height);
        (pos, s, e, vec![s, e])
    } else {
        let (c, s, e) = calculate_object_position(ctx, class, width, height);
        tracing::debug!(
            center_x = c.x.raw(),
            center_y = c.y.raw(),
            start_x = s.x.raw(),
            start_y = s.y.raw(),
            end_x = e.x.raw(),
            end_y = e.y.raw(),
            ctx_direction = ?ctx.direction,
            "[Rust calculate_object_position for {:?}]", class
        );
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
            // corner_radius was initialized to cylrad before attributes
            ellipse_rad: style.corner_radius,
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
            // cref: splineInit (pikchr.c:1656) - pObj->rad = 1000
            radius: Inches(1000.0),
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
        ClassName::Dot => {
            // C: renders with r = pObj->rad, but sets w = rad * 6
            // So: radius = width / 6
            let radius = width / 6.0;
            tracing::debug!(
                center_x = center.x.raw(),
                center_y = center.y.raw(),
                radius = radius.raw(),
                "[Rust dot created]"
            );
            ShapeEnum::Dot(DotShape {
                center,
                radius,
                style: style.clone(),
                text: text.clone(),
            })
        }
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

    tracing::debug!(
        name = ?final_name,
        class = ?class,
        layer = layer,
        "creating RenderedObject"
    );

    Ok(RenderedObject {
        name: final_name,
        shape,
        start_attachment: from_attachment,
        end_attachment: to_attachment,
        layer,
        direction: object_direction,
        class_name: class,
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
/// cref: pik_place_adjust (pikchr.c:5829) - adjusts position based on edge
/// cref: pik_set_at (pikchr.c:6195-6199) - converts Start/End to compass points
fn calculate_center_from_edge(
    edge: EdgePoint,
    target: PointIn,
    width: Inches,
    height: Inches,
    class: ClassName,
    direction: Direction,
) -> PointIn {
    // Convert Start/End to compass points based on direction
    // cref: pik_set_at - eDirToCp maps direction to compass point:
    //   Right -> East, Down -> South, Left -> West, Up -> North
    // For End: use outDir (same as current direction)
    // For Start: use (inDir+2)%4, which is OPPOSITE of direction
    let edge = match edge {
        EdgePoint::Start => {
            // Start is at the entry edge (opposite of direction)
            match direction {
                Direction::Right => EdgePoint::West,
                Direction::Down => EdgePoint::North,
                Direction::Left => EdgePoint::East,
                Direction::Up => EdgePoint::South,
            }
        }
        EdgePoint::End => {
            // End is at the exit edge (same as direction)
            match direction {
                Direction::Right => EdgePoint::East,
                Direction::Down => EdgePoint::South,
                Direction::Left => EdgePoint::West,
                Direction::Up => EdgePoint::North,
            }
        }
        other => other,
    };

    if matches!(edge, EdgePoint::Center | EdgePoint::C) {
        return target;
    }

    let hw = width / 2.0;
    let hh = height / 2.0;

    // For diagonal corners on rectangular shapes, the corner is at the full (hw, hh) distance.
    // For round shapes, the diagonal point is on the perimeter at (0.707*r, 0.707*r).
    // For cardinal directions (N/S/E/W), the offset is simply (0, hh) or (hw, 0).
    let is_diagonal = matches!(
        edge,
        EdgePoint::NorthEast | EdgePoint::NorthWest | EdgePoint::SouthEast | EdgePoint::SouthWest
    );

    let offset = if is_diagonal && !class.is_round() {
        // For box-like shapes, diagonal corners are at full (hw, hh) with appropriate signs
        let unit = edge.to_unit_vec();
        let sign_x = unit.dx().signum();
        let sign_y = unit.dy().signum();
        OffsetIn::new(Inches(sign_x * hw.0), Inches(sign_y * hh.0))
    } else {
        // For round shapes with diagonal edges OR any cardinal direction:
        // The diagonal unit vectors already have 1/√2 built in (e.g., SOUTH_EAST.dx = 0.707)
        // So we just scale by hw/hh directly to get the correct perimeter point.
        // cref: pik_elem_bbox (pikchr.c:3788-3798) - uses rx = (1-1/√2)*rad, then pt = w2-rx = rad/√2
        edge.to_unit_vec().scale_xy(hw, hh)
    };

    // Edge point = center + offset, so center = edge point - offset
    let center = target - offset;

    tracing::debug!(
        ?edge,
        target_x = target.x.0,
        target_y = target.y.0,
        width = width.0,
        height = height.0,
        offset_x = offset.dx.0,
        offset_y = offset.dy.0,
        center_x = center.x.0,
        center_y = center.y.0,
        is_round = class.is_round(),
        "[calculate_center_from_edge]"
    );

    center
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
