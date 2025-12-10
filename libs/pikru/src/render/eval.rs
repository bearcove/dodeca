//! Expression evaluation functions

use crate::ast::*;
use crate::types::{Length as Inches, OffsetIn, Point, UnitVec};

use super::context::RenderContext;
use super::types::*;

pub fn eval_expr(ctx: &RenderContext, expr: &Expr) -> Result<Value, miette::Report> {
    match expr {
        Expr::Number(n) => {
            // Validate user-provided numbers at entry point
            let len = Inches::try_new(*n)
                .map_err(|e| miette::miette!("Invalid numeric literal: {}", e))?;
            Ok(Value::Len(len))
        }
        Expr::Variable(name) => ctx
            .variables
            .get(name)
            .copied()
            .map(Value::from)
            .ok_or_else(|| miette::miette!("Undefined variable: {}", name)),
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
                NumProperty::Width => r.width,
                NumProperty::Height => r.height,
                NumProperty::Radius | NumProperty::Diameter => {
                    r.width.min(r.height) / 2.0 // typed min and div
                }
                NumProperty::Thickness => r.style.stroke_width,
            };
            Ok(Value::Len(val))
        }
        Expr::ObjectCoord(obj, coord) => {
            let r = resolve_object(ctx, obj)
                .ok_or_else(|| miette::miette!("Unknown object in coord lookup"))?;
            Ok(Value::Len(match coord {
                Coord::X => r.center.x,
                Coord::Y => r.center.y,
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
                Nth::First(_) | Nth::Ordinal(1, _, _) => r.start,
                Nth::Last(_) => r.end,
                _ => r.center,
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
        RValue::PlaceName(_) => Ok(Value::Scalar(0.0)), // Color names return 0
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
            let dir = edge_point_offset(edge);
            Ok(base + dir * d)
        }
        Position::Heading(dist, heading, base_pos) => {
            let d = eval_len(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            let angle = match heading {
                HeadingDir::EdgePoint(ep) => edge_point_to_angle(ep),
                HeadingDir::Expr(e) => eval_scalar(ctx, e).unwrap_or(0.0),
            };
            // Convert angle (degrees, 0 = north, clockwise) to radians
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

pub fn endpoint_object_from_position(ctx: &RenderContext, pos: &Position) -> Option<EndpointObject> {
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

/// Get the unit offset direction for an edge point
pub fn edge_point_offset(edge: &EdgePoint) -> UnitVec {
    match edge {
        EdgePoint::North | EdgePoint::N | EdgePoint::Top | EdgePoint::T => UnitVec::NORTH,
        EdgePoint::South | EdgePoint::S | EdgePoint::Bottom => UnitVec::SOUTH,
        EdgePoint::East | EdgePoint::E | EdgePoint::Right => UnitVec::EAST,
        EdgePoint::West | EdgePoint::W | EdgePoint::Left => UnitVec::WEST,
        EdgePoint::NorthEast => UnitVec::NORTH_EAST,
        EdgePoint::NorthWest => UnitVec::NORTH_WEST,
        EdgePoint::SouthEast => UnitVec::SOUTH_EAST,
        EdgePoint::SouthWest => UnitVec::SOUTH_WEST,
        EdgePoint::Center | EdgePoint::C => UnitVec::ZERO,
        EdgePoint::Start => UnitVec::WEST, // Start is typically the entry direction
        EdgePoint::End => UnitVec::EAST,   // End is typically the exit direction
    }
}

/// Convert an edge point to an angle (degrees, 0 = north, clockwise)
fn edge_point_to_angle(edge: &EdgePoint) -> f64 {
    match edge {
        EdgePoint::North | EdgePoint::N | EdgePoint::Top | EdgePoint::T => 0.0,
        EdgePoint::NorthEast => 45.0,
        EdgePoint::East | EdgePoint::E | EdgePoint::Right => 90.0,
        EdgePoint::SouthEast => 135.0,
        EdgePoint::South | EdgePoint::S | EdgePoint::Bottom => 180.0,
        EdgePoint::SouthWest => 225.0,
        EdgePoint::West | EdgePoint::W | EdgePoint::Left => 270.0,
        EdgePoint::NorthWest => 315.0,
        _ => 0.0,
    }
}

fn eval_place(ctx: &RenderContext, place: &Place) -> Result<PointIn, miette::Report> {
    match place {
        Place::Object(obj) => {
            if let Some(rendered) = resolve_object(ctx, obj) {
                Ok(rendered.center)
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
                    Nth::First(_) | Nth::Ordinal(1, _, _) => Ok(rendered.start),
                    Nth::Last(_) => Ok(rendered.end),
                    _ => Ok(rendered.center),
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
                let oc = class.as_ref().and_then(|c| nth_class_to_object_class(c));
                ctx.get_last_object(oc)
            }
            Nth::First(class) => {
                let oc = class.as_ref().and_then(|c| nth_class_to_object_class(c));
                ctx.get_nth_object(1, oc)
            }
            Nth::Ordinal(n, _, class) => {
                let oc = class.as_ref().and_then(|c| nth_class_to_object_class(c));
                ctx.get_nth_object(*n as usize, oc)
            }
            Nth::Previous => {
                let len = ctx.object_list.len();
                if len > 1 {
                    ctx.object_list.get(len - 2)
                } else {
                    None
                }
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

    let next_name = &path[0];
    let remaining = &path[1..];

    // Search in children for matching name
    for child in &obj.children {
        if child.name.as_ref() == Some(next_name) {
            return resolve_path_in_object(child, remaining);
        }
    }

    None
}

fn nth_class_to_object_class(nc: &NthClass) -> Option<ObjectClass> {
    match nc {
        NthClass::ClassName(cn) => Some(match cn {
            ClassName::Box => ObjectClass::Box,
            ClassName::Circle => ObjectClass::Circle,
            ClassName::Ellipse => ObjectClass::Ellipse,
            ClassName::Oval => ObjectClass::Oval,
            ClassName::Cylinder => ObjectClass::Cylinder,
            ClassName::Diamond => ObjectClass::Diamond,
            ClassName::File => ObjectClass::File,
            ClassName::Line => ObjectClass::Line,
            ClassName::Arrow => ObjectClass::Arrow,
            ClassName::Spline => ObjectClass::Spline,
            ClassName::Arc => ObjectClass::Arc,
            ClassName::Move => ObjectClass::Move,
            ClassName::Dot => ObjectClass::Dot,
            ClassName::Text => ObjectClass::Text,
        }),
        NthClass::Sublist => Some(ObjectClass::Sublist),
    }
}

fn get_edge_point(obj: &RenderedObject, edge: &EdgePoint) -> PointIn {
    match edge {
        EdgePoint::Center | EdgePoint::C => obj.center,
        EdgePoint::Start => obj.start,
        EdgePoint::End => obj.end,
        _ => obj.edge_point(edge_point_offset(edge)),
    }
}

pub fn eval_color(rvalue: &RValue) -> String {
    // Try to extract a color name from the rvalue
    let name = match rvalue {
        RValue::PlaceName(name) => Some(name.as_str()),
        RValue::Expr(expr) => {
            // Check if expr is a simple variable reference (color name)
            extract_color_name_from_expr(expr)
        }
    };

    if let Some(name) = name {
        // Common color names (case-insensitive)
        match name.to_lowercase().as_str() {
            "red" => "red".to_string(),
            "blue" => "blue".to_string(),
            "green" => "green".to_string(),
            "yellow" => "yellow".to_string(),
            "orange" => "orange".to_string(),
            "purple" => "purple".to_string(),
            "pink" => "pink".to_string(),
            "black" => "black".to_string(),
            "white" => "white".to_string(),
            "gray" | "grey" => "gray".to_string(),
            "cyan" => "cyan".to_string(),
            "magenta" => "magenta".to_string(),
            "none" | "off" => "none".to_string(),
            _ => name.to_string(),
        }
    } else {
        "black".to_string()
    }
}

/// Try to extract a color name from an expression.
/// Returns Some if the expr is a simple variable reference that could be a color name.
fn extract_color_name_from_expr(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Variable(name) => Some(name.as_str()),
        _ => None,
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
