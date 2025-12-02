//! SVG rendering for pikchr diagrams

use crate::ast::*;
use std::collections::HashMap;
use std::fmt::Write;

/// Default sizes and settings (in pixels)
mod defaults {
    pub const LINE_WIDTH: f64 = 75.0;      // Default line length
    pub const BOX_WIDTH: f64 = 75.0;
    pub const BOX_HEIGHT: f64 = 50.0;
    pub const CIRCLE_RADIUS: f64 = 25.0;
    pub const ARROW_HEAD_SIZE: f64 = 8.0;
    pub const STROKE_WIDTH: f64 = 2.0;
    pub const FONT_SIZE: f64 = 12.0;
    pub const MARGIN: f64 = 10.0;
}

/// A point in 2D space
#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Bounding box
#[derive(Debug, Clone, Copy, Default)]
pub struct BoundingBox {
    pub min: Point,
    pub max: Point,
}

impl BoundingBox {
    pub fn new() -> Self {
        Self {
            min: Point::new(f64::MAX, f64::MAX),
            max: Point::new(f64::MIN, f64::MIN),
        }
    }

    pub fn expand(&mut self, p: Point) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    pub fn expand_rect(&mut self, center: Point, width: f64, height: f64) {
        self.expand(Point::new(center.x - width / 2.0, center.y - height / 2.0));
        self.expand(Point::new(center.x + width / 2.0, center.y + height / 2.0));
    }

    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }
}

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
    pub center: Point,
    pub width: f64,
    pub height: f64,
    pub start: Point,
    pub end: Point,
    /// Waypoints for multi-segment lines (includes start, intermediate points, and end)
    pub waypoints: Vec<Point>,
    pub text: Vec<PositionedText>,
    pub style: ObjectStyle,
    /// Child objects for sublists
    pub children: Vec<RenderedObject>,
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
    pub stroke_width: f64,
    pub dashed: bool,
    pub dotted: bool,
    pub arrow_start: bool,
    pub arrow_end: bool,
    pub invisible: bool,
    pub corner_radius: f64,
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
            corner_radius: 0.0,
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
    pub position: Point,
    /// Named objects for reference
    pub objects: HashMap<String, RenderedObject>,
    /// All objects in order
    pub object_list: Vec<RenderedObject>,
    /// Variables
    pub variables: HashMap<String, f64>,
    /// Bounding box of all objects
    pub bounds: BoundingBox,
}

impl Default for RenderContext {
    fn default() -> Self {
        let mut ctx = Self {
            direction: Direction::Right,
            position: Point::new(0.0, 0.0),
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
        // Default dimensions
        self.variables.insert("boxwid".to_string(), defaults::BOX_WIDTH);
        self.variables.insert("boxht".to_string(), defaults::BOX_HEIGHT);
        self.variables.insert("circlerad".to_string(), defaults::CIRCLE_RADIUS);
        self.variables.insert("linewid".to_string(), defaults::LINE_WIDTH);
        self.variables.insert("lineht".to_string(), defaults::LINE_WIDTH);
        self.variables.insert("arrowwid".to_string(), defaults::LINE_WIDTH);
        self.variables.insert("arrowht".to_string(), defaults::LINE_WIDTH);
        self.variables.insert("arrowhead".to_string(), defaults::ARROW_HEAD_SIZE);
        self.variables.insert("arcrad".to_string(), defaults::LINE_WIDTH / 2.0);
        self.variables.insert("ellipsewid".to_string(), defaults::BOX_WIDTH);
        self.variables.insert("ellipseht".to_string(), defaults::BOX_HEIGHT);
        self.variables.insert("movewid".to_string(), defaults::LINE_WIDTH);
        self.variables.insert("textwid".to_string(), 0.0);
        self.variables.insert("textht".to_string(), defaults::FONT_SIZE);
        self.variables.insert("scale".to_string(), 1.0);
        self.variables.insert("thickness".to_string(), defaults::STROKE_WIDTH);
        self.variables.insert("fontscale".to_string(), 1.0);
        self.variables.insert("charht".to_string(), defaults::FONT_SIZE);
        self.variables.insert("charwid".to_string(), defaults::FONT_SIZE * 0.6);
        self.variables.insert("margin".to_string(), defaults::MARGIN);

        // Color names as numeric values (0-1 range)
        // These are approximate X11 color values
        self.variables.insert("white".to_string(), 0xffffff as f64);
        self.variables.insert("black".to_string(), 0x000000 as f64);
        self.variables.insert("red".to_string(), 0xff0000 as f64);
        self.variables.insert("green".to_string(), 0x00ff00 as f64);
        self.variables.insert("blue".to_string(), 0x0000ff as f64);
        self.variables.insert("yellow".to_string(), 0xffff00 as f64);
        self.variables.insert("cyan".to_string(), 0x00ffff as f64);
        self.variables.insert("magenta".to_string(), 0xff00ff as f64);
        self.variables.insert("gray".to_string(), 0x808080 as f64);
        self.variables.insert("grey".to_string(), 0x808080 as f64);
        self.variables.insert("lightgray".to_string(), 0xd3d3d3 as f64);
        self.variables.insert("lightgrey".to_string(), 0xd3d3d3 as f64);
        self.variables.insert("darkgray".to_string(), 0xa9a9a9 as f64);
        self.variables.insert("darkgrey".to_string(), 0xa9a9a9 as f64);
        self.variables.insert("orange".to_string(), 0xffa500 as f64);
        self.variables.insert("pink".to_string(), 0xffc0cb as f64);
        self.variables.insert("purple".to_string(), 0x800080 as f64);
        self.variables.insert("bisque".to_string(), 0xffe4c4 as f64);
        self.variables.insert("beige".to_string(), 0xf5f5dc as f64);
        self.variables.insert("brown".to_string(), 0xa52a2a as f64);
        self.variables.insert("coral".to_string(), 0xff7f50 as f64);
        self.variables.insert("gold".to_string(), 0xffd700 as f64);
        self.variables.insert("ivory".to_string(), 0xfffff0 as f64);
        self.variables.insert("khaki".to_string(), 0xf0e68c as f64);
        self.variables.insert("lavender".to_string(), 0xe6e6fa as f64);
        self.variables.insert("linen".to_string(), 0xfaf0e6 as f64);
        self.variables.insert("maroon".to_string(), 0x800000 as f64);
        self.variables.insert("navy".to_string(), 0x000080 as f64);
        self.variables.insert("olive".to_string(), 0x808000 as f64);
        self.variables.insert("salmon".to_string(), 0xfa8072 as f64);
        self.variables.insert("silver".to_string(), 0xc0c0c0 as f64);
        self.variables.insert("tan".to_string(), 0xd2b48c as f64);
        self.variables.insert("teal".to_string(), 0x008080 as f64);
        self.variables.insert("tomato".to_string(), 0xff6347 as f64);
        self.variables.insert("turquoise".to_string(), 0x40e0d0 as f64);
        self.variables.insert("violet".to_string(), 0xee82ee as f64);
        self.variables.insert("wheat".to_string(), 0xf5deb3 as f64);

        // Common abbreviations
        self.variables.insert("wid".to_string(), defaults::BOX_WIDTH);
        self.variables.insert("ht".to_string(), defaults::BOX_HEIGHT);
        self.variables.insert("rad".to_string(), defaults::CIRCLE_RADIUS);
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
        let filtered: Vec<_> = self.object_list.iter()
            .filter(|o| class.map(|c| o.class == c).unwrap_or(true))
            .collect();
        filtered.get(n.saturating_sub(1)).copied()
    }

    /// Get the last object of a class
    pub fn get_last_object(&self, class: Option<ObjectClass>) -> Option<&RenderedObject> {
        self.object_list.iter()
            .rev()
            .find(|o| class.map(|c| o.class == c).unwrap_or(true))
    }

    /// Move position in the current direction
    pub fn advance(&mut self, distance: f64) {
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
        match obj.class {
            ObjectClass::Line | ObjectClass::Arrow | ObjectClass::Spline | ObjectClass::Arc => {
                // Expand bounds to include all waypoints
                for pt in &obj.waypoints {
                    self.bounds.expand(*pt);
                }
            }
            _ => {
                self.bounds.expand_rect(obj.center, obj.width, obj.height);
            }
        }

        // Update position to the exit point of the object
        self.position = obj.end;

        // Store named objects
        if let Some(ref name) = obj.name {
            self.objects.insert(name.clone(), obj.clone());
        }

        self.object_list.push(obj);
    }
}

/// Render a pikchr program to SVG
pub fn render(program: &Program) -> Result<String, miette::Report> {
    let mut ctx = RenderContext::new();

    // Process all statements
    for stmt in &program.statements {
        render_statement(&mut ctx, stmt)?;
    }

    // Generate SVG
    generate_svg(&ctx)
}

fn render_statement(ctx: &mut RenderContext, stmt: &Statement) -> Result<(), miette::Report> {
    match stmt {
        Statement::Direction(dir) => {
            ctx.direction = *dir;
        }
        Statement::Assignment(assign) => {
            let value = eval_rvalue(ctx, &assign.rvalue)?;
            match &assign.lvalue {
                LValue::Variable(name) => {
                    ctx.variables.insert(name.clone(), value);
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
        Statement::Define(_) | Statement::MacroCall(_) | Statement::Assert(_) | Statement::Print(_) => {
            // Not rendered
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
            ClassName::Circle => (ObjectClass::Circle, defaults::CIRCLE_RADIUS * 2.0, defaults::CIRCLE_RADIUS * 2.0),
            ClassName::Ellipse => (ObjectClass::Ellipse, defaults::BOX_WIDTH, defaults::BOX_HEIGHT),
            ClassName::Oval => (ObjectClass::Oval, defaults::BOX_WIDTH, defaults::BOX_HEIGHT / 2.0),
            ClassName::Cylinder => (ObjectClass::Cylinder, defaults::BOX_WIDTH, defaults::BOX_HEIGHT),
            ClassName::Diamond => (ObjectClass::Diamond, defaults::BOX_WIDTH, defaults::BOX_HEIGHT),
            ClassName::File => (ObjectClass::File, defaults::BOX_WIDTH, defaults::BOX_HEIGHT),
            ClassName::Line => (ObjectClass::Line, defaults::LINE_WIDTH, 0.0),
            ClassName::Arrow => (ObjectClass::Arrow, defaults::LINE_WIDTH, 0.0),
            ClassName::Spline => (ObjectClass::Spline, defaults::LINE_WIDTH, 0.0),
            ClassName::Arc => (ObjectClass::Arc, defaults::LINE_WIDTH, defaults::LINE_WIDTH),
            ClassName::Move => (ObjectClass::Move, defaults::LINE_WIDTH, 0.0),
            ClassName::Dot => (ObjectClass::Dot, 4.0, 4.0),
            ClassName::Text => (ObjectClass::Text, 0.0, 0.0),
        },
        BaseType::Text(s, _) => {
            // Estimate text dimensions
            let w = s.value.len() as f64 * defaults::FONT_SIZE * 0.6;
            let h = defaults::FONT_SIZE * 1.2;
            (ObjectClass::Text, w, h)
        }
        BaseType::Sublist(_) => (ObjectClass::Sublist, defaults::BOX_WIDTH, defaults::BOX_HEIGHT),
    };

    let mut style = ObjectStyle::default();
    let mut text = Vec::new();
    let mut explicit_position: Option<Point> = None;
    let mut from_position: Option<Point> = None;
    let mut to_position: Option<Point> = None;
    let mut line_direction: Option<Direction> = None;
    let mut line_distance: Option<f64> = None;
    let mut then_clauses: Vec<ThenClause> = Vec::new();
    let mut with_clause: Option<(EdgePoint, Point)> = None; // (edge, target_position)

    // Extract text from basetype
    if let BaseType::Text(s, pos) = &obj_stmt.basetype {
        text.push(PositionedText::from_textposition(s.value.clone(), pos.as_ref()));
    }

    // Default arrow style for arrows
    if class == ObjectClass::Arrow {
        style.arrow_end = true;
    }

    // Process attributes
    for attr in &obj_stmt.attributes {
        match attr {
            Attribute::NumProperty(prop, relexpr) => {
                let val = eval_expr(ctx, &relexpr.expr)?;
                match prop {
                    NumProperty::Width => width = val,
                    NumProperty::Height => height = val,
                    NumProperty::Radius => {
                        // For circles/ellipses, radius sets size
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
                text.push(PositionedText::from_textposition(s.value.clone(), pos.as_ref()));
            }
            Attribute::At(pos) => {
                if let Ok(p) = eval_position(ctx, pos) {
                    explicit_position = Some(p);
                }
            }
            Attribute::From(pos) => {
                if let Ok(p) = eval_position(ctx, pos) {
                    from_position = Some(p);
                }
            }
            Attribute::To(pos) => {
                if let Ok(p) = eval_position(ctx, pos) {
                    to_position = Some(p);
                }
            }
            Attribute::DirectionMove(_go, dir, dist) => {
                line_direction = Some(*dir);
                if let Some(relexpr) = dist {
                    if let Ok(d) = eval_expr(ctx, &relexpr.expr) {
                        line_distance = Some(d);
                    }
                }
            }
            Attribute::BareExpr(relexpr) => {
                // A bare expression is typically a distance
                if let Ok(d) = eval_expr(ctx, &relexpr.expr) {
                    line_distance = Some(d);
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
        let max_text_width = text.iter()
            .map(|t| t.value.len() as f64 * char_width)
            .fold(0.0_f64, |a, b| a.max(b));

        // Count lines (excluding above/below positioned text)
        let center_lines = text.iter()
            .filter(|t| !t.above && !t.below)
            .count();

        let fit_width = max_text_width + padding * 2.0;
        let fit_height = (center_lines as f64 * defaults::FONT_SIZE) + padding * 2.0;

        // Only expand, don't shrink
        width = width.max(fit_width);
        height = height.max(fit_height);
    }

    // Determine the effective direction and distance
    let effective_dir = line_direction.unwrap_or(ctx.direction);
    let effective_distance = line_distance.unwrap_or(width);

    // Calculate position based on object type
    let (center, start, end, waypoints) = if from_position.is_some() || to_position.is_some() || line_direction.is_some() || !then_clauses.is_empty() {
        // Line-like objects with explicit from/to, direction, or then clauses
        let start = from_position.unwrap_or(ctx.position);

        // Build waypoints starting from start
        let mut points = vec![start];
        let mut current_pos = start;
        let mut current_dir = effective_dir;

        // First segment (from start in direction with distance)
        if to_position.is_none() && then_clauses.is_empty() {
            // Simple line: just one segment
            let next = move_in_direction(current_pos, current_dir, effective_distance);
            points.push(next);
        } else if to_position.is_some() && then_clauses.is_empty() {
            // from X to Y - just two points
            points.push(to_position.unwrap());
        } else {
            // Has then clauses - first move in initial direction
            let next = move_in_direction(current_pos, current_dir, effective_distance);
            points.push(next);
            current_pos = next;

            // Process each then clause
            for clause in &then_clauses {
                let (next_point, next_dir) = eval_then_clause(ctx, clause, current_pos, current_dir, width)?;
                points.push(next_point);
                current_pos = next_point;
                current_dir = next_dir;
            }
        }

        let end = *points.last().unwrap_or(&start);
        let center = Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
        (center, start, end, points)
    } else if let Some((edge, target)) = with_clause {
        // Position object so that specified edge is at target position
        let center = calculate_center_from_edge(edge, target, width, height);
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

    // Handle sublist: render nested statements
    let children = if let BaseType::Sublist(statements) = &obj_stmt.basetype {
        render_sublist(ctx, statements)?
    } else {
        Vec::new()
    };

    Ok(RenderedObject {
        name,
        class,
        center,
        width,
        height,
        start,
        end,
        waypoints,
        text,
        style,
        children,
    })
}

/// Render a sublist of statements and return the rendered children
fn render_sublist(ctx: &mut RenderContext, statements: &[Statement]) -> Result<Vec<RenderedObject>, miette::Report> {
    // Save current context state
    let saved_position = ctx.position;
    let saved_direction = ctx.direction;

    // Reset position for sublist (start from origin, will be offset later)
    ctx.position = Point::new(0.0, 0.0);

    // Render each statement in the sublist
    let mut children = Vec::new();
    for stmt in statements {
        if let Statement::Object(obj_stmt) = stmt {
            let rendered = render_object_stmt(ctx, obj_stmt, None)?;
            children.push(rendered);
        }
    }

    // Restore context state
    ctx.position = saved_position;
    ctx.direction = saved_direction;

    Ok(children)
}

/// Calculate center position given that a specific edge should be at target
fn calculate_center_from_edge(edge: EdgePoint, target: Point, width: f64, height: f64) -> Point {
    let hw = width / 2.0;
    let hh = height / 2.0;

    match edge {
        EdgePoint::North | EdgePoint::N | EdgePoint::Top | EdgePoint::T => {
            Point::new(target.x, target.y + hh)
        }
        EdgePoint::South | EdgePoint::S | EdgePoint::Bottom => {
            Point::new(target.x, target.y - hh)
        }
        EdgePoint::East | EdgePoint::E | EdgePoint::Right => {
            Point::new(target.x - hw, target.y)
        }
        EdgePoint::West | EdgePoint::W | EdgePoint::Left => {
            Point::new(target.x + hw, target.y)
        }
        EdgePoint::NorthEast => Point::new(target.x - hw, target.y + hh),
        EdgePoint::NorthWest => Point::new(target.x + hw, target.y + hh),
        EdgePoint::SouthEast => Point::new(target.x - hw, target.y - hh),
        EdgePoint::SouthWest => Point::new(target.x + hw, target.y - hh),
        EdgePoint::Center | EdgePoint::C => target,
        EdgePoint::Start | EdgePoint::End => target, // For lines, just use target
    }
}

/// Move a point in a direction by a distance
fn move_in_direction(pos: Point, dir: Direction, distance: f64) -> Point {
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
    current_pos: Point,
    current_dir: Direction,
    default_distance: f64,
) -> Result<(Point, Direction), miette::Report> {
    match clause {
        ThenClause::To(pos) => {
            let target = eval_position(ctx, pos)?;
            Ok((target, current_dir))
        }
        ThenClause::DirectionMove(dir, dist) => {
            let distance = if let Some(relexpr) = dist {
                eval_expr(ctx, &relexpr.expr)?
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
                eval_expr(ctx, &relexpr.expr)?
            } else {
                default_distance
            };
            let angle = eval_expr(ctx, angle_expr)?;
            // Convert angle (degrees, 0 = north/up, clockwise) to radians
            let rad = (90.0 - angle).to_radians();
            let next = Point::new(
                current_pos.x + distance * rad.cos(),
                current_pos.y - distance * rad.sin(),
            );
            Ok((next, current_dir))
        }
        ThenClause::EdgePoint(dist, edge) => {
            let distance = if let Some(relexpr) = dist {
                eval_expr(ctx, &relexpr.expr)?
            } else {
                default_distance
            };
            // Get direction from edge point
            let (dx, dy) = edge_point_offset(edge);
            let next = Point::new(
                current_pos.x + dx * distance,
                current_pos.y + dy * distance,
            );
            Ok((next, current_dir))
        }
    }
}

/// Calculate start/end points for an object at a specific center position
fn calculate_object_position_at(
    direction: Direction,
    center: Point,
    width: f64,
    height: f64,
) -> (Point, Point, Point) {
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
    width: f64,
    height: f64,
) -> (Point, Point, Point) {
    let start = ctx.position;

    // For line-like objects, calculate end based on direction and length
    let (end, center) = match class {
        ObjectClass::Line | ObjectClass::Arrow | ObjectClass::Spline | ObjectClass::Move => {
            let end = match ctx.direction {
                Direction::Right => Point::new(start.x + width, start.y),
                Direction::Left => Point::new(start.x - width, start.y),
                Direction::Up => Point::new(start.x, start.y - width),
                Direction::Down => Point::new(start.x, start.y + width),
            };
            let center = Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
            (end, center)
        }
        _ => {
            // For shaped objects, center at the current position
            // and adjust start/end to be entry/exit points
            let center = match ctx.direction {
                Direction::Right => Point::new(start.x + width / 2.0, start.y),
                Direction::Left => Point::new(start.x - width / 2.0, start.y),
                Direction::Up => Point::new(start.x, start.y - height / 2.0),
                Direction::Down => Point::new(start.x, start.y + height / 2.0),
            };
            let end = match ctx.direction {
                Direction::Right => Point::new(center.x + width / 2.0, center.y),
                Direction::Left => Point::new(center.x - width / 2.0, center.y),
                Direction::Up => Point::new(center.x, center.y - height / 2.0),
                Direction::Down => Point::new(center.x, center.y + height / 2.0),
            };
            (end, center)
        }
    };

    (center, start, end)
}

fn eval_expr(ctx: &RenderContext, expr: &Expr) -> Result<f64, miette::Report> {
    match expr {
        Expr::Number(n) => Ok(*n),
        Expr::Variable(name) => {
            ctx.variables.get(name).copied()
                .ok_or_else(|| miette::miette!("Undefined variable: {}", name))
        }
        Expr::BinaryOp(lhs, op, rhs) => {
            let l = eval_expr(ctx, lhs)?;
            let r = eval_expr(ctx, rhs)?;
            Ok(match op {
                BinaryOp::Add => l + r,
                BinaryOp::Sub => l - r,
                BinaryOp::Mul => l * r,
                BinaryOp::Div => l / r,
            })
        }
        Expr::UnaryOp(op, e) => {
            let v = eval_expr(ctx, e)?;
            Ok(match op {
                UnaryOp::Neg => -v,
                UnaryOp::Pos => v,
            })
        }
        Expr::ParenExpr(e) => eval_expr(ctx, e),
        Expr::FuncCall(fc) => {
            let args: Result<Vec<f64>, _> = fc.args.iter().map(|a| eval_expr(ctx, a)).collect();
            let args = args?;
            Ok(match fc.func {
                Function::Abs => args[0].abs(),
                Function::Cos => args[0].to_radians().cos(),
                Function::Sin => args[0].to_radians().sin(),
                Function::Int => args[0].trunc(),
                Function::Sqrt => args[0].sqrt(),
                Function::Max => args[0].max(args[1]),
                Function::Min => args[0].min(args[1]),
            })
        }
        _ => Ok(0.0),
    }
}

fn eval_rvalue(ctx: &RenderContext, rvalue: &RValue) -> Result<f64, miette::Report> {
    match rvalue {
        RValue::Expr(e) => eval_expr(ctx, e),
        RValue::PlaceName(_) => Ok(0.0), // Color names return 0
    }
}

fn eval_position(ctx: &RenderContext, pos: &Position) -> Result<Point, miette::Report> {
    match pos {
        Position::Coords(x, y) => {
            let px = eval_expr(ctx, x)?;
            let py = eval_expr(ctx, y)?;
            Ok(Point::new(px, py))
        }
        Position::Place(place) => eval_place(ctx, place),
        Position::PlaceOffset(place, op, dx, dy) => {
            let base = eval_place(ctx, place)?;
            let dx_val = eval_expr(ctx, dx)?;
            let dy_val = eval_expr(ctx, dy)?;
            match op {
                BinaryOp::Add => Ok(Point::new(base.x + dx_val, base.y + dy_val)),
                BinaryOp::Sub => Ok(Point::new(base.x - dx_val, base.y - dy_val)),
                _ => Ok(base),
            }
        }
        Position::Between(factor, pos1, pos2) => {
            let f = eval_expr(ctx, factor)?;
            let p1 = eval_position(ctx, pos1)?;
            let p2 = eval_position(ctx, pos2)?;
            Ok(Point::new(
                p1.x + (p2.x - p1.x) * f,
                p1.y + (p2.y - p1.y) * f,
            ))
        }
        Position::Bracket(factor, pos1, pos2) => {
            // Same as between
            let f = eval_expr(ctx, factor)?;
            let p1 = eval_position(ctx, pos1)?;
            let p2 = eval_position(ctx, pos2)?;
            Ok(Point::new(
                p1.x + (p2.x - p1.x) * f,
                p1.y + (p2.y - p1.y) * f,
            ))
        }
        Position::AboveBelow(dist, ab, base_pos) => {
            let d = eval_expr(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            match ab {
                AboveBelow::Above => Ok(Point::new(base.x, base.y - d)),
                AboveBelow::Below => Ok(Point::new(base.x, base.y + d)),
            }
        }
        Position::LeftRightOf(dist, lr, base_pos) => {
            let d = eval_expr(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            match lr {
                LeftRight::Left => Ok(Point::new(base.x - d, base.y)),
                LeftRight::Right => Ok(Point::new(base.x + d, base.y)),
            }
        }
        Position::EdgePointOf(dist, edge, base_pos) => {
            let d = eval_expr(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            // Calculate offset based on edge direction
            let (dx, dy) = edge_point_offset(edge);
            Ok(Point::new(base.x + dx * d, base.y + dy * d))
        }
        Position::Heading(dist, heading, base_pos) => {
            let d = eval_expr(ctx, dist)?;
            let base = eval_position(ctx, base_pos)?;
            let angle = match heading {
                HeadingDir::EdgePoint(ep) => edge_point_to_angle(ep),
                HeadingDir::Expr(e) => eval_expr(ctx, e).unwrap_or(0.0),
            };
            // Convert angle (degrees, 0 = north, clockwise) to radians
            let rad = (90.0 - angle).to_radians();
            Ok(Point::new(
                base.x + d * rad.cos(),
                base.y - d * rad.sin(),
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

/// Get the unit offset direction for an edge point
fn edge_point_offset(edge: &EdgePoint) -> (f64, f64) {
    match edge {
        EdgePoint::North | EdgePoint::N | EdgePoint::Top | EdgePoint::T => (0.0, -1.0),
        EdgePoint::South | EdgePoint::S | EdgePoint::Bottom => (0.0, 1.0),
        EdgePoint::East | EdgePoint::E | EdgePoint::Right => (1.0, 0.0),
        EdgePoint::West | EdgePoint::W | EdgePoint::Left => (-1.0, 0.0),
        EdgePoint::NorthEast => (0.707, -0.707),
        EdgePoint::NorthWest => (-0.707, -0.707),
        EdgePoint::SouthEast => (0.707, 0.707),
        EdgePoint::SouthWest => (-0.707, 0.707),
        EdgePoint::Center | EdgePoint::C => (0.0, 0.0),
        EdgePoint::Start => (-1.0, 0.0),
        EdgePoint::End => (1.0, 0.0),
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

fn eval_place(ctx: &RenderContext, place: &Place) -> Result<Point, miette::Report> {
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
            match &name.base {
                ObjectNameBase::PlaceName(n) => ctx.get_object(n),
                ObjectNameBase::This => ctx.last_object(),
            }
        }
        Object::Nth(nth) => {
            match nth {
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
            }
        }
    }
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

fn get_edge_point(obj: &RenderedObject, edge: &EdgePoint) -> Point {
    let cx = obj.center.x;
    let cy = obj.center.y;
    let hw = obj.width / 2.0;
    let hh = obj.height / 2.0;

    match edge {
        EdgePoint::North | EdgePoint::N | EdgePoint::Top | EdgePoint::T => Point::new(cx, cy - hh),
        EdgePoint::South | EdgePoint::S | EdgePoint::Bottom => Point::new(cx, cy + hh),
        EdgePoint::East | EdgePoint::E | EdgePoint::Right => Point::new(cx + hw, cy),
        EdgePoint::West | EdgePoint::W | EdgePoint::Left => Point::new(cx - hw, cy),
        EdgePoint::NorthEast => Point::new(cx + hw, cy - hh),
        EdgePoint::NorthWest => Point::new(cx - hw, cy - hh),
        EdgePoint::SouthEast => Point::new(cx + hw, cy + hh),
        EdgePoint::SouthWest => Point::new(cx - hw, cy + hh),
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

fn generate_svg(ctx: &RenderContext) -> Result<String, miette::Report> {
    let margin = defaults::MARGIN;
    let mut bounds = ctx.bounds;

    // Ensure we have some content
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        bounds = BoundingBox {
            min: Point::new(0.0, 0.0),
            max: Point::new(100.0, 100.0),
        };
    }

    let view_width = bounds.width() + margin * 2.0;
    let view_height = bounds.height() + margin * 2.0;
    let offset_x = -bounds.min.x + margin;
    let offset_y = -bounds.min.y + margin;

    let mut svg = String::new();

    // SVG header
    writeln!(svg, r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {:.2} {:.2}">"#,
             view_width, view_height).unwrap();

    // Arrowheads are now rendered inline as polygon elements (matching C pikchr)

    // Render each object
    for obj in &ctx.object_list {
        if obj.style.invisible {
            continue;
        }

        let tx = obj.center.x + offset_x;
        let ty = obj.center.y + offset_y;
        let sx = obj.start.x + offset_x;
        let sy = obj.start.y + offset_y;
        let ex = obj.end.x + offset_x;
        let ey = obj.end.y + offset_y;

        let stroke_style = format_stroke_style(&obj.style);

        match obj.class {
            ObjectClass::Box => {
                let x = tx - obj.width / 2.0;
                let y = ty - obj.height / 2.0;
                if obj.style.corner_radius > 0.0 {
                    writeln!(svg, r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="{:.2}" ry="{:.2}" {}/>"#,
                             x, y, obj.width, obj.height, obj.style.corner_radius, obj.style.corner_radius, stroke_style).unwrap();
                } else {
                    writeln!(svg, r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" {}/>"#,
                             x, y, obj.width, obj.height, stroke_style).unwrap();
                }
            }
            ObjectClass::Circle => {
                let r = obj.width / 2.0;
                writeln!(svg, r#"  <circle cx="{:.2}" cy="{:.2}" r="{:.2}" {}/>"#,
                         tx, ty, r, stroke_style).unwrap();
            }
            ObjectClass::Dot => {
                // Dot is a small filled circle
                let r = obj.width / 2.0;
                let fill = if obj.style.fill == "none" { &obj.style.stroke } else { &obj.style.fill };
                writeln!(svg, r#"  <circle cx="{:.2}" cy="{:.2}" r="{:.2}" fill="{}" stroke="none"/>"#,
                         tx, ty, r, fill).unwrap();
            }
            ObjectClass::Ellipse => {
                let rx = obj.width / 2.0;
                let ry = obj.height / 2.0;
                writeln!(svg, r#"  <ellipse cx="{:.2}" cy="{:.2}" rx="{:.2}" ry="{:.2}" {}/>"#,
                         tx, ty, rx, ry, stroke_style).unwrap();
            }
            ObjectClass::Oval => {
                // Oval is a pill shape (rounded rectangle with fully rounded ends)
                render_oval(&mut svg, tx, ty, obj.width, obj.height, &stroke_style);
            }
            ObjectClass::Cylinder => {
                // Cylinder: elliptical top/bottom with vertical sides
                render_cylinder(&mut svg, tx, ty, obj.width, obj.height, &stroke_style, &obj.style);
            }
            ObjectClass::File => {
                // File: document shape with folded corner
                render_file(&mut svg, tx, ty, obj.width, obj.height, &stroke_style);
            }
            ObjectClass::Line | ObjectClass::Arrow => {
                // Apply chop if needed (shorten line from both ends)
                let chop_amount = if obj.style.chop { defaults::CIRCLE_RADIUS } else { 0.0 };
                let (draw_sx, draw_sy, draw_ex, draw_ey) = if chop_amount > 0.0 {
                    chop_line(sx, sy, ex, ey, chop_amount)
                } else {
                    (sx, sy, ex, ey)
                };

                if obj.waypoints.len() <= 2 {
                    // Simple line - render as <path> (matching C pikchr)
                    // First render arrowhead polygon if needed (rendered before line, like C)
                    if obj.style.arrow_end {
                        render_arrowhead(&mut svg, draw_sx, draw_sy, draw_ex, draw_ey, &obj.style);
                    }
                    // Render the line path
                    writeln!(svg, r#"  <path d="M{:.2},{:.2}L{:.2},{:.2}" {}/>"#,
                             draw_sx, draw_sy, draw_ex, draw_ey, stroke_style).unwrap();
                    if obj.style.arrow_start {
                        render_arrowhead_start(&mut svg, draw_sx, draw_sy, draw_ex, draw_ey, &obj.style);
                    }
                } else {
                    // Multi-segment polyline - chop first and last segments
                    let mut points = obj.waypoints.clone();
                    if obj.style.chop && points.len() >= 2 {
                        // Chop start
                        let p0 = points[0];
                        let p1 = points[1];
                        let (new_x, new_y, _, _) = chop_line(p0.x, p0.y, p1.x, p1.y, chop_amount);
                        points[0] = Point::new(new_x, new_y);

                        // Chop end
                        let n = points.len();
                        let pn1 = points[n - 2];
                        let pn = points[n - 1];
                        let (_, _, new_x, new_y) = chop_line(pn1.x, pn1.y, pn.x, pn.y, chop_amount);
                        points[n - 1] = Point::new(new_x, new_y);
                    }

                    // Build path string
                    let path_str: String = points.iter().enumerate()
                        .map(|(i, p)| {
                            let cmd = if i == 0 { "M" } else { "L" };
                            format!("{}{:.2},{:.2}", cmd, p.x + offset_x, p.y + offset_y)
                        })
                        .collect::<Vec<_>>()
                        .join("");

                    if obj.style.close_path {
                        // Closed polygon - add Z to close path
                        writeln!(svg, r#"  <path d="{}Z" {}/>"#, path_str, stroke_style).unwrap();
                    } else {
                        // Render arrowheads
                        let n = points.len();
                        if obj.style.arrow_end && n >= 2 {
                            let p1 = points[n - 2];
                            let p2 = points[n - 1];
                            render_arrowhead(&mut svg, p1.x + offset_x, p1.y + offset_y,
                                           p2.x + offset_x, p2.y + offset_y, &obj.style);
                        }
                        writeln!(svg, r#"  <path d="{}" {}/>"#, path_str, stroke_style).unwrap();
                        if obj.style.arrow_start && n >= 2 {
                            let p1 = points[0];
                            let p2 = points[1];
                            render_arrowhead_start(&mut svg, p1.x + offset_x, p1.y + offset_y,
                                                  p2.x + offset_x, p2.y + offset_y, &obj.style);
                        }
                    }
                }
            }
            ObjectClass::Spline => {
                if obj.waypoints.len() <= 2 {
                    // Simple line for spline with only 2 points
                    if obj.style.arrow_end {
                        render_arrowhead(&mut svg, sx, sy, ex, ey, &obj.style);
                    }
                    writeln!(svg, r#"  <path d="M{:.2},{:.2}L{:.2},{:.2}" {}/>"#,
                             sx, sy, ex, ey, stroke_style).unwrap();
                    if obj.style.arrow_start {
                        render_arrowhead_start(&mut svg, sx, sy, ex, ey, &obj.style);
                    }
                } else {
                    // Multi-segment spline - use a smooth path with curves
                    render_spline_path(&mut svg, &obj.waypoints, offset_x, offset_y, &stroke_style, &obj.style);
                }
            }
            ObjectClass::Arc => {
                // Arc: quarter circle arc
                render_arc(&mut svg, sx, sy, ex, ey, obj.width, &obj.style, &stroke_style);
            }
            ObjectClass::Diamond => {
                let points = format!("{:.2},{:.2} {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
                    tx, ty - obj.height / 2.0,  // top
                    tx + obj.width / 2.0, ty,   // right
                    tx, ty + obj.height / 2.0,  // bottom
                    tx - obj.width / 2.0, ty    // left
                );
                writeln!(svg, r#"  <polygon points="{}" {}/>"#, points, stroke_style).unwrap();
            }
            ObjectClass::Text => {
                render_positioned_text(&mut svg, &obj.text, tx, ty, obj.width, obj.height);
            }
            ObjectClass::Move => {
                // Move is invisible
            }
            ObjectClass::Sublist => {
                // Render sublist children with offset
                render_sublist_children(&mut svg, &obj.children, tx, ty);
            }
        }

        // Render text labels inside objects
        if obj.class != ObjectClass::Text && !obj.text.is_empty() {
            render_positioned_text(&mut svg, &obj.text, tx, ty, obj.width, obj.height);
        }
    }

    writeln!(svg, "</svg>").unwrap();

    Ok(svg)
}

/// Render positioned text labels
fn render_positioned_text(svg: &mut String, texts: &[PositionedText], cx: f64, cy: f64, width: f64, height: f64) {
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
        let font_size = get_font_size(text);
        let text_y = cy - height / 2.0 - font_size * (above_texts.len() - i) as f64;
        let (text_x, anchor) = get_text_anchor(text, cx, width);
        render_styled_text(svg, text, text_x, text_y, anchor, font_size);
    }

    // Render center texts (inside the shape)
    for (i, text) in center_texts.iter().enumerate() {
        let font_size = get_font_size(text);
        let text_y = cy + (i as f64 - center_texts.len() as f64 / 2.0 + 0.5) * font_size;
        let (text_x, anchor) = get_text_anchor(text, cx, width);
        render_styled_text(svg, text, text_x, text_y, anchor, font_size);
    }

    // Render below texts (below the shape)
    for (i, text) in below_texts.iter().enumerate() {
        let font_size = get_font_size(text);
        let text_y = cy + height / 2.0 + font_size * (i + 1) as f64;
        let (text_x, anchor) = get_text_anchor(text, cx, width);
        render_styled_text(svg, text, text_x, text_y, anchor, font_size);
    }
}

/// Get font size based on text style (big/small)
fn get_font_size(text: &PositionedText) -> f64 {
    if text.big {
        defaults::FONT_SIZE * 1.4
    } else if text.small {
        defaults::FONT_SIZE * 0.7
    } else {
        defaults::FONT_SIZE
    }
}

/// Render a single styled text element
fn render_styled_text(svg: &mut String, text: &PositionedText, x: f64, y: f64, anchor: &str, font_size: f64) {
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

    writeln!(svg, r#"  <text x="{:.2}" y="{:.2}" text-anchor="{}" dominant-baseline="middle" font-size="{:.2}"{}>{}</text>"#,
             x, y, anchor, font_size, style_str, escape_xml(&text.value)).unwrap();
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

/// Render sublist children with an offset
fn render_sublist_children(svg: &mut String, children: &[RenderedObject], offset_x: f64, offset_y: f64) {
    for child in children {
        let tx = child.center.x + offset_x;
        let ty = child.center.y + offset_y;
        let sx = child.start.x + offset_x;
        let sy = child.start.y + offset_y;
        let ex = child.end.x + offset_x;
        let ey = child.end.y + offset_y;

        let stroke_style = format_stroke_style(&child.style);

        match child.class {
            ObjectClass::Box => {
                let x = tx - child.width / 2.0;
                let y = ty - child.height / 2.0;
                if child.style.corner_radius > 0.0 {
                    writeln!(svg, r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="{:.2}" ry="{:.2}" {}/>"#,
                             x, y, child.width, child.height, child.style.corner_radius, child.style.corner_radius, stroke_style).unwrap();
                } else {
                    writeln!(svg, r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" {}/>"#,
                             x, y, child.width, child.height, stroke_style).unwrap();
                }
            }
            ObjectClass::Circle => {
                let r = child.width / 2.0;
                writeln!(svg, r#"  <circle cx="{:.2}" cy="{:.2}" r="{:.2}" {}/>"#,
                         tx, ty, r, stroke_style).unwrap();
            }
            ObjectClass::Line | ObjectClass::Arrow => {
                let marker_end = if child.style.arrow_end { r#" marker-end="url(#arrowhead)""# } else { "" };
                let marker_start = if child.style.arrow_start { r#" marker-start="url(#arrowhead-start)""# } else { "" };
                writeln!(svg, r#"  <line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" {}{}{}/>"#,
                         sx, sy, ex, ey, stroke_style, marker_end, marker_start).unwrap();
            }
            _ => {
                // Other shapes can be added as needed
            }
        }

        // Render text for child
        if !child.text.is_empty() {
            render_positioned_text(svg, &child.text, tx, ty, child.width, child.height);
        }

        // Recursively render nested sublists
        if !child.children.is_empty() {
            render_sublist_children(svg, &child.children, tx, ty);
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

/// Render an oval (pill shape)
fn render_oval(svg: &mut String, cx: f64, cy: f64, width: f64, height: f64, stroke_style: &str) {
    // Oval is a rounded rectangle where the radius is half the smaller dimension
    let radius = height.min(width) / 2.0;
    let x = cx - width / 2.0;
    let y = cy - height / 2.0;
    writeln!(svg, r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="{:.2}" ry="{:.2}" {}/>"#,
             x, y, width, height, radius, radius, stroke_style).unwrap();
}

/// Render a cylinder shape
fn render_cylinder(svg: &mut String, cx: f64, cy: f64, width: f64, height: f64, _stroke_style: &str, style: &ObjectStyle) {
    // Cylinder has elliptical top and bottom
    // The ellipse height is about 1/4 of the total width for a nice 3D effect
    let rx = width / 2.0;
    let ry = width / 8.0; // Ellipse vertical radius

    let top_y = cy - height / 2.0 + ry;
    let bottom_y = cy + height / 2.0 - ry;

    // Draw the body (two vertical lines + bottom ellipse arc)
    // Path: left side down, bottom arc, right side up
    let path = format!(
        "M {:.2},{:.2} L {:.2},{:.2} A {:.2},{:.2} 0 0,0 {:.2},{:.2} L {:.2},{:.2}",
        cx - rx, top_y,        // Start at top-left
        cx - rx, bottom_y,     // Line down to bottom-left
        rx, ry,                // Arc radii
        cx + rx, bottom_y,     // Arc to bottom-right
        cx + rx, top_y         // Line up to top-right
    );

    // Use fill for the body if specified
    let body_fill = if style.fill != "none" { &style.fill } else { "none" };
    writeln!(svg, r#"  <path d="{}" stroke="{}" fill="{}" stroke-width="{:.2}"/>"#,
             path, style.stroke, body_fill, style.stroke_width).unwrap();

    // Draw the top ellipse (full ellipse, filled)
    writeln!(svg, r#"  <ellipse cx="{:.2}" cy="{:.2}" rx="{:.2}" ry="{:.2}" stroke="{}" fill="{}" stroke-width="{:.2}"/>"#,
             cx, top_y, rx, ry, style.stroke, body_fill, style.stroke_width).unwrap();

    // Draw the bottom ellipse arc (only the front half, as a visible edge)
    let bottom_arc = format!(
        "M {:.2},{:.2} A {:.2},{:.2} 0 0,0 {:.2},{:.2}",
        cx - rx, bottom_y,
        rx, ry,
        cx + rx, bottom_y
    );
    writeln!(svg, r#"  <path d="{}" stroke="{}" fill="none" stroke-width="{:.2}"/>"#,
             bottom_arc, style.stroke, style.stroke_width).unwrap();
}

/// Render a file shape (document with folded corner)
fn render_file(svg: &mut String, cx: f64, cy: f64, width: f64, height: f64, stroke_style: &str) {
    // File shape: rectangle with top-right corner folded
    let fold_size = width.min(height) * 0.2; // Fold is 20% of smaller dimension

    let x = cx - width / 2.0;
    let y = cy - height / 2.0;
    let right = cx + width / 2.0;
    let bottom = cy + height / 2.0;

    // Main outline path (going clockwise from top-left)
    // Top-left -> top-right minus fold -> fold corner -> bottom-right -> bottom-left -> close
    let path = format!(
        "M {:.2},{:.2} L {:.2},{:.2} L {:.2},{:.2} L {:.2},{:.2} L {:.2},{:.2} Z",
        x, y,                          // Top-left
        right - fold_size, y,          // Top-right minus fold
        right, y + fold_size,          // Fold corner (diagonal)
        right, bottom,                 // Bottom-right
        x, bottom                      // Bottom-left
    );
    writeln!(svg, r#"  <path d="{}" {}/>"#, path, stroke_style).unwrap();

    // Draw the fold line (the crease)
    let fold_path = format!(
        "M {:.2},{:.2} L {:.2},{:.2} L {:.2},{:.2}",
        right - fold_size, y,              // Start at corner
        right - fold_size, y + fold_size,  // Down to fold
        right, y + fold_size               // Across to edge
    );
    writeln!(svg, r#"  <path d="{}" stroke="black" fill="none" stroke-width="1"/>"#, fold_path).unwrap();
}

/// Render a spline path (smooth bezier curves through waypoints)
fn render_spline_path(
    svg: &mut String,
    waypoints: &[Point],
    offset_x: f64,
    offset_y: f64,
    stroke_style: &str,
    style: &ObjectStyle,
) {
    if waypoints.is_empty() {
        return;
    }

    // Convert waypoints to offset coordinates
    let points: Vec<(f64, f64)> = waypoints
        .iter()
        .map(|p| (p.x + offset_x, p.y + offset_y))
        .collect();

    // Render arrowhead at end if needed
    let n = points.len();
    if style.arrow_end && n >= 2 {
        let p1 = points[n - 2];
        let p2 = points[n - 1];
        render_arrowhead(svg, p1.0, p1.1, p2.0, p2.1, style);
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
                path.push_str(&format!(" Q {:.2},{:.2} {:.2},{:.2}", prev.0, prev.1, mid.0, mid.1));
            }

            if i < points.len() - 1 {
                // Middle segments: curve through midpoints
                let next = points[i + 1];
                let mid = ((curr.0 + next.0) / 2.0, (curr.1 + next.1) / 2.0);
                path.push_str(&format!(" Q {:.2},{:.2} {:.2},{:.2}", curr.0, curr.1, mid.0, mid.1));
            } else {
                // Last segment: end at the final point
                path.push_str(&format!(" Q {:.2},{:.2} {:.2},{:.2}",
                    (prev.0 + curr.0) / 2.0, (prev.1 + curr.1) / 2.0,
                    curr.0, curr.1));
            }
        }
    }

    writeln!(svg, r#"  <path d="{}" {}/>"#, path, stroke_style).unwrap();

    // Render arrowhead at start if needed
    if style.arrow_start && n >= 2 {
        let p1 = points[0];
        let p2 = points[1];
        render_arrowhead_start(svg, p1.0, p1.1, p2.0, p2.1, style);
    }
}

/// Render an arc
fn render_arc(svg: &mut String, sx: f64, sy: f64, ex: f64, ey: f64, radius: f64, style: &ObjectStyle, stroke_style: &str) {
    // Determine arc direction and sweep
    let dx = ex - sx;
    let dy = ey - sy;

    // Default to quarter-circle arc
    let r = if radius > 0.0 { radius / 2.0 } else { (dx.abs() + dy.abs()) / 2.0 };

    // sweep-flag: 0 = counter-clockwise, 1 = clockwise
    // large-arc-flag: 0 = small arc, 1 = large arc
    let sweep = 1; // Default clockwise
    let large_arc = 0;

    // Render arrowheads as inline polygons
    if style.arrow_end {
        render_arrowhead(svg, sx, sy, ex, ey, style);
    }

    let path = format!(
        "M {:.2},{:.2} A {:.2},{:.2} 0 {} {} {:.2},{:.2}",
        sx, sy, r, r, large_arc, sweep, ex, ey
    );
    writeln!(svg, r#"  <path d="{}" {}/>"#, path, stroke_style).unwrap();

    if style.arrow_start {
        render_arrowhead_start(svg, sx, sy, ex, ey, style);
    }
}

fn format_stroke_style(style: &ObjectStyle) -> String {
    let mut parts = Vec::new();

    parts.push(format!(r#"stroke="{}""#, style.stroke));
    parts.push(format!(r#"fill="{}""#, style.fill));
    parts.push(format!(r#"stroke-width="{:.2}""#, style.stroke_width));

    if style.dashed {
        parts.push(r#"stroke-dasharray="5,5""#.to_string());
    } else if style.dotted {
        parts.push(r#"stroke-dasharray="2,2""#.to_string());
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
fn render_arrowhead(svg: &mut String, sx: f64, sy: f64, ex: f64, ey: f64, style: &ObjectStyle) {
    // Arrow dimensions (matching C pikchr proportions)
    let arrow_len = 11.52;  // Length of arrowhead
    let arrow_width = 4.32; // Half-width of arrowhead base

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
    // Base points are arrow_len back along the line, offset by arrow_width perpendicular
    let base_x = ex - ux * arrow_len;
    let base_y = ey - uy * arrow_len;

    let p1_x = base_x + px * arrow_width;
    let p1_y = base_y + py * arrow_width;
    let p2_x = base_x - px * arrow_width;
    let p2_y = base_y - py * arrow_width;

    writeln!(svg, r#"  <polygon points="{:.2},{:.2} {:.2},{:.2} {:.2},{:.2}" fill="{}"/>"#,
             ex, ey, p1_x, p1_y, p2_x, p2_y, style.stroke).unwrap();
}

/// Render an arrowhead at the start of a line (pointing backwards)
fn render_arrowhead_start(svg: &mut String, sx: f64, sy: f64, ex: f64, ey: f64, style: &ObjectStyle) {
    // Just render arrowhead in the opposite direction
    render_arrowhead(svg, ex, ey, sx, sy, style);
}
