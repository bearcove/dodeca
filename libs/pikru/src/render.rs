//! SVG rendering for pikchr diagrams

use crate::ast::*;
use crate::types::{BoxIn, EvalValue, Length as Inches, Point, PtIn, Scaler, Size, UnitVec};
use std::collections::HashMap;
use std::fmt::Write;
use time::{OffsetDateTime, format_description};

/// Generate a UTC timestamp in YYYYMMDDhhmmss format for data-pikchr-date attribute
fn utc_timestamp() -> String {
    let now = OffsetDateTime::now_utc();
    let format = format_description::parse("[year][month][day][hour][minute][second]")
        .expect("valid format");
    now.format(&format).unwrap_or_default()
}

/// Generic numeric value that can be either a length (in inches) or a unitless scalar.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Value {
    Len(Inches),
    Scalar(f64),
}

impl Value {
    #[allow(dead_code)]
    fn as_len(self) -> Result<Inches, miette::Report> {
        match self {
            Value::Len(l) => Ok(l),
            Value::Scalar(_) => Err(miette::miette!("Expected length value, got scalar")),
        }
    }
    fn as_scalar(self) -> Result<f64, miette::Report> {
        match self {
            Value::Scalar(s) => Ok(s),
            Value::Len(_) => Err(miette::miette!("Expected scalar value, got length")),
        }
    }
}

impl From<EvalValue> for Value {
    fn from(ev: EvalValue) -> Self {
        match ev {
            EvalValue::Length(l) => Value::Len(l),
            EvalValue::Scalar(s) => Value::Scalar(s),
            // Colors are used as numeric values in expressions (24-bit RGB)
            EvalValue::Color(c) => Value::Scalar(c as f64),
        }
    }
}

impl From<Value> for EvalValue {
    fn from(v: Value) -> Self {
        match v {
            Value::Len(l) => EvalValue::Length(l),
            Value::Scalar(s) => EvalValue::Scalar(s),
        }
    }
}

/// Default sizes and settings (all in inches to mirror C implementation)
mod defaults {
    use super::Inches;

    pub const LINE_WIDTH: Inches = Inches::inches(0.5); // linewid default
    pub const BOX_WIDTH: Inches = Inches::inches(0.75);
    pub const BOX_HEIGHT: Inches = Inches::inches(0.5);
    pub const CIRCLE_RADIUS: Inches = Inches::inches(0.25);
    pub const STROKE_WIDTH: Inches = Inches::inches(0.015);
    pub const FONT_SIZE: f64 = 0.14; // approx charht (kept as f64 for text calculations)
    pub const MARGIN: f64 = 0.0; // kept as f64 for variable lookups
}

/// A point in 2D space
pub type PointIn = PtIn;

fn pin(x: f64, y: f64) -> PointIn {
    Point::new(Inches(x), Inches(y))
}

/// Bounding box
pub type BoundingBox = BoxIn;

/// Text with optional positioning and styling attributes
#[derive(Debug, Clone)]
pub struct PositionedText {
    pub value: String,
    // Positioning
    pub above: bool,
    pub below: bool,
    pub ljust: bool,
    pub rjust: bool,
    // Styling
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub big: bool,
    pub small: bool,
}

impl PositionedText {
    pub fn new(value: String) -> Self {
        Self {
            value,
            above: false,
            below: false,
            ljust: false,
            rjust: false,
            bold: false,
            italic: false,
            mono: false,
            big: false,
            small: false,
        }
    }

    pub fn from_textposition(value: String, pos: Option<&TextPosition>) -> Self {
        let mut pt = Self::new(value);
        if let Some(pos) = pos {
            for attr in &pos.attrs {
                match attr {
                    TextAttr::Above => pt.above = true,
                    TextAttr::Below => pt.below = true,
                    TextAttr::LJust => pt.ljust = true,
                    TextAttr::RJust => pt.rjust = true,
                    TextAttr::Bold => pt.bold = true,
                    TextAttr::Italic => pt.italic = true,
                    TextAttr::Mono => pt.mono = true,
                    TextAttr::Big => pt.big = true,
                    TextAttr::Small => pt.small = true,
                    _ => {}
                }
            }
        }
        pt
    }
}

/// A rendered object with its properties
#[derive(Debug, Clone)]
pub struct RenderedObject {
    pub name: Option<String>,
    pub class: ObjectClass,
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub start: PointIn,
    pub end: PointIn,
    pub start_attachment: Option<EndpointObject>,
    pub end_attachment: Option<EndpointObject>,
    /// Waypoints for multi-segment lines (includes start, intermediate points, and end)
    pub waypoints: Vec<PointIn>,
    pub text: Vec<PositionedText>,
    pub style: ObjectStyle,
    /// Child objects for sublists
    pub children: Vec<RenderedObject>,
}

#[derive(Debug, Clone)]
pub struct EndpointObject {
    pub class: ObjectClass,
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
}

impl EndpointObject {
    fn from_rendered(obj: &RenderedObject) -> Self {
        Self {
            class: obj.class,
            center: obj.center,
            width: obj.width,
            height: obj.height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObjectClass {
    Box,
    Circle,
    Ellipse,
    Oval,
    Cylinder,
    Diamond,
    File,
    Line,
    Arrow,
    Spline,
    Arc,
    Move,
    Dot,
    Text,
    Sublist,
}

#[derive(Debug, Clone)]
pub struct ObjectStyle {
    pub stroke: String,
    pub fill: String,
    pub stroke_width: Inches,
    pub dashed: bool,
    pub dotted: bool,
    pub arrow_start: bool,
    pub arrow_end: bool,
    pub invisible: bool,
    pub corner_radius: Inches,
    pub chop: bool,
    pub fit: bool,
    pub close_path: bool,
}

impl Default for ObjectStyle {
    fn default() -> Self {
        Self {
            stroke: "black".to_string(),
            fill: "none".to_string(),
            stroke_width: defaults::STROKE_WIDTH,
            dashed: false,
            dotted: false,
            arrow_start: false,
            arrow_end: false,
            invisible: false,
            corner_radius: Inches::ZERO,
            chop: false,
            fit: false,
            close_path: false,
        }
    }
}

/// Rendering context
pub struct RenderContext {
    /// Current direction
    pub direction: Direction,
    /// Current position (where the next object will be placed)
    pub position: PointIn,
    /// Named objects for reference
    pub objects: HashMap<String, RenderedObject>,
    /// All objects in order
    pub object_list: Vec<RenderedObject>,
    /// Variables (typed: lengths, scalars, colors)
    pub variables: HashMap<String, EvalValue>,
    /// Bounding box of all objects
    pub bounds: BoundingBox,
}

impl Default for RenderContext {
    fn default() -> Self {
        let mut ctx = Self {
            direction: Direction::Right,
            position: pin(0.0, 0.0),
            objects: HashMap::new(),
            object_list: Vec::new(),
            variables: HashMap::new(),
            bounds: BoundingBox::new(),
        };
        // Initialize built-in variables
        ctx.init_builtin_variables();
        ctx
    }
}

impl RenderContext {
    fn init_builtin_variables(&mut self) {
        // Built‑in length defaults mirror pikchr.c aBuiltin[] (all in inches)
        let lengths: &[(&str, f64)] = &[
            ("arcrad", 0.25),
            ("arrowht", 0.08),
            ("arrowwid", 0.06),
            ("boxht", 0.5),
            ("boxrad", 0.0),
            ("boxwid", 0.75),
            ("charht", 0.14),
            ("charwid", 0.08),
            ("circlerad", 0.25),
            ("cylht", 0.5),
            ("cylrad", 0.075),
            ("cylwid", 0.75),
            ("dashwid", 0.05),
            ("diamondht", 0.75),
            ("diamondwid", 1.0),
            ("dotrad", 0.015),
            ("ellipseht", 0.5),
            ("ellipsewid", 0.75),
            ("fileht", 0.75),
            ("filerad", 0.15),
            ("filewid", 0.5),
            ("lineht", 0.5),
            ("linewid", 0.5),
            ("movewid", 0.5),
            ("ovalht", 0.5),
            ("ovalwid", 1.0),
            ("textht", 0.5),
            ("textwid", 0.75),
            ("thickness", 0.015),
            ("margin", 0.0),
            ("leftmargin", 0.0),
            ("rightmargin", 0.0),
            ("topmargin", 0.0),
            ("bottommargin", 0.0),
            // Common aliases
            ("wid", 0.75),
            ("ht", 0.5),
            ("rad", 0.25),
        ];
        for (k, v) in lengths {
            self.variables
                .insert((*k).to_string(), EvalValue::Length(Inches(*v)));
        }

        // Unitless scalars
        let scalars: &[(&str, f64)] = &[
            ("arrowhead", 2.0), // Multiplier, not a length
            ("scale", 1.0),
        ];
        for (k, v) in scalars {
            self.variables
                .insert((*k).to_string(), EvalValue::Scalar(*v));
        }

        // Color variables (fill = -1 means "no fill", stored as special sentinel)
        // Note: fill/color are color slots, but fill=-1 signals "none"
        self.variables
            .insert("fill".to_string(), EvalValue::Scalar(-1.0)); // Sentinel for "no fill"
        self.variables
            .insert("color".to_string(), EvalValue::Color(0x000000)); // Black stroke

        // Named colors (match C's 24-bit values)
        let colors: &[(&str, u32)] = &[
            ("white", 0xffffff),
            ("black", 0x000000),
            ("red", 0xff0000),
            ("green", 0x00ff00),
            ("blue", 0x0000ff),
            ("yellow", 0xffff00),
            ("cyan", 0x00ffff),
            ("magenta", 0xff00ff),
            ("gray", 0x808080),
            ("grey", 0x808080),
            ("lightgray", 0xd3d3d3),
            ("lightgrey", 0xd3d3d3),
            ("darkgray", 0xa9a9a9),
            ("darkgrey", 0xa9a9a9),
            ("orange", 0xffa500),
            ("pink", 0xffc0cb),
            ("purple", 0x800080),
            ("bisque", 0xffe4c4),
            ("beige", 0xf5f5dc),
            ("brown", 0xa52a2a),
            ("coral", 0xff7f50),
            ("gold", 0xffd700),
            ("ivory", 0xfffff0),
            ("khaki", 0xf0e68c),
            ("lavender", 0xe6e6fa),
            ("linen", 0xfaf0e6),
            ("maroon", 0x800000),
            ("navy", 0x000080),
            ("olive", 0x808000),
            ("salmon", 0xfa8072),
            ("silver", 0xc0c0c0),
            ("tan", 0xd2b48c),
            ("teal", 0x008080),
            ("tomato", 0xff6347),
            ("turquoise", 0x40e0d0),
            ("violet", 0xee82ee),
            ("wheat", 0xf5deb3),
        ];
        for (k, v) in colors {
            self.variables
                .insert((*k).to_string(), EvalValue::Color(*v));
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Get the last rendered object
    pub fn last_object(&self) -> Option<&RenderedObject> {
        self.object_list.last()
    }

    /// Get an object by name
    pub fn get_object(&self, name: &str) -> Option<&RenderedObject> {
        self.objects.get(name)
    }

    /// Get the nth object of a class (1-indexed)
    pub fn get_nth_object(&self, n: usize, class: Option<ObjectClass>) -> Option<&RenderedObject> {
        let filtered: Vec<_> = self
            .object_list
            .iter()
            .filter(|o| class.map(|c| o.class == c).unwrap_or(true))
            .collect();
        filtered.get(n.saturating_sub(1)).copied()
    }

    /// Get the last object of a class
    pub fn get_last_object(&self, class: Option<ObjectClass>) -> Option<&RenderedObject> {
        self.object_list
            .iter()
            .rev()
            .find(|o| class.map(|c| o.class == c).unwrap_or(true))
    }

    /// Move position in the current direction
    pub fn advance(&mut self, distance: Inches) {
        match self.direction {
            Direction::Right => self.position.x += distance,
            Direction::Left => self.position.x -= distance,
            Direction::Up => self.position.y -= distance,
            Direction::Down => self.position.y += distance,
        }
    }

    /// Add an object to the context
    pub fn add_object(&mut self, obj: RenderedObject) {
        // Update bounds
        expand_object_bounds(&mut self.bounds, &obj);

        // Update position to the exit point of the object
        self.position = obj.end;

        // Store named objects
        if let Some(ref name) = obj.name {
            self.objects.insert(name.clone(), obj.clone());
        }

        self.object_list.push(obj);
    }
}

/// Expand a bounding box to include a rendered object (recursing into sublists)
fn expand_object_bounds(bounds: &mut BoundingBox, obj: &RenderedObject) {
    match obj.class {
        ObjectClass::Line | ObjectClass::Arrow | ObjectClass::Spline | ObjectClass::Arc => {
            // Include stroke thickness padding for lines
            let pad = obj.style.stroke_width / 2.0;
            for pt in &obj.waypoints {
                bounds.expand_point(Point::new(pt.x - pad, pt.y - pad));
                bounds.expand_point(Point::new(pt.x + pad, pt.y + pad));
            }
        }
        ObjectClass::Sublist => {
            for child in &obj.children {
                expand_object_bounds(bounds, child);
            }
        }
        _ => bounds.expand_rect(
            obj.center,
            Size {
                w: obj.width,
                h: obj.height,
            },
        ),
    }
}

/// Compute the bounding box of a list of rendered objects (in local coordinates)
fn compute_children_bounds(children: &[RenderedObject]) -> BoundingBox {
    let mut bounds = BoundingBox::new();
    for child in children {
        expand_object_bounds(&mut bounds, child);
    }
    bounds
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
                    PrintArg::Expr(e) => match eval_expr(ctx, e) {
                        Ok(Value::Scalar(v)) => format!("{}", v),
                        Ok(Value::Len(l)) => format!("{}", l.0),
                        Err(err) => format!("[{}]", err),
                    },
                    PrintArg::PlaceName(name) => name.clone(),
                };
                parts.push(s);
            }
            print_lines.push(parts.join(" "));
        }
        Statement::Assert(_) => {
            // Not rendered
        }
        Statement::Define(_) | Statement::MacroCall(_) => {
            // Already handled earlier
        }
    }
    Ok(())
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
                defaults::BOX_WIDTH,
                defaults::BOX_HEIGHT / 2.0,
            ),
            ClassName::Cylinder => (
                ObjectClass::Cylinder,
                defaults::BOX_WIDTH,
                defaults::BOX_HEIGHT,
            ),
            ClassName::Diamond => (
                ObjectClass::Diamond,
                defaults::BOX_WIDTH,
                defaults::BOX_HEIGHT,
            ),
            ClassName::File => (ObjectClass::File, defaults::BOX_WIDTH, defaults::BOX_HEIGHT),
            ClassName::Line => (ObjectClass::Line, defaults::LINE_WIDTH, Inches::ZERO),
            ClassName::Arrow => (ObjectClass::Arrow, defaults::LINE_WIDTH, Inches::ZERO),
            ClassName::Spline => (ObjectClass::Spline, defaults::LINE_WIDTH, Inches::ZERO),
            ClassName::Arc => (ObjectClass::Arc, defaults::LINE_WIDTH, defaults::LINE_WIDTH),
            ClassName::Move => (ObjectClass::Move, defaults::LINE_WIDTH, Inches::ZERO),
            ClassName::Dot => (ObjectClass::Dot, Inches(0.03), Inches(0.03)),
            ClassName::Text => (ObjectClass::Text, Inches::ZERO, Inches::ZERO),
        },
        BaseType::Text(s, _) => {
            // Estimate text dimensions
            let w = s.value.len() as f64 * defaults::FONT_SIZE * 0.6;
            let h = defaults::FONT_SIZE * 1.2;
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
    let mut direction_offset_x: Inches = Inches::ZERO;
    let mut direction_offset_y: Inches = Inches::ZERO;
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
                    NumProperty::Width => width = val,
                    NumProperty::Height => height = val,
                    NumProperty::Radius => {
                        // For circles/ellipses, radius sets size (diameter = 2 * radius)
                        // For boxes, radius sets corner rounding
                        match class {
                            ObjectClass::Circle | ObjectClass::Ellipse | ObjectClass::Arc => {
                                width = val * 2.0;
                                height = val * 2.0;
                            }
                            _ => {
                                style.corner_radius = val;
                            }
                        }
                    }
                    NumProperty::Diameter => {
                        width = val;
                        height = val;
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
                if let Ok(p) = eval_position(ctx, pos) {
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
                match dir {
                    Direction::Right => direction_offset_x += distance,
                    Direction::Left => direction_offset_x -= distance,
                    Direction::Up => direction_offset_y -= distance,
                    Direction::Down => direction_offset_y += distance,
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
                    // Apply in context direction
                    match ctx.direction {
                        Direction::Right => direction_offset_x += val,
                        Direction::Left => direction_offset_x -= val,
                        Direction::Up => direction_offset_y -= val,
                        Direction::Down => direction_offset_y += val,
                    }
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
                let (hw, hh) = (last_obj.width / 2.0, last_obj.height / 2.0);
                let c = last_obj.center;
                let exit_x = if direction_offset_x > Inches::ZERO {
                    c.x + hw // moving right, exit from right edge
                } else if direction_offset_x < Inches::ZERO {
                    c.x - hw // moving left, exit from left edge
                } else {
                    c.x // no horizontal movement, use center
                };
                let exit_y = if direction_offset_y > Inches::ZERO {
                    c.y + hh // moving down, exit from bottom edge
                } else if direction_offset_y < Inches::ZERO {
                    c.y - hh // moving up, exit from top edge
                } else {
                    c.y // no vertical movement, use center
                };
                Point::new(exit_x, exit_y)
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
                let next = Point::new(
                    current_pos.x + direction_offset_x,
                    current_pos.y + direction_offset_y,
                );
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
        if let Statement::Object(obj_stmt) = stmt {
            let obj = render_object_stmt(&mut ctx, obj_stmt, None)?;
            ctx.add_object(obj);
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
fn move_in_direction(pos: PointIn, dir: Direction, distance: Inches) -> PointIn {
    match dir {
        Direction::Right => Point::new(pos.x + distance, pos.y),
        Direction::Left => Point::new(pos.x - distance, pos.y),
        Direction::Up => Point::new(pos.x, pos.y - distance),
        Direction::Down => Point::new(pos.x, pos.y + distance),
    }
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
    let (start, end) = match direction {
        Direction::Right => (
            Point::new(center.x - width / 2.0, center.y),
            Point::new(center.x + width / 2.0, center.y),
        ),
        Direction::Left => (
            Point::new(center.x + width / 2.0, center.y),
            Point::new(center.x - width / 2.0, center.y),
        ),
        Direction::Up => (
            Point::new(center.x, center.y + height / 2.0),
            Point::new(center.x, center.y - height / 2.0),
        ),
        Direction::Down => (
            Point::new(center.x, center.y - height / 2.0),
            Point::new(center.x, center.y + height / 2.0),
        ),
    };
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
            let end = match ctx.direction {
                Direction::Right => Point::new(start.x + width, start.y),
                Direction::Left => Point::new(start.x - width, start.y),
                Direction::Up => Point::new(start.x, start.y - width),
                Direction::Down => Point::new(start.x, start.y + width),
            };
            let mid = start.midpoint(end);
            (start, end, mid)
        }
        _ => {
            // For shaped objects (box, circle, etc.):
            // The entry edge is placed at the current cursor, not the center.
            // This matches C pikchr behavior where objects chain edge-to-edge.
            let (half_w, half_h) = (width / 2.0, height / 2.0);
            let center = match ctx.direction {
                Direction::Right => Point::new(ctx.position.x + half_w, ctx.position.y),
                Direction::Left => Point::new(ctx.position.x - half_w, ctx.position.y),
                Direction::Up => Point::new(ctx.position.x, ctx.position.y - half_h),
                Direction::Down => Point::new(ctx.position.x, ctx.position.y + half_h),
            };
            let start = match ctx.direction {
                Direction::Right => Point::new(center.x - half_w, center.y),
                Direction::Left => Point::new(center.x + half_w, center.y),
                Direction::Up => Point::new(center.x, center.y + half_h),
                Direction::Down => Point::new(center.x, center.y - half_h),
            };
            let end = match ctx.direction {
                Direction::Right => Point::new(center.x + half_w, center.y),
                Direction::Left => Point::new(center.x - half_w, center.y),
                Direction::Up => Point::new(center.x, center.y - half_h),
                Direction::Down => Point::new(center.x, center.y + half_h),
            };
            (start, end, center)
        }
    };

    (center, start, end)
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

fn eval_expr(ctx: &RenderContext, expr: &Expr) -> Result<Value, miette::Report> {
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
                Function::Cos => Scalar(args[0].as_scalar()?.to_radians().cos()),
                Function::Sin => Scalar(args[0].as_scalar()?.to_radians().sin()),
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
                    let a = args[0].as_scalar()?;
                    let b = args[1].as_scalar()?;
                    Scalar(a.max(b))
                }
                Function::Min => {
                    let a = args[0].as_scalar()?;
                    let b = args[1].as_scalar()?;
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

fn eval_len(ctx: &RenderContext, expr: &Expr) -> Result<Inches, miette::Report> {
    match eval_expr(ctx, expr)? {
        Value::Len(l) => Ok(l),
        Value::Scalar(s) => Ok(Inches(s)), // treat scalar as inches for len contexts
    }
}

fn eval_scalar(ctx: &RenderContext, expr: &Expr) -> Result<f64, miette::Report> {
    match eval_expr(ctx, expr)? {
        Value::Scalar(s) => Ok(s),
        Value::Len(l) => Ok(l.0),
    }
}

fn eval_rvalue(ctx: &RenderContext, rvalue: &RValue) -> Result<Value, miette::Report> {
    match rvalue {
        RValue::Expr(e) => eval_expr(ctx, e),
        RValue::PlaceName(_) => Ok(Value::Scalar(0.0)), // Color names return 0
    }
}

fn eval_position(ctx: &RenderContext, pos: &Position) -> Result<PointIn, miette::Report> {
    match pos {
        Position::Coords(x, y) => {
            let px = eval_len(ctx, x)?;
            let py = eval_len(ctx, y)?;
            Ok(Point::new(px, py))
        }
        Position::Place(place) => eval_place(ctx, place),
        Position::PlaceOffset(place, op, dx, dy) => {
            let base = eval_place(ctx, place)?;
            let dx_val = eval_len(ctx, dx)?;
            let dy_val = eval_len(ctx, dy)?;
            match op {
                BinaryOp::Add => Ok(Point::new(base.x + dx_val, base.y + dy_val)),
                BinaryOp::Sub => Ok(Point::new(base.x - dx_val, base.y - dy_val)),
                _ => Ok(base),
            }
        }
        Position::Between(factor, pos1, pos2) => {
            let f = eval_scalar(ctx, factor)?;
            let p1 = eval_position(ctx, pos1)?;
            let p2 = eval_position(ctx, pos2)?;
            Ok(Point::new(
                p1.x + (p2.x - p1.x) * f,
                p1.y + (p2.y - p1.y) * f,
            ))
        }
        Position::Bracket(factor, pos1, pos2) => {
            // Same as between
            let f = eval_scalar(ctx, factor)?;
            let p1 = eval_position(ctx, pos1)?;
            let p2 = eval_position(ctx, pos2)?;
            Ok(Point::new(
                p1.x + (p2.x - p1.x) * f,
                p1.y + (p2.y - p1.y) * f,
            ))
        }
        Position::AboveBelow(dist, ab, base_pos) => {
            let d = eval_len(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            match ab {
                AboveBelow::Above => Ok(Point::new(base.x, base.y - d)),
                AboveBelow::Below => Ok(Point::new(base.x, base.y + d)),
            }
        }
        Position::LeftRightOf(dist, lr, base_pos) => {
            let d = eval_len(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            match lr {
                LeftRight::Left => Ok(Point::new(base.x - d, base.y)),
                LeftRight::Right => Ok(Point::new(base.x + d, base.y)),
            }
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
            let rad = (90.0 - angle).to_radians();
            Ok(Point::new(
                base.x + Inches(d.0 * rad.cos()),
                base.y - Inches(d.0 * rad.sin()),
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

fn endpoint_object_from_position(ctx: &RenderContext, pos: &Position) -> Option<EndpointObject> {
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
fn edge_point_offset(edge: &EdgePoint) -> UnitVec {
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

fn resolve_object<'a>(ctx: &'a RenderContext, obj: &Object) -> Option<&'a RenderedObject> {
    match obj {
        Object::Named(name) => {
            // First resolve the base object
            let base_obj = match &name.base {
                ObjectNameBase::PlaceName(n) => ctx.get_object(n),
                ObjectNameBase::This => ctx.last_object(),
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
    let cx = obj.center.x.0;
    let cy = obj.center.y.0;
    let hw = obj.width.0 / 2.0;
    let hh = obj.height.0 / 2.0;

    // For circles/ellipses, diagonal edge points (ne, nw, se, sw) use the actual
    // point on the perimeter at 45 degrees, not the bounding box corner.
    // The diagonal factor is 1/sqrt(2) ≈ 0.707
    let is_round = matches!(
        obj.class,
        ObjectClass::Circle | ObjectClass::Ellipse | ObjectClass::Oval
    );
    let diag = if is_round {
        std::f64::consts::FRAC_1_SQRT_2
    } else {
        1.0
    };

    match edge {
        EdgePoint::North | EdgePoint::N | EdgePoint::Top | EdgePoint::T => {
            Point::new(Inches(cx), Inches(cy - hh))
        }
        EdgePoint::South | EdgePoint::S | EdgePoint::Bottom => {
            Point::new(Inches(cx), Inches(cy + hh))
        }
        EdgePoint::East | EdgePoint::E | EdgePoint::Right => {
            Point::new(Inches(cx + hw), Inches(cy))
        }
        EdgePoint::West | EdgePoint::W | EdgePoint::Left => Point::new(Inches(cx - hw), Inches(cy)),
        EdgePoint::NorthEast => Point::new(Inches(cx + hw * diag), Inches(cy - hh * diag)),
        EdgePoint::NorthWest => Point::new(Inches(cx - hw * diag), Inches(cy - hh * diag)),
        EdgePoint::SouthEast => Point::new(Inches(cx + hw * diag), Inches(cy + hh * diag)),
        EdgePoint::SouthWest => Point::new(Inches(cx - hw * diag), Inches(cy + hh * diag)),
        EdgePoint::Center | EdgePoint::C => obj.center,
        EdgePoint::Start => obj.start,
        EdgePoint::End => obj.end,
    }
}

fn eval_color(rvalue: &RValue) -> String {
    match rvalue {
        RValue::PlaceName(name) => {
            // Common color names
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
                _ => name.clone(),
            }
        }
        RValue::Expr(_) => "black".to_string(),
    }
}

/// Helper to extract a length from an EvalValue, with fallback
fn get_length(ctx: &RenderContext, name: &str, default: f64) -> f64 {
    ctx.variables
        .get(name)
        .and_then(|v| v.as_length())
        .map(|l| l.raw())
        .unwrap_or(default)
}

/// Helper to extract a scalar from an EvalValue, with fallback
fn get_scalar(ctx: &RenderContext, name: &str, default: f64) -> f64 {
    ctx.variables
        .get(name)
        .map(|v| v.as_scalar())
        .unwrap_or(default)
}

fn generate_svg(ctx: &RenderContext) -> Result<String, miette::Report> {
    let margin_base = get_length(ctx, "margin", defaults::MARGIN);
    let left_margin = get_length(ctx, "leftmargin", 0.0);
    let right_margin = get_length(ctx, "rightmargin", 0.0);
    let top_margin = get_length(ctx, "topmargin", 0.0);
    let bottom_margin = get_length(ctx, "bottommargin", 0.0);
    let thickness = get_length(ctx, "thickness", defaults::STROKE_WIDTH.raw());

    let margin = margin_base + thickness;
    let r_scale = 144.0; // match pikchr.c rScale
    let scale = get_scalar(ctx, "scale", 1.0);
    let eff_scale = r_scale * scale;
    let scaler = Scaler::try_new(eff_scale)
        .map_err(|e| miette::miette!("invalid scale value {}: {}", eff_scale, e))?;
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

    let mut svg = String::new();

    // SVG header (add class and pixel dimensions like C)
    let width_px = scaler.px(view_width);
    let height_px = scaler.px(view_height);
    let timestamp = utc_timestamp();
    writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" style="font-size:initial;" class="pikchr" width="{:.0}" height="{:.0}" viewBox="0 0 {:.2} {:.2}" data-pikchr-date="{}">"#,
        width_px,
        height_px,
        width_px,
        height_px,
        timestamp
    )
    .unwrap();

    // Arrowheads are now rendered inline as polygon elements (matching C pikchr)

    // Render each object
    for obj in &ctx.object_list {
        if obj.style.invisible {
            continue;
        }

        let tx = scaler.px(obj.center.x + offset_x);
        let ty = scaler.px(obj.center.y + offset_y);
        let sx = scaler.px(obj.start.x + offset_x);
        let sy = scaler.px(obj.start.y + offset_y);
        let ex = scaler.px(obj.end.x + offset_x);
        let ey = scaler.px(obj.end.y + offset_y);

        let stroke_style = format_stroke_style(&obj.style, &scaler, dashwid);

        match obj.class {
            ObjectClass::Box => {
                // Use <path> like C pikchr for consistency
                let x1 = tx - scaler.px(obj.width / 2.0);
                let y1 = ty - scaler.px(obj.height / 2.0);
                let x2 = tx + scaler.px(obj.width / 2.0);
                let y2 = ty + scaler.px(obj.height / 2.0);
                if obj.style.corner_radius.0 > 0.0 {
                    // Rounded corners - keep using rect for now (C pikchr also uses path but complex)
                    writeln!(svg, r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="{:.2}" ry="{:.2}" {}/>"#,
                             x1, y1, scaler.px(obj.width), scaler.px(obj.height), scaler.px(obj.style.corner_radius), scaler.px(obj.style.corner_radius), stroke_style).unwrap();
                } else {
                    // C pikchr renders boxes as path: M x1,y2 L x2,y2 L x2,y1 L x1,y1 Z
                    // (starts at bottom-left, goes clockwise)
                    writeln!(
                        svg,
                        r#"  <path d="M{:.2},{:.2}L{:.2},{:.2}L{:.2},{:.2}L{:.2},{:.2}Z" {}/>"#,
                        x1, y2, x2, y2, x2, y1, x1, y1, stroke_style
                    )
                    .unwrap();
                }
            }
            ObjectClass::Circle => {
                let r = scaler.px(obj.width / 2.0);
                writeln!(
                    svg,
                    r#"  <circle cx="{:.2}" cy="{:.2}" r="{:.2}" {}/>"#,
                    tx, ty, r, stroke_style
                )
                .unwrap();
            }
            ObjectClass::Dot => {
                // Dot is a small filled circle
                let r = scaler.px(obj.width / 2.0);
                let fill = if obj.style.fill == "none" {
                    &obj.style.stroke
                } else {
                    &obj.style.fill
                };
                writeln!(
                    svg,
                    r#"  <circle cx="{:.2}" cy="{:.2}" r="{:.2}" fill="{}" stroke="none"/>"#,
                    tx, ty, r, fill
                )
                .unwrap();
            }
            ObjectClass::Ellipse => {
                let rx = scaler.px(obj.width / 2.0);
                let ry = scaler.px(obj.height / 2.0);
                writeln!(
                    svg,
                    r#"  <ellipse cx="{:.2}" cy="{:.2}" rx="{:.2}" ry="{:.2}" {}/>"#,
                    tx, ty, rx, ry, stroke_style
                )
                .unwrap();
            }
            ObjectClass::Oval => {
                // Oval is a pill shape (rounded rectangle with fully rounded ends)
                render_oval(
                    &mut svg,
                    tx,
                    ty,
                    obj.width,
                    obj.height,
                    &scaler,
                    &stroke_style,
                );
            }
            ObjectClass::Cylinder => {
                // Cylinder: elliptical top/bottom with vertical sides
                render_cylinder(
                    &mut svg,
                    tx,
                    ty,
                    obj.width,
                    obj.height,
                    &scaler,
                    &stroke_style,
                    &obj.style,
                );
            }
            ObjectClass::File => {
                // File: document shape with folded corner
                render_file(
                    &mut svg,
                    tx,
                    ty,
                    obj.width,
                    obj.height,
                    &scaler,
                    &stroke_style,
                );
            }
            ObjectClass::Line | ObjectClass::Arrow => {
                let (auto_sx, auto_sy, auto_ex, auto_ey) = if obj.waypoints.len() <= 2 {
                    apply_auto_chop_simple_line(&scaler, obj, sx, sy, ex, ey, offset_x, offset_y)
                } else {
                    (sx, sy, ex, ey)
                };
                // Apply chop if needed (shorten line from both ends)
                let chop_amount_px = if obj.style.chop {
                    scaler.px(defaults::CIRCLE_RADIUS)
                } else {
                    0.0
                };
                let (draw_sx, draw_sy, draw_ex, draw_ey) = if chop_amount_px > 0.0 {
                    chop_line(auto_sx, auto_sy, auto_ex, auto_ey, chop_amount_px)
                } else {
                    (auto_sx, auto_sy, auto_ex, auto_ey)
                };

                if obj.waypoints.len() <= 2 {
                    // Simple line - render as <path> (matching C pikchr)
                    // First render arrowhead polygon if needed (rendered before line, like C)
                    if obj.style.arrow_end {
                        render_arrowhead(
                            &mut svg,
                            draw_sx,
                            draw_sy,
                            draw_ex,
                            draw_ey,
                            &obj.style,
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        );
                    }
                    if obj.style.arrow_start {
                        render_arrowhead_start(
                            &mut svg,
                            draw_sx,
                            draw_sy,
                            draw_ex,
                            draw_ey,
                            &obj.style,
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        );
                    }

                    // Chop line endpoints for arrowheads (by arrowht/2 as in C pikchr, in pixels)
                    let arrow_chop_px = arrow_len_px.0 / 2.0;
                    let (line_sx, line_sy, line_ex, line_ey) = {
                        let mut sx = draw_sx;
                        let mut sy = draw_sy;
                        let mut ex = draw_ex;
                        let mut ey = draw_ey;

                        if obj.style.arrow_start {
                            let (new_sx, new_sy, _, _) = chop_line(sx, sy, ex, ey, arrow_chop_px);
                            sx = new_sx;
                            sy = new_sy;
                        }
                        if obj.style.arrow_end {
                            let (_, _, new_ex, new_ey) = chop_line(sx, sy, ex, ey, arrow_chop_px);
                            ex = new_ex;
                            ey = new_ey;
                        }
                        (sx, sy, ex, ey)
                    };

                    // Render the line path (with chopped endpoints)
                    writeln!(
                        svg,
                        r#"  <path d="M{:.2},{:.2}L{:.2},{:.2}" {}/>"#,
                        line_sx, line_sy, line_ex, line_ey, stroke_style
                    )
                    .unwrap();
                } else {
                    // Multi-segment polyline - chop first and last segments
                    let mut points = obj.waypoints.clone();
                    if obj.style.chop && points.len() >= 2 {
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
                        // Build path string (no arrow chopping for closed paths)
                        let path_str: String = points
                            .iter()
                            .enumerate()
                            .map(|(i, p)| {
                                let cmd = if i == 0 { "M" } else { "L" };
                                format!(
                                    "{}{:.2},{:.2}",
                                    cmd,
                                    scaler.px(p.x + offset_x),
                                    scaler.px(p.y + offset_y)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        // Closed polygon - add Z to close path
                        writeln!(svg, r#"  <path d="{}Z" {}/>"#, path_str, stroke_style).unwrap();
                    } else {
                        // Render arrowheads first (before chopping line for path)
                        let n = points.len();
                        if obj.style.arrow_end && n >= 2 {
                            let p1 = points[n - 2];
                            let p2 = points[n - 1];
                            render_arrowhead(
                                &mut svg,
                                scaler.px(p1.x + offset_x),
                                scaler.px(p1.y + offset_y),
                                scaler.px(p2.x + offset_x),
                                scaler.px(p2.y + offset_y),
                                &obj.style,
                                arrow_len_px.0,
                                arrow_wid_px.0,
                            );
                        }
                        if obj.style.arrow_start && n >= 2 {
                            let p1 = points[0];
                            let p2 = points[1];
                            render_arrowhead_start(
                                &mut svg,
                                scaler.px(p1.x + offset_x),
                                scaler.px(p1.y + offset_y),
                                scaler.px(p2.x + offset_x),
                                scaler.px(p2.y + offset_y),
                                &obj.style,
                                arrow_len_px.0,
                                arrow_wid_px.0,
                            );
                        }

                        // Chop line endpoints for arrowheads (by arrowht/2 as in C pikchr)
                        let arrow_chop = arrow_ht / 2.0;
                        if obj.style.arrow_start && n >= 2 {
                            let p0 = points[0];
                            let p1 = points[1];
                            let (new_x, new_y, _, _) =
                                chop_line(p0.x.0, p0.y.0, p1.x.0, p1.y.0, arrow_chop.0);
                            points[0] = Point::new(Inches(new_x), Inches(new_y));
                        }
                        if obj.style.arrow_end && n >= 2 {
                            let pn1 = points[n - 2];
                            let pn = points[n - 1];
                            let (_, _, new_x, new_y) =
                                chop_line(pn1.x.0, pn1.y.0, pn.x.0, pn.y.0, arrow_chop.0);
                            points[n - 1] = Point::new(Inches(new_x), Inches(new_y));
                        }

                        // Build path string with chopped endpoints
                        let path_str: String = points
                            .iter()
                            .enumerate()
                            .map(|(i, p)| {
                                let cmd = if i == 0 { "M" } else { "L" };
                                format!(
                                    "{}{:.2},{:.2}",
                                    cmd,
                                    scaler.px(p.x + offset_x),
                                    scaler.px(p.y + offset_y)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        writeln!(svg, r#"  <path d="{}" {}/>"#, path_str, stroke_style).unwrap();
                    }
                }
            }
            ObjectClass::Spline => {
                if obj.waypoints.len() <= 2 {
                    // Simple line for spline with only 2 points
                    if obj.style.arrow_end {
                        render_arrowhead(
                            &mut svg,
                            sx,
                            sy,
                            ex,
                            ey,
                            &obj.style,
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        );
                    }
                    writeln!(
                        svg,
                        r#"  <path d="M{:.2},{:.2}L{:.2},{:.2}" {}/>"#,
                        sx, sy, ex, ey, stroke_style
                    )
                    .unwrap();
                    if obj.style.arrow_start {
                        render_arrowhead_start(
                            &mut svg,
                            sx,
                            sy,
                            ex,
                            ey,
                            &obj.style,
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        );
                    }
                } else {
                    // Multi-segment spline - use a smooth path with curves
                    render_spline_path(
                        &mut svg,
                        &obj.waypoints,
                        offset_x,
                        offset_y,
                        &stroke_style,
                        &obj.style,
                        arrow_len_px.0,
                        arrow_wid_px.0,
                    );
                }
            }
            ObjectClass::Arc => {
                // Arc: quarter circle arc
                render_arc(
                    &mut svg,
                    sx,
                    sy,
                    ex,
                    ey,
                    obj.width.0,
                    &obj.style,
                    &stroke_style,
                    arrow_len_px.0,
                    arrow_wid_px.0,
                );
            }
            ObjectClass::Diamond => {
                let points = format!(
                    "{:.2},{:.2} {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
                    tx,
                    ty - obj.height.0 / 2.0, // top
                    tx + obj.width.0 / 2.0,
                    ty, // right
                    tx,
                    ty + obj.height.0 / 2.0, // bottom
                    tx - obj.width.0 / 2.0,
                    ty // left
                );
                writeln!(svg, r#"  <polygon points="{}" {}/>"#, points, stroke_style).unwrap();
            }
            ObjectClass::Text => {
                render_positioned_text(
                    &mut svg,
                    &obj.text,
                    tx,
                    ty,
                    scaler.px(obj.width),
                    scaler.px(obj.height),
                    &scaler,
                );
            }
            ObjectClass::Move => {
                // Move is invisible
            }
            ObjectClass::Sublist => {
                // Render sublist children with offset
                render_sublist_children(
                    &mut svg,
                    &obj.children,
                    offset_x,
                    offset_y,
                    &scaler,
                    dashwid,
                );
            }
        }

        // Render text labels inside objects
        if obj.class != ObjectClass::Text && !obj.text.is_empty() {
            render_positioned_text(
                &mut svg,
                &obj.text,
                tx,
                ty,
                scaler.px(obj.width),
                scaler.px(obj.height),
                &scaler,
            );
        }
    }

    writeln!(svg, "</svg>").unwrap();

    Ok(svg)
}

/// Render positioned text labels
fn render_positioned_text(
    svg: &mut String,
    texts: &[PositionedText],
    cx: f64,
    cy: f64,
    width: f64,
    height: f64,
    scaler: &Scaler,
) {
    // Group texts by their vertical position (above, below, or center)
    let mut above_texts: Vec<&PositionedText> = Vec::new();
    let mut center_texts: Vec<&PositionedText> = Vec::new();
    let mut below_texts: Vec<&PositionedText> = Vec::new();

    for text in texts {
        if text.above {
            above_texts.push(text);
        } else if text.below {
            below_texts.push(text);
        } else {
            center_texts.push(text);
        }
    }

    // Render above texts (above the shape)
    for (i, text) in above_texts.iter().enumerate() {
        let font_size = get_font_size(text, scaler);
        let text_y = cy - height / 2.0 - font_size * (above_texts.len() - i) as f64;
        let (text_x, anchor) = get_text_anchor(text, cx, width);
        render_styled_text(svg, text, text_x, text_y, anchor, font_size);
    }

    // Render center texts (inside the shape)
    for (i, text) in center_texts.iter().enumerate() {
        let font_size = get_font_size(text, scaler);
        let text_y = cy + (i as f64 - center_texts.len() as f64 / 2.0 + 0.5) * font_size;
        let (text_x, anchor) = get_text_anchor(text, cx, width);
        render_styled_text(svg, text, text_x, text_y, anchor, font_size);
    }

    // Render below texts (below the shape)
    for (i, text) in below_texts.iter().enumerate() {
        let font_size = get_font_size(text, scaler);
        let text_y = cy + height / 2.0 + font_size * (i + 1) as f64;
        let (text_x, anchor) = get_text_anchor(text, cx, width);
        render_styled_text(svg, text, text_x, text_y, anchor, font_size);
    }
}

/// Get font size based on text style (big/small)
fn get_font_size(text: &PositionedText, scaler: &Scaler) -> f64 {
    let base = scaler.px(Inches(defaults::FONT_SIZE));
    if text.big {
        base * 1.4
    } else if text.small {
        base * 0.7
    } else {
        base
    }
}

/// Render a single styled text element
fn render_styled_text(
    svg: &mut String,
    text: &PositionedText,
    x: f64,
    y: f64,
    anchor: &str,
    _font_size: f64,
) {
    let mut style_parts: Vec<String> = Vec::new();

    // Font family
    if text.mono {
        style_parts.push("font-family=\"monospace\"".to_string());
    }

    // Font weight
    if text.bold {
        style_parts.push("font-weight=\"bold\"".to_string());
    }

    // Font style
    if text.italic {
        style_parts.push("font-style=\"italic\"".to_string());
    }

    let style_str = if style_parts.is_empty() {
        String::new()
    } else {
        format!(" {}", style_parts.join(" "))
    };

    writeln!(svg, r#"  <text x="{:.2}" y="{:.2}" text-anchor="{}" fill="rgb(0,0,0)" dominant-baseline="central"{}>{}</text>"#,
             x, y, anchor, style_str, escape_xml(&text.value)).unwrap();
}

/// Get text x position and anchor based on justification
fn get_text_anchor(text: &PositionedText, cx: f64, width: f64) -> (f64, &'static str) {
    if text.ljust {
        (cx - width / 2.0 + 5.0, "start") // Small padding from left edge
    } else if text.rjust {
        (cx + width / 2.0 - 5.0, "end") // Small padding from right edge
    } else {
        (cx, "middle")
    }
}

/// Render sublist children that are already in world coordinates; only apply global offset/margins.
fn render_sublist_children(
    svg: &mut String,
    children: &[RenderedObject],
    offx: Inches,
    offy: Inches,
    scaler: &Scaler,
    dashwid: Inches,
) {
    for child in children {
        let tx = scaler.px(child.center.x + offx);
        let ty = scaler.px(child.center.y + offy);
        let sx = scaler.px(child.start.x + offx);
        let sy = scaler.px(child.start.y + offy);
        let ex = scaler.px(child.end.x + offx);
        let ey = scaler.px(child.end.y + offy);

        // Sublist children inherit dash settings from caller
        let stroke_style = format_stroke_style(&child.style, scaler, dashwid);

        match child.class {
            ObjectClass::Box => {
                let x1 = tx - scaler.px(child.width / 2.0);
                let y1 = ty - scaler.px(child.height / 2.0);
                let x2 = tx + scaler.px(child.width / 2.0);
                let y2 = ty + scaler.px(child.height / 2.0);
                if child.style.corner_radius.0 > 0.0 {
                    let r = scaler.px(child.style.corner_radius);
                    writeln!(svg, r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="{:.2}" ry="{:.2}" {}/>"#,
                             x1, y1, scaler.px(child.width), scaler.px(child.height), r, r, stroke_style).unwrap();
                } else {
                    // C pikchr renders boxes as path
                    writeln!(
                        svg,
                        r#"  <path d="M{:.2},{:.2}L{:.2},{:.2}L{:.2},{:.2}L{:.2},{:.2}Z" {}/>"#,
                        x1, y2, x2, y2, x2, y1, x1, y1, stroke_style
                    )
                    .unwrap();
                }
            }
            ObjectClass::Circle => {
                let r = scaler.px(child.width / 2.0);
                writeln!(
                    svg,
                    r#"  <circle cx="{:.2}" cy="{:.2}" r="{:.2}" {}/>"#,
                    tx, ty, r, stroke_style
                )
                .unwrap();
            }
            ObjectClass::Line | ObjectClass::Arrow => {
                let marker_end = if child.style.arrow_end {
                    r#" marker-end="url(#arrowhead)""#
                } else {
                    ""
                };
                let marker_start = if child.style.arrow_start {
                    r#" marker-start="url(#arrowhead-start)""#
                } else {
                    ""
                };
                writeln!(
                    svg,
                    r#"  <line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" {}{}{}/>"#,
                    sx, sy, ex, ey, stroke_style, marker_end, marker_start
                )
                .unwrap();
            }
            _ => {
                // Other shapes can be added as needed
            }
        }

        // Render text for child
        if !child.text.is_empty() {
            render_positioned_text(
                svg,
                &child.text,
                tx,
                ty,
                scaler.px(child.width),
                scaler.px(child.height),
                scaler,
            );
        }

        // Recursively render nested sublists
        if !child.children.is_empty() {
            render_sublist_children(svg, &child.children, offx, offy, scaler, dashwid);
        }
    }
}

/// Shorten a line by `amount` from both ends
/// Returns (new_x1, new_y1, new_x2, new_y2)
fn chop_line(x1: f64, y1: f64, x2: f64, y2: f64, amount: f64) -> (f64, f64, f64, f64) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();

    if len < amount * 2.0 {
        // Line is too short to chop, return midpoint for both
        let mx = (x1 + x2) / 2.0;
        let my = (y1 + y2) / 2.0;
        return (mx, my, mx, my);
    }

    // Unit vector along the line
    let ux = dx / len;
    let uy = dy / len;

    // New endpoints
    let new_x1 = x1 + ux * amount;
    let new_y1 = y1 + uy * amount;
    let new_x2 = x2 - ux * amount;
    let new_y2 = y2 - uy * amount;

    (new_x1, new_y1, new_x2, new_y2)
}

fn apply_auto_chop_simple_line(
    scaler: &Scaler,
    obj: &RenderedObject,
    sx: f64,
    sy: f64,
    ex: f64,
    ey: f64,
    offset_x: Inches,
    offset_y: Inches,
) -> (f64, f64, f64, f64) {
    if obj.start_attachment.is_none() && obj.end_attachment.is_none() {
        return (sx, sy, ex, ey);
    }

    // Convert attachment centers to pixels with offset applied
    let end_center_px = obj
        .end_attachment
        .as_ref()
        .map(|info| {
            let px = scaler.px(info.center.x + offset_x);
            let py = scaler.px(info.center.y + offset_y);
            (px, py)
        })
        .unwrap_or((ex, ey));

    let start_center_px = obj
        .start_attachment
        .as_ref()
        .map(|info| {
            let px = scaler.px(info.center.x + offset_x);
            let py = scaler.px(info.center.y + offset_y);
            (px, py)
        })
        .unwrap_or((sx, sy));

    let mut new_start = (sx, sy);
    if let Some(ref start_info) = obj.start_attachment {
        // Chop against start object, toward the end object's center
        if let Some(chopped) =
            chop_against_endpoint(scaler, start_info, end_center_px, offset_x, offset_y)
        {
            new_start = chopped;
        }
    }

    let mut new_end = (ex, ey);
    if let Some(ref end_info) = obj.end_attachment {
        // Chop against end object, toward the start object's center
        if let Some(chopped) =
            chop_against_endpoint(scaler, end_info, start_center_px, offset_x, offset_y)
        {
            new_end = chopped;
        }
    }

    (new_start.0, new_start.1, new_end.0, new_end.1)
}

fn chop_against_endpoint(
    scaler: &Scaler,
    endpoint: &EndpointObject,
    toward: (f64, f64),
    offset_x: Inches,
    offset_y: Inches,
) -> Option<(f64, f64)> {
    match endpoint.class {
        ObjectClass::Circle | ObjectClass::Ellipse | ObjectClass::Oval => {
            let cx = scaler.px(endpoint.center.x + offset_x);
            let cy = scaler.px(endpoint.center.y + offset_y);
            let rx = scaler.px(endpoint.width / 2.0);
            let ry = scaler.px(endpoint.height / 2.0);
            chop_against_ellipse(cx, cy, rx, ry, toward)
        }
        _ => None,
    }
}

fn chop_against_ellipse(
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    toward: (f64, f64),
) -> Option<(f64, f64)> {
    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }

    let dx = toward.0 - cx;
    let dy = toward.1 - cy;
    if dx.abs() < f64::EPSILON && dy.abs() < f64::EPSILON {
        return None;
    }

    let denom = (dx * dx) / (rx * rx) + (dy * dy) / (ry * ry);
    if denom <= 0.0 {
        return None;
    }

    let scale = 1.0 / denom.sqrt();
    Some((cx + dx * scale, cy + dy * scale))
}

/// Render an oval (pill shape)
fn render_oval(
    svg: &mut String,
    cx: f64,
    cy: f64,
    width_in: Inches,
    height_in: Inches,
    scaler: &Scaler,
    stroke_style: &str,
) {
    // Oval is a rounded rectangle where the radius is half the smaller dimension (still in inches until scaled)
    let width = scaler.px(width_in);
    let height = scaler.px(height_in);
    let radius = height.min(width) / 2.0;
    let x = cx - width / 2.0;
    let y = cy - height / 2.0;
    writeln!(
        svg,
        r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="{:.2}" ry="{:.2}" {}/>"#,
        x, y, width, height, radius, radius, stroke_style
    )
    .unwrap();
}

/// Render a cylinder shape
fn render_cylinder(
    svg: &mut String,
    cx: f64,
    cy: f64,
    width_in: Inches,
    height_in: Inches,
    scaler: &Scaler,
    _stroke_style: &str,
    style: &ObjectStyle,
) {
    // Cylinder has elliptical top and bottom
    // The ellipse height is about 1/4 of the total width for a nice 3D effect
    let width = scaler.px(width_in);
    let height = scaler.px(height_in);
    let rx = width / 2.0;
    let ry = width / 8.0; // Ellipse vertical radius

    let top_y = cy - height / 2.0 + ry;
    let bottom_y = cy + height / 2.0 - ry;

    // Draw the body (two vertical lines + bottom ellipse arc)
    // Path: left side down, bottom arc, right side up
    let path = format!(
        "M {:.2},{:.2} L {:.2},{:.2} A {:.2},{:.2} 0 0,0 {:.2},{:.2} L {:.2},{:.2}",
        cx - rx,
        top_y, // Start at top-left
        cx - rx,
        bottom_y, // Line down to bottom-left
        rx,
        ry, // Arc radii
        cx + rx,
        bottom_y, // Arc to bottom-right
        cx + rx,
        top_y // Line up to top-right
    );

    // Use fill for the body if specified
    let body_fill = if style.fill != "none" {
        &style.fill
    } else {
        "none"
    };
    writeln!(
        svg,
        r#"  <path d="{}" stroke="{}" fill="{}" stroke-width="{:.2}"/>"#,
        path, style.stroke, body_fill, style.stroke_width.0
    )
    .unwrap();

    // Draw the top ellipse (full ellipse, filled)
    writeln!(svg, r#"  <ellipse cx="{:.2}" cy="{:.2}" rx="{:.2}" ry="{:.2}" stroke="{}" fill="{}" stroke-width="{:.2}"/>"#,
             cx, top_y, rx, ry, style.stroke, body_fill, style.stroke_width.0).unwrap();

    // Draw the bottom ellipse arc (only the front half, as a visible edge)
    let bottom_arc = format!(
        "M {:.2},{:.2} A {:.2},{:.2} 0 0,0 {:.2},{:.2}",
        cx - rx,
        bottom_y,
        rx,
        ry,
        cx + rx,
        bottom_y
    );
    writeln!(
        svg,
        r#"  <path d="{}" stroke="{}" fill="none" stroke-width="{:.2}"/>"#,
        bottom_arc, style.stroke, style.stroke_width.0
    )
    .unwrap();
}

/// Render a file shape (document with folded corner)
fn render_file(
    svg: &mut String,
    cx: f64,
    cy: f64,
    width_in: Inches,
    height_in: Inches,
    scaler: &Scaler,
    stroke_style: &str,
) {
    // File shape: rectangle with top-right corner folded
    let width = scaler.px(width_in);
    let height = scaler.px(height_in);
    let fold_size = width.min(height) * 0.2; // Fold is 20% of smaller dimension

    let x = cx - width / 2.0;
    let y = cy - height / 2.0;
    let right = cx + width / 2.0;
    let bottom = cy + height / 2.0;

    // Main outline path (going clockwise from top-left)
    // Top-left -> top-right minus fold -> fold corner -> bottom-right -> bottom-left -> close
    let path = format!(
        "M {:.2},{:.2} L {:.2},{:.2} L {:.2},{:.2} L {:.2},{:.2} L {:.2},{:.2} Z",
        x,
        y, // Top-left
        right - fold_size,
        y, // Top-right minus fold
        right,
        y + fold_size, // Fold corner (diagonal)
        right,
        bottom, // Bottom-right
        x,
        bottom // Bottom-left
    );
    writeln!(svg, r#"  <path d="{}" {}/>"#, path, stroke_style).unwrap();

    // Draw the fold line (the crease)
    let fold_path = format!(
        "M {:.2},{:.2} L {:.2},{:.2} L {:.2},{:.2}",
        right - fold_size,
        y, // Start at corner
        right - fold_size,
        y + fold_size, // Down to fold
        right,
        y + fold_size // Across to edge
    );
    writeln!(
        svg,
        r#"  <path d="{}" stroke="black" fill="none" stroke-width="1"/>"#,
        fold_path
    )
    .unwrap();
}

/// Render a spline path (smooth bezier curves through waypoints)
fn render_spline_path(
    svg: &mut String,
    waypoints: &[PointIn],
    offset_x: Inches,
    offset_y: Inches,
    stroke_style: &str,
    style: &ObjectStyle,
    arrow_len: f64,
    arrow_width: f64,
) {
    if waypoints.is_empty() {
        return;
    }

    // Convert waypoints to offset coordinates
    let points: Vec<(f64, f64)> = waypoints
        .iter()
        .map(|p| (p.x.0 + offset_x.0, p.y.0 + offset_y.0))
        .collect();

    // Render arrowhead at end if needed
    let n = points.len();
    if style.arrow_end && n >= 2 {
        let p1 = points[n - 2];
        let p2 = points[n - 1];
        render_arrowhead(svg, p1.0, p1.1, p2.0, p2.1, style, arrow_len, arrow_width);
    }

    // Build a smooth bezier path through the points
    let mut path = format!("M {:.2},{:.2}", points[0].0, points[0].1);

    if points.len() == 2 {
        // Just a line
        path.push_str(&format!(" L {:.2},{:.2}", points[1].0, points[1].1));
    } else {
        // Use quadratic bezier curves for smoothness
        // For each segment, use the midpoint as control point
        for i in 1..points.len() {
            let prev = points[i - 1];
            let curr = points[i];

            if i == 1 {
                // First segment: quadratic from start
                let mid = ((prev.0 + curr.0) / 2.0, (prev.1 + curr.1) / 2.0);
                path.push_str(&format!(
                    " Q {:.2},{:.2} {:.2},{:.2}",
                    prev.0, prev.1, mid.0, mid.1
                ));
            }

            if i < points.len() - 1 {
                // Middle segments: curve through midpoints
                let next = points[i + 1];
                let mid = ((curr.0 + next.0) / 2.0, (curr.1 + next.1) / 2.0);
                path.push_str(&format!(
                    " Q {:.2},{:.2} {:.2},{:.2}",
                    curr.0, curr.1, mid.0, mid.1
                ));
            } else {
                // Last segment: end at the final point
                path.push_str(&format!(
                    " Q {:.2},{:.2} {:.2},{:.2}",
                    (prev.0 + curr.0) / 2.0,
                    (prev.1 + curr.1) / 2.0,
                    curr.0,
                    curr.1
                ));
            }
        }
    }

    writeln!(svg, r#"  <path d="{}" {}/>"#, path, stroke_style).unwrap();

    // Render arrowhead at start if needed
    if style.arrow_start && n >= 2 {
        let p1 = points[0];
        let p2 = points[1];
        render_arrowhead_start(svg, p1.0, p1.1, p2.0, p2.1, style, arrow_len, arrow_width);
    }
}

/// Render an arc
fn render_arc(
    svg: &mut String,
    sx: f64,
    sy: f64,
    ex: f64,
    ey: f64,
    radius: f64,
    style: &ObjectStyle,
    stroke_style: &str,
    arrow_len: f64,
    arrow_width: f64,
) {
    // Determine arc direction and sweep
    let dx = ex - sx;
    let dy = ey - sy;

    // Default to quarter-circle arc
    let r = if radius > 0.0 {
        radius / 2.0
    } else {
        (dx.abs() + dy.abs()) / 2.0
    };

    // sweep-flag: 0 = counter-clockwise, 1 = clockwise
    // large-arc-flag: 0 = small arc, 1 = large arc
    let sweep = 1; // Default clockwise
    let large_arc = 0;

    // Render arrowheads as inline polygons
    if style.arrow_end {
        render_arrowhead(svg, sx, sy, ex, ey, style, arrow_len, arrow_width);
    }

    let path = format!(
        "M {:.2},{:.2} A {:.2},{:.2} 0 {} {} {:.2},{:.2}",
        sx, sy, r, r, large_arc, sweep, ex, ey
    );
    writeln!(svg, r#"  <path d="{}" {}/>"#, path, stroke_style).unwrap();

    if style.arrow_start {
        render_arrowhead_start(svg, sx, sy, ex, ey, style, arrow_len, arrow_width);
    }
}

fn format_stroke_style(style: &ObjectStyle, scaler: &Scaler, dashwid: Inches) -> String {
    let mut parts = Vec::new();

    parts.push(format!(r#"stroke="{}""#, style.stroke));
    parts.push(format!(r#"fill="{}""#, style.fill));
    parts.push(format!(
        r#"stroke-width="{:.2}""#,
        scaler.len(style.stroke_width).0
    ));

    if style.dashed {
        // C pikchr: stroke-dasharray: dashwid, dashwid
        let dash = scaler.len(dashwid).0;
        parts.push(format!("stroke-dasharray=\"{:.2},{:.2}\"", dash, dash));
    } else if style.dotted {
        // C pikchr: stroke-dasharray: stroke_width, dashwid
        let sw = scaler.len(style.stroke_width).0.max(2.1); // min 2.1 per C source
        let dash = scaler.len(dashwid).0;
        parts.push(format!("stroke-dasharray=\"{:.2},{:.2}\"", sw, dash));
    }

    parts.join(" ")
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render an arrowhead polygon at the end of a line
/// The arrowhead points in the direction from (sx, sy) to (ex, ey)
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
        r#"  <polygon points="{:.2},{:.2} {:.2},{:.2} {:.2},{:.2}" fill="{}"/>"#,
        ex, ey, p1_x, p1_y, p2_x, p2_y, style.stroke
    )
    .unwrap();
}

/// Render an arrowhead at the start of a line (pointing backwards)
fn render_arrowhead_start(
    svg: &mut String,
    sx: f64,
    sy: f64,
    ex: f64,
    ey: f64,
    style: &ObjectStyle,
    arrow_len: f64,
    arrow_width: f64,
) {
    render_arrowhead(svg, ex, ey, sx, sy, style, arrow_len, arrow_width);
}
