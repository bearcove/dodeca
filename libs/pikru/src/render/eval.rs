//! Expression evaluation functions

use crate::ast::*;
use crate::types::{Angle, EvalValue, Length as Inches, OffsetIn, Point};

use super::context::RenderContext;
use super::types::*;

// From implementations for EvalValue
impl From<f64> for EvalValue {
    fn from(value: f64) -> Self {
        EvalValue::Scalar(value)
    }
}

impl From<f32> for EvalValue {
    fn from(value: f32) -> Self {
        EvalValue::Scalar(value as f64)
    }
}

// From implementations for Length (Inches)
impl From<f64> for Inches {
    fn from(value: f64) -> Self {
        Inches::inches(value)
    }
}

impl From<f32> for Inches {
    fn from(value: f32) -> Self {
        Inches::inches(value as f64)
    }
}

pub fn eval_expr(ctx: &RenderContext, expr: &Expr) -> Result<Value, miette::Report> {
    match expr {
        Expr::Number(n) => {
            // Validate user-provided numbers at entry point
            let len = Inches::try_new(*n)
                .map_err(|e| miette::miette!("Invalid numeric literal: {}", e))?;
            Ok(Value::Len(len))
        }
        Expr::Variable(name) => {
            // cref: pik_get_var (pikchr.c:6625) - falls back to color lookup
            if let Some(val) = ctx.variables.get(name) {
                Ok(Value::from(*val))
            } else {
                // Try parsing as a color name (always succeeds, returns Raw if unknown)
                let color = name.parse::<crate::types::Color>().unwrap();
                let rgb_str = color.to_rgb_string();
                if let Some(rgb) = rgb_str.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
                    let parts: Vec<&str> = rgb.split(',').collect();
                    if parts.len() == 3 {
                        if let (Ok(r), Ok(g), Ok(b)) = (
                            parts[0].trim().parse::<u32>(),
                            parts[1].trim().parse::<u32>(),
                            parts[2].trim().parse::<u32>(),
                        ) {
                            let color_val = (r << 16) | (g << 8) | b;
                            return Ok(Value::from(EvalValue::Color(color_val)));
                        }
                    }
                }
                Err(miette::miette!("Undefined variable: {}", name))
            }
        }
        Expr::BuiltinVar(b) => {
            let key = match b {
                BuiltinVar::Fill => "fill",
                BuiltinVar::Color => "color",
                BuiltinVar::Thickness => "thickness",
            };
            ctx.variables
                .get(key)
                .copied()
                .map(Value::from)
                .ok_or_else(|| miette::miette!("Undefined builtin: {}", key))
        }
        Expr::BinaryOp(lhs, op, rhs) => {
            let l = eval_expr(ctx, lhs)?;
            let r = eval_expr(ctx, rhs)?;
            use Value::*;
            let result = match (l, r, op) {
                // Length + Length = Length (typed op)
                (Len(a), Len(b), BinaryOp::Add) => Len(a + b),
                // Length - Length = Length (typed op)
                (Len(a), Len(b), BinaryOp::Sub) => Len(a - b),
                // Length * Length = Scalar (area-like, unitless)
                (Len(a), Len(b), BinaryOp::Mul) => Scalar(a.raw() * b.raw()),
                // Length / Length = Scalar (ratio, via checked_div)
                (Len(a), Len(b), BinaryOp::Div) => a
                    .checked_div(b)
                    .map(|s| Scalar(s.raw()))
                    .ok_or_else(|| miette::miette!("Division by zero"))?,
                // Length + Scalar: treat scalar as length (C compatibility)
                (Len(a), Scalar(b), BinaryOp::Add) => Len(a + Inches::inches(b)),
                (Len(a), Scalar(b), BinaryOp::Sub) => Len(a - Inches::inches(b)),
                // Length * Scalar = Length (scaling, typed op)
                (Len(a), Scalar(b), BinaryOp::Mul) => Len(a * b),
                // Length / Scalar = Length (typed op)
                (Len(a), Scalar(b), BinaryOp::Div) => {
                    if b == 0.0 {
                        return Err(miette::miette!("Division by zero"));
                    }
                    Len(a / b)
                }
                // Scalar + Length: treat scalar as length (C compatibility)
                (Scalar(a), Len(b), BinaryOp::Add) => Len(Inches::inches(a) + b),
                (Scalar(a), Len(b), BinaryOp::Sub) => Len(Inches::inches(a) - b),
                // Scalar * Length = Length (scaling, typed op)
                (Scalar(a), Len(b), BinaryOp::Mul) => Len(crate::types::Scalar(a) * b),
                // Scalar / Length = Scalar (inverse scaling)
                (Scalar(a), Len(b), BinaryOp::Div) => {
                    if b.raw() == 0.0 {
                        return Err(miette::miette!("Division by zero"));
                    }
                    Scalar(a / b.raw())
                }
                // Scalar ops (unitless)
                (Scalar(a), Scalar(b), BinaryOp::Add) => Scalar(a + b),
                (Scalar(a), Scalar(b), BinaryOp::Sub) => Scalar(a - b),
                (Scalar(a), Scalar(b), BinaryOp::Mul) => Scalar(a * b),
                (Scalar(a), Scalar(b), BinaryOp::Div) => {
                    if b == 0.0 {
                        return Err(miette::miette!("Division by zero"));
                    }
                    Scalar(a / b)
                }
            };
            // Validate result is finite (catches overflow to infinity)
            validate_value(result)
        }
        Expr::UnaryOp(op, e) => {
            let v = eval_expr(ctx, e)?;
            Ok(match (op, v) {
                (UnaryOp::Neg, Value::Len(l)) => Value::Len(-l), // typed Neg op
                (UnaryOp::Pos, Value::Len(l)) => Value::Len(l),
                (UnaryOp::Neg, Value::Scalar(s)) => Value::Scalar(-s),
                (UnaryOp::Pos, Value::Scalar(s)) => Value::Scalar(s),
            })
        }
        Expr::ParenExpr(e) => eval_expr(ctx, e),
        Expr::FuncCall(fc) => {
            let args: Result<Vec<Value>, _> = fc.args.iter().map(|a| eval_expr(ctx, a)).collect();
            let args = args?;
            use Value::*;
            let result = match fc.func {
                Function::Abs => match args[0] {
                    Len(l) => Len(l.abs()), // typed abs
                    Scalar(s) => Scalar(s.abs()),
                },
                Function::Cos => {
                    let v = match args[0] {
                        Len(l) => l.raw(),
                        Scalar(s) => s,
                    };
                    Scalar(v.to_radians().cos())
                }
                Function::Sin => {
                    let v = match args[0] {
                        Len(l) => l.raw(),
                        Scalar(s) => s,
                    };
                    Scalar(v.to_radians().sin())
                }
                Function::Int => match args[0] {
                    Len(l) => Len(Inches::inches(l.raw().trunc())),
                    Scalar(s) => Scalar(s.trunc()),
                },
                Function::Sqrt => match args[0] {
                    Len(l) if l.raw() < 0.0 => return Err(miette::miette!("sqrt of negative")),
                    Len(l) => Len(Inches::inches(l.raw().sqrt())),
                    Scalar(s) if s < 0.0 => return Err(miette::miette!("sqrt of negative")),
                    Scalar(s) => Scalar(s.sqrt()),
                },
                Function::Max => {
                    let a = match args[0] {
                        Len(l) => l.raw(),
                        Scalar(s) => s,
                    };
                    let b = match args[1] {
                        Len(l) => l.raw(),
                        Scalar(s) => s,
                    };
                    Scalar(a.max(b))
                }
                Function::Min => {
                    let a = match args[0] {
                        Len(l) => l.raw(),
                        Scalar(s) => s,
                    };
                    let b = match args[1] {
                        Len(l) => l.raw(),
                        Scalar(s) => s,
                    };
                    Scalar(a.min(b))
                }
            };
            validate_value(result)
        }
        Expr::DistCall(p1, p2) => {
            let a = eval_position(ctx, p1)?;
            let b = eval_position(ctx, p2)?;
            // Use typed subtraction: Point - Point = Offset
            let offset = b - a;
            let dist = Inches::inches((offset.dx.raw().powi(2) + offset.dy.raw().powi(2)).sqrt());
            Ok(Value::Len(dist))
        }
        Expr::ObjectProp(obj, prop) => {
            let r = resolve_object(ctx, obj)
                .ok_or_else(|| miette::miette!("Unknown object in property lookup"))?;
            let val = match prop {
                NumProperty::Width => r.width(),
                NumProperty::Height => r.height(),
                NumProperty::Radius | NumProperty::Diameter => {
                    r.width().min(r.height()) / 2.0 // typed min and div
                }
                NumProperty::Thickness => r.style().stroke_width,
            };
            Ok(Value::Len(val))
        }
        Expr::ObjectCoord(obj, coord) => {
            let r = resolve_object(ctx, obj)
                .ok_or_else(|| miette::miette!("Unknown object in coord lookup"))?;
            Ok(Value::Len(match coord {
                Coord::X => r.center().x,
                Coord::Y => r.center().y,
            }))
        }
        Expr::ObjectEdgeCoord(obj, edge, coord) => {
            let r = resolve_object(ctx, obj)
                .ok_or_else(|| miette::miette!("Unknown object in edge coord lookup"))?;
            let pt = get_edge_point(r, edge);
            Ok(Value::Len(match coord {
                Coord::X => pt.x,
                Coord::Y => pt.y,
            }))
        }
        Expr::VertexCoord(nth, obj, coord) => {
            let r = resolve_object(ctx, obj)
                .ok_or_else(|| miette::miette!("Unknown object in vertex coord lookup"))?;
            let target = match nth {
                Nth::First(_) | Nth::Ordinal(1, _, _) => r.start(),
                Nth::Last(_) => r.end(),
                _ => r.center(),
            };
            Ok(Value::Len(match coord {
                Coord::X => target.x,
                Coord::Y => target.y,
            }))
        }
        Expr::PlaceName(name) => Err(miette::miette!(
            "Unsupported place name in expression: {}",
            name
        )),
    }
}

pub fn eval_len(ctx: &RenderContext, expr: &Expr) -> Result<Inches, miette::Report> {
    match eval_expr(ctx, expr)? {
        Value::Len(l) => Ok(l),
        Value::Scalar(s) => Ok(Inches(s)), // treat scalar as inches for len contexts
    }
}

pub fn eval_scalar(ctx: &RenderContext, expr: &Expr) -> Result<f64, miette::Report> {
    match eval_expr(ctx, expr)? {
        Value::Scalar(s) => Ok(s),
        Value::Len(l) => Ok(l.0),
    }
}

pub fn eval_rvalue(ctx: &RenderContext, rvalue: &RValue) -> Result<Value, miette::Report> {
    match rvalue {
        RValue::Expr(e) => eval_expr(ctx, e),
        RValue::PlaceName(name) => {
            // Try to parse as a color name
            let color = name.parse::<crate::types::Color>().unwrap();
            let rgb_str = color.to_rgb_string();
            if let Some(rgb) = rgb_str.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
                let parts: Vec<&str> = rgb.split(',').collect();
                if parts.len() == 3 {
                    if let (Ok(r), Ok(g), Ok(b)) = (
                        parts[0].trim().parse::<u32>(),
                        parts[1].trim().parse::<u32>(),
                        parts[2].trim().parse::<u32>(),
                    ) {
                        let color_val = (r << 16) | (g << 8) | b;
                        return Ok(Value::from(EvalValue::Color(color_val)));
                    }
                }
            }
            Ok(Value::Scalar(0.0))
        }
    }
}

pub fn eval_position(ctx: &RenderContext, pos: &Position) -> Result<PointIn, miette::Report> {
    match pos {
        Position::Coords(x, y) => {
            let px = eval_len(ctx, x)?;
            let py = eval_len(ctx, y)?;
            Ok(Point::new(px, py))
        }
        Position::Place(place) => {
            let result = eval_place(ctx, place)?;
            tracing::debug!(
                ?place,
                result_x = result.x.0,
                result_y = result.y.0,
                "Position::Place"
            );
            Ok(result)
        }
        Position::PlaceOffset(place, op, dx, dy) => {
            let base = eval_place(ctx, place)?;
            let dx_val = eval_len(ctx, dx)?;
            let dy_val = eval_len(ctx, dy)?;
            let offset = OffsetIn::new(dx_val, dy_val);
            match op {
                BinaryOp::Add => Ok(base + offset),
                BinaryOp::Sub => Ok(base - offset),
                _ => Ok(base),
            }
        }
        Position::Between(factor, pos1, pos2) => {
            let f = eval_scalar(ctx, factor)?;
            let p1 = eval_position(ctx, pos1)?;
            let p2 = eval_position(ctx, pos2)?;
            // Interpolate: p1 + (p2 - p1) * f
            let result = p1 + (p2 - p1) * f;
            tracing::debug!(
                f = f,
                p1_x = p1.x.0,
                p1_y = p1.y.0,
                p2_x = p2.x.0,
                p2_y = p2.y.0,
                result_x = result.x.0,
                result_y = result.y.0,
                "Position::Between calculation"
            );
            Ok(result)
        }
        Position::Bracket(factor, pos1, pos2) => {
            // Same as between: p1 + (p2 - p1) * f
            let f = eval_scalar(ctx, factor)?;
            let p1 = eval_position(ctx, pos1)?;
            let p2 = eval_position(ctx, pos2)?;
            Ok(p1 + (p2 - p1) * f)
        }
        Position::AboveBelow(dist, ab, base_pos) => {
            let d = eval_len(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            // Y-up: Above = +Y, Below = -Y
            let offset = match ab {
                AboveBelow::Above => OffsetIn::new(Inches::ZERO, d),
                AboveBelow::Below => OffsetIn::new(Inches::ZERO, -d),
            };
            Ok(base + offset)
        }
        Position::LeftRightOf(dist, lr, base_pos) => {
            let d = eval_len(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            let offset = match lr {
                LeftRight::Left => OffsetIn::new(-d, Inches::ZERO),
                LeftRight::Right => OffsetIn::new(d, Inches::ZERO),
            };
            Ok(base + offset)
        }
        Position::EdgePointOf(dist, edge, base_pos) => {
            let d = eval_len(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            // Calculate offset based on edge direction
            let dir = edge.to_unit_vec();
            Ok(base + dir * d)
        }
        Position::Heading(dist, heading, base_pos) => {
            let d = eval_len(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            let angle = match heading {
                HeadingDir::EdgePoint(ep) => ep.to_angle(),
                HeadingDir::Expr(e) => Angle::degrees(eval_scalar(ctx, e).unwrap_or(0.0)),
            };
            // C pikchr uses: pt.x += dist*sin(r); pt.y += dist*cos(r);
            // We use the same Y-up convention internally, flip happens in to_svg().
            let rad = angle.to_radians();
            Ok(Point::new(
                base.x + Inches(d.0 * rad.sin()),
                base.y + Inches(d.0 * rad.cos()),
            ))
        }
        Position::Tuple(pos1, pos2) => {
            // Extract x from pos1, y from pos2
            let p1 = eval_position(ctx, pos1)?;
            let p2 = eval_position(ctx, pos2)?;
            Ok(Point::new(p1.x, p2.y))
        }
    }
}

pub fn endpoint_object_from_position(
    ctx: &RenderContext,
    pos: &Position,
) -> Option<EndpointObject> {
    match pos {
        Position::Place(place) => endpoint_object_from_place(ctx, place),
        // Extract underlying Place from offset positions (e.g., C0.ne + (0.05,0))
        Position::PlaceOffset(place, _, _, _) => endpoint_object_from_place(ctx, place),
        // For "between A and B", use the first position's object for chop
        Position::Between(_, pos1, _) => endpoint_object_from_position(ctx, pos1),
        // For "above/below X", extract from X
        Position::AboveBelow(_, _, inner) => endpoint_object_from_position(ctx, inner),
        // For angle bracket <A, B>, use first position
        Position::Bracket(_, pos1, _) => endpoint_object_from_position(ctx, pos1),
        _ => None,
    }
}

fn endpoint_object_from_place(ctx: &RenderContext, place: &Place) -> Option<EndpointObject> {
    match place {
        Place::Object(obj)
        | Place::ObjectEdge(obj, _)
        | Place::EdgePointOf(_, obj)
        | Place::Vertex(_, obj) => resolve_object(ctx, obj).map(EndpointObject::from_rendered),
    }
}

fn eval_place(ctx: &RenderContext, place: &Place) -> Result<PointIn, miette::Report> {
    match place {
        Place::Object(obj) => {
            if let Some(rendered) = resolve_object(ctx, obj) {
                Ok(rendered.center())
            } else {
                Ok(ctx.position)
            }
        }
        Place::ObjectEdge(obj, edge) => {
            if let Some(rendered) = resolve_object(ctx, obj) {
                Ok(get_edge_point(rendered, edge))
            } else {
                Ok(ctx.position)
            }
        }
        Place::EdgePointOf(edge, obj) => {
            if let Some(rendered) = resolve_object(ctx, obj) {
                Ok(get_edge_point(rendered, edge))
            } else {
                Ok(ctx.position)
            }
        }
        Place::Vertex(nth, obj) => {
            // For now, just return the start or end point
            if let Some(rendered) = resolve_object(ctx, obj) {
                match nth {
                    Nth::First(_) | Nth::Ordinal(1, _, _) => Ok(rendered.start()),
                    Nth::Last(_) => Ok(rendered.end()),
                    _ => Ok(rendered.center()),
                }
            } else {
                Ok(ctx.position)
            }
        }
    }
}

pub fn resolve_object<'a>(ctx: &'a RenderContext, obj: &Object) -> Option<&'a RenderedObject> {
    match obj {
        Object::Named(name) => {
            // First resolve the base object
            let base_obj = match &name.base {
                ObjectNameBase::PlaceName(n) => ctx.get_object(n),
                ObjectNameBase::This => ctx.current_object.as_ref().or_else(|| ctx.last_object()),
            }?;

            // Then follow the path through sublists (e.g., Main.A -> Main's child A)
            if name.path.is_empty() {
                Some(base_obj)
            } else {
                resolve_path_in_object(base_obj, &name.path)
            }
        }
        Object::Nth(nth) => match nth {
            Nth::Last(class) => {
                let oc = class.as_ref().and_then(|c| nth_class_to_class_name(c));
                ctx.get_last_object(oc)
            }
            Nth::First(class) => {
                let oc = class.as_ref().and_then(|c| nth_class_to_class_name(c));
                ctx.get_nth_object(1, oc)
            }
            Nth::Ordinal(n, _, class) => {
                let oc = class.as_ref().and_then(|c| nth_class_to_class_name(c));
                ctx.get_nth_object(*n as usize, oc)
            }
            Nth::Previous => {
                // "previous" refers to the most recently completed object
                // Since the current object hasn't been added to the list yet,
                // "previous" is the last object in the list
                ctx.object_list.last()
            }
        },
    }
}

/// Resolve a path within an object's children (e.g., ["A"] finds child named "A")
fn resolve_path_in_object<'a>(
    obj: &'a RenderedObject,
    path: &[String],
) -> Option<&'a RenderedObject> {
    if path.is_empty() {
        return Some(obj);
    }

    let (next_name, remaining) = path.split_first().unwrap();
    let children = obj.children()?;
    let child = children
        .iter()
        .find(|child| child.name.as_deref() == Some(next_name.as_str()))?;

    resolve_path_in_object(child, remaining)
}

fn nth_class_to_class_name(nc: &NthClass) -> Option<ClassName> {
    match nc {
        NthClass::ClassName(cn) => Some(*cn),
        NthClass::Sublist => Some(ClassName::Sublist),
    }
}

// cref: pik_set_at (pikchr.c:6195-6199) - converts Start/End to compass points
fn get_edge_point(obj: &RenderedObject, edge: &EdgePoint) -> PointIn {
    use crate::ast::Direction;

    // Convert Start/End to compass points based on object's stored direction
    let resolved_edge = match edge {
        EdgePoint::Start => {
            // Start is at the entry edge (opposite of object's direction)
            match obj.direction {
                Direction::Right => EdgePoint::West,
                Direction::Down => EdgePoint::North,
                Direction::Left => EdgePoint::East,
                Direction::Up => EdgePoint::South,
            }
        }
        EdgePoint::End => {
            // End is at the exit edge (same as object's direction)
            match obj.direction {
                Direction::Right => EdgePoint::East,
                Direction::Down => EdgePoint::South,
                Direction::Left => EdgePoint::West,
                Direction::Up => EdgePoint::North,
            }
        }
        other => *other,
    };

    match resolved_edge {
        EdgePoint::Center | EdgePoint::C => obj.center(),
        _ => obj.edge_point(resolved_edge.to_unit_vec()),
    }
}

// cref: pik_get_color_from_name
pub fn eval_color(ctx: &RenderContext, rvalue: &RValue) -> String {
    match rvalue {
        // Color name like "Red", "blue", "lightgray"
        RValue::PlaceName(name) => name
            .parse::<crate::types::Color>()
            .unwrap()
            .to_string(),
        // Expression - could be a variable like $featurecolor or a hex literal
        RValue::Expr(expr) => match expr {
            Expr::Variable(name) => {
                // Look up variable in context
                if let Some(val) = ctx.variables.get(name) {
                    match val {
                        EvalValue::Color(c) => format!("#{:06x}", c),
                        // Scalar or Length could be a hex color value (e.g., 0xfedbce)
                        EvalValue::Scalar(s) => format!("#{:06x}", *s as u32),
                        EvalValue::Length(l) => format!("#{:06x}", l.raw() as u32),
                    }
                } else {
                    // Undefined variable - fall back to parsing as color name
                    name.parse::<crate::types::Color>()
                        .unwrap()
                        .to_string()
                }
            }
            Expr::Number(n) => {
                // Numeric literal like 0xfedbce
                format!("#{:06x}", *n as u32)
            }
            _ => "black".to_string(),
        },
    }
}

/// Helper to extract a length from an EvalValue, with fallback
pub fn get_length(ctx: &RenderContext, name: &str, default: f64) -> f64 {
    ctx.variables
        .get(name)
        .and_then(|v| v.as_length())
        .map(|l| l.raw())
        .unwrap_or(default)
}

/// Helper to extract a scalar from an EvalValue, with fallback
pub fn get_scalar(ctx: &RenderContext, name: &str, default: f64) -> f64 {
    ctx.variables
        .get(name)
        .map(|v| v.as_scalar())
        .unwrap_or(default)
}

/// Validate that a Value is finite (not NaN or infinity from overflow)
fn validate_value(v: Value) -> Result<Value, miette::Report> {
    match v {
        Value::Len(l) if !l.is_finite() => Err(miette::miette!(
            "Arithmetic overflow (result is infinite or NaN)"
        )),
        Value::Scalar(s) if !s.is_finite() => Err(miette::miette!(
            "Arithmetic overflow (result is infinite or NaN)"
        )),
        _ => Ok(v),
    }
}
