//! SVG rendering for pikchr diagrams

use crate::ast::*;
use crate::types::{BoxIn, EvalValue, Length as Inches, Point, PtIn, Scaler, Size, UnitVec};
use facet_svg::facet_xml::SerializeOptions;
use facet_svg::{
    Circle, Color, Ellipse, Path, PathData, Polygon, Rect, Svg, SvgNode, SvgStyle, Text, facet_xml,
    fmt_num,
};
use std::collections::HashMap;
use std::fmt::Write;
use time::{OffsetDateTime, format_description};

// fmt_num is imported from facet_svg

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
    #[allow(dead_code)]
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
    pub const DIAMOND_WIDTH: Inches = Inches::inches(1.0); // C pikchr: diamond is larger than box
    pub const DIAMOND_HEIGHT: Inches = Inches::inches(0.75);
    pub const CIRCLE_RADIUS: Inches = Inches::inches(0.25);
    pub const STROKE_WIDTH: Inches = Inches::inches(0.015);
    pub const FONT_SIZE: f64 = 0.14; // approx charht (kept as f64 for text calculations)
    pub const MARGIN: f64 = 0.0; // kept as f64 for variable lookups
    pub const CHARWID: f64 = 0.08; // default character width in inches
}

/// Proportional character widths from C pikchr's awChar table.
/// Values are relative widths where 100 = average character width.
/// Index is (ASCII code - 0x20) for printable characters 0x20-0x7e.
#[rustfmt::skip]
const AW_CHAR: [u8; 95] = [
    // ' '   !    "    #    $    %    &    '
       45,  55,  62, 115,  90, 132, 125,  40,
    // (    )    *    +    ,    -    .    /
       55,  55,  71, 115,  45,  48,  45,  50,
    // 0    1    2    3    4    5    6    7
       91,  91,  91,  91,  91,  91,  91,  91,
    // 8    9    :    ;    <    =    >    ?
       91,  91,  50,  50, 120, 120, 120,  78,
    // @    A    B    C    D    E    F    G
      142, 102, 105, 110, 115, 105,  98, 105,
    // H    I    J    K    L    M    N    O
      125,  58,  58, 107,  95, 145, 125, 115,
    // P    Q    R    S    T    U    V    W
       95, 115, 107,  95,  97, 118, 102, 150,
    // X    Y    Z    [    \    ]    ^    _
      100,  93, 100,  58,  50,  58, 119,  72,
    // `    a    b    c    d    e    f    g
       72,  86,  92,  80,  92,  85,  52,  92,
    // h    i    j    k    l    m    n    o
       92,  47,  47,  88,  48, 135,  92,  86,
    // p    q    r    s    t    u    v    w
       92,  92,  69,  75,  58,  92,  80, 121,
    // x    y    z    {    |    }    ~
       81,  80,  76,  91,  49,  91, 118,
];

/// Calculate text width using proportional character widths like C pikchr.
/// Returns width in "hundredths" (sum of AW_CHAR values).
fn pik_text_length(text: &str) -> u32 {
    let mut cnt: u32 = 0;
    for c in text.chars() {
        if c >= ' ' && c <= '~' {
            cnt += AW_CHAR[(c as usize) - 0x20] as u32;
        } else {
            // Non-ASCII or control: use average width (100)
            cnt += 100;
        }
    }
    cnt
}

/// Calculate text width in inches using proportional character widths.
fn text_width_inches(text: &str, charwid: f64) -> f64 {
    let length_hundredths = pik_text_length(text);
    // C formula: pik_text_length * charWidth * 0.01
    length_hundredths as f64 * charwid * 0.01
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
    pub corner_radius: Inches,
}

impl EndpointObject {
    fn from_rendered(obj: &RenderedObject) -> Self {
        Self {
            class: obj.class,
            center: obj.center,
            width: obj.width,
            height: obj.height,
            corner_radius: obj.style.corner_radius,
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
    /// Current object being constructed (for `this` keyword support)
    pub current_object: Option<RenderedObject>,
    /// Macro definitions (name -> body)
    pub macros: HashMap<String, String>,
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
            current_object: None,
            macros: HashMap::new(),
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
            Direction::Up => self.position.y += distance,
            Direction::Down => self.position.y -= distance,
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
        _ => bounds.expand_rect(
            obj.center,
            Size {
                w: obj.width,
                h: obj.height,
            },
        ),
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
                defaults::DIAMOND_WIDTH,
                defaults::DIAMOND_HEIGHT,
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
                    Direction::Up => direction_offset_y += distance,
                    Direction::Down => direction_offset_y -= distance,
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
                        Direction::Up => direction_offset_y += val,
                        Direction::Down => direction_offset_y -= val,
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
fn move_in_direction(pos: PointIn, dir: Direction, distance: Inches) -> PointIn {
    match dir {
        Direction::Right => Point::new(pos.x + distance, pos.y),
        Direction::Left => Point::new(pos.x - distance, pos.y),
        Direction::Up => Point::new(pos.x, pos.y + distance),
        Direction::Down => Point::new(pos.x, pos.y - distance),
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
            Point::new(center.x, center.y - height / 2.0),
            Point::new(center.x, center.y + height / 2.0),
        ),
        Direction::Down => (
            Point::new(center.x, center.y + height / 2.0),
            Point::new(center.x, center.y - height / 2.0),
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
                Direction::Up => Point::new(start.x, start.y + width),
                Direction::Down => Point::new(start.x, start.y - width),
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
                Direction::Up => Point::new(ctx.position.x, ctx.position.y + half_h),
                Direction::Down => Point::new(ctx.position.x, ctx.position.y - half_h),
            };
            let start = match ctx.direction {
                Direction::Right => Point::new(center.x - half_w, center.y),
                Direction::Left => Point::new(center.x + half_w, center.y),
                Direction::Up => Point::new(center.x, center.y - half_h),
                Direction::Down => Point::new(center.x, center.y + half_h),
            };
            let end = match ctx.direction {
                Direction::Right => Point::new(center.x + half_w, center.y),
                Direction::Left => Point::new(center.x - half_w, center.y),
                Direction::Up => Point::new(center.x, center.y + half_h),
                Direction::Down => Point::new(center.x, center.y - half_h),
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
            // Pikchr uses Y-UP semantics in user syntax, but we use Y-DOWN internally.
            // So adding Y moves up (smaller Y in our system), subtracting moves down.
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
                AboveBelow::Above => Ok(Point::new(base.x, base.y + d)),
                AboveBelow::Below => Ok(Point::new(base.x, base.y - d)),
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
            // C pikchr uses: pt.x += dist*sin(r); pt.y += dist*cos(r);
            // But C uses Y-up internally and flips on output. We use Y-down (SVG),
            // so we negate the Y component here.
            let rad = angle.to_radians();
            Ok(Point::new(
                base.x + Inches(d.0 * rad.sin()),
                base.y - Inches(d.0 * rad.cos()),
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
    let r_scale = 144.0; // match pikchr.c rScale - always use base scale for coordinates
    let scale = get_scalar(ctx, "scale", 1.0);
    // C pikchr uses r_scale for coordinate conversion, scale only affects display size
    let scaler = Scaler::try_new(r_scale)
        .map_err(|e| miette::miette!("invalid scale value {}: {}", r_scale, e))?;
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
        if obj.style.invisible {
            continue;
        }

        let tx = scaler.px(obj.center.x + offset_x);
        let ty = scaler.px(obj.center.y + offset_y);
        let sx = scaler.px(obj.start.x + offset_x);
        let sy = scaler.px(obj.start.y + offset_y);
        let ex = scaler.px(obj.end.x + offset_x);
        let ey = scaler.px(obj.end.y + offset_y);

        let svg_style = create_svg_style(&obj.style, &scaler, dashwid);

        match obj.class {
            ObjectClass::Box => {
                let x1 = tx - scaler.px(obj.width / 2.0);
                let x2 = tx + scaler.px(obj.width / 2.0);
                let y1 = ty - scaler.px(obj.height / 2.0);
                let y2 = ty + scaler.px(obj.height / 2.0);

                let path_data = if obj.style.corner_radius > Inches::ZERO {
                    // Rounded box using proper path with arcs
                    let r = scaler.px(obj.style.corner_radius);
                    create_rounded_box_path(x1, y1, x2, y2, r)
                } else {
                    // Regular box: C pikchr renders boxes as path: M x1,y2 L x2,y2 L x2,y1 L x1,y1 Z
                    // (starts at bottom-left, goes clockwise)
                    PathData::new()
                        .m(x1, y2) // Start at bottom-left
                        .l(x2, y2) // Line to bottom-right
                        .l(x2, y1) // Line to top-right
                        .l(x1, y1) // Line to top-left
                        .z() // Close path
                };

                let path = Path {
                    d: Some(path_data),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style,
                };
                svg_children.push(SvgNode::Path(path));
            }
            ObjectClass::Circle => {
                let r = scaler.px(obj.width / 2.0);
                let circle = Circle {
                    cx: Some(tx),
                    cy: Some(ty),
                    r: Some(r),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style,
                };
                svg_children.push(SvgNode::Circle(circle));
            }
            ObjectClass::Dot => {
                // Dot is a small filled circle
                let r = scaler.px(obj.width / 2.0);
                let fill = if obj.style.fill == "none" {
                    &obj.style.stroke
                } else {
                    &obj.style.fill
                };
                let mut dot_style = SvgStyle::new();
                dot_style
                    .properties
                    .insert("fill".to_string(), fill.to_string());
                let circle = Circle {
                    cx: Some(tx),
                    cy: Some(ty),
                    r: Some(r),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: dot_style,
                };
                svg_children.push(SvgNode::Circle(circle));
            }
            ObjectClass::Ellipse => {
                let rx = scaler.px(obj.width / 2.0);
                let ry = scaler.px(obj.height / 2.0);
                let ellipse = Ellipse {
                    cx: Some(tx),
                    cy: Some(ty),
                    rx: Some(rx),
                    ry: Some(ry),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style.clone(),
                };
                svg_children.push(SvgNode::Ellipse(ellipse));
            }
            ObjectClass::Oval => {
                // Oval is a pill shape (rounded rectangle with fully rounded ends)
                let width = scaler.px(obj.width);
                let height = scaler.px(obj.height);
                let rx = height / 2.0; // Fully rounded ends
                let ry = height / 2.0;
                let x = tx - width / 2.0;
                let y = ty - height / 2.0;

                let rect = Rect {
                    x: Some(x),
                    y: Some(y),
                    width: Some(width),
                    height: Some(height),
                    rx: Some(rx),
                    ry: Some(ry),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style.clone(),
                };
                svg_children.push(SvgNode::Rect(rect));
            }
            ObjectClass::Cylinder => {
                // Cylinder: proper path-based rendering matching C pikchr
                let width = scaler.px(obj.width);
                let height = scaler.px(obj.height);
                let rx = width / 2.0;
                let ry = width / 8.0; // Ellipse vertical radius
                let top_y = ty - height / 2.0 + ry;

                let (body_path, bottom_arc_path) = create_cylinder_paths(tx, ty, width, height);

                // Body path with fill
                let body = Path {
                    d: Some(body_path),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style.clone(),
                };
                svg_children.push(SvgNode::Path(body));

                // Top ellipse (full ellipse, filled)
                let top_ellipse = Ellipse {
                    cx: Some(tx),
                    cy: Some(top_y),
                    rx: Some(rx),
                    ry: Some(ry),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style.clone(),
                };
                svg_children.push(SvgNode::Ellipse(top_ellipse));

                // Bottom arc (only front half, stroke only)
                let mut bottom_arc_style = svg_style.clone();
                bottom_arc_style
                    .properties
                    .insert("fill".to_string(), "none".to_string());

                let bottom_arc = Path {
                    d: Some(bottom_arc_path),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: bottom_arc_style,
                };
                svg_children.push(SvgNode::Path(bottom_arc));
            }
            ObjectClass::File => {
                // File: proper path-based rendering with fold matching C pikchr
                let width = scaler.px(obj.width);
                let height = scaler.px(obj.height);

                let (main_path, fold_path) = create_file_paths(tx, ty, width, height);

                // Main outline path
                let main = Path {
                    d: Some(main_path),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style.clone(),
                };
                svg_children.push(SvgNode::Path(main));

                // Fold line (stroke only, no fill)
                let mut fold_style = SvgStyle::new();
                fold_style
                    .properties
                    .insert("fill".to_string(), "none".to_string());
                fold_style
                    .properties
                    .insert("stroke".to_string(), "rgb(0,0,0)".to_string());
                fold_style
                    .properties
                    .insert("stroke-width".to_string(), "1".to_string());

                let fold = Path {
                    d: Some(fold_path),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: fold_style,
                };
                svg_children.push(SvgNode::Path(fold));
            }
            ObjectClass::Diamond => {
                // Diamond: rotated square/rhombus with vertices at edges
                // C pikchr uses path: M left,cy L cx,bottom L right,cy L cx,top Z
                let half_w = scaler.px(obj.width / 2.0);
                let half_h = scaler.px(obj.height / 2.0);
                let left = tx - half_w;
                let right = tx + half_w;
                let top = ty - half_h;
                let bottom = ty + half_h;
                let path_data = PathData::new()
                    .m(left, ty) // Left vertex
                    .l(tx, bottom) // Bottom vertex
                    .l(right, ty) // Right vertex
                    .l(tx, top) // Top vertex
                    .z(); // Close path
                let path = Path {
                    d: Some(path_data),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style,
                };
                svg_children.push(SvgNode::Path(path));
            }
            ObjectClass::Line | ObjectClass::Arrow => {
                // Auto-chop always applies for object-attached endpoints (trims to boundary)
                // The explicit "chop" attribute is for additional user-requested shortening
                let (draw_sx, draw_sy, draw_ex, draw_ey) = if obj.waypoints.len() <= 2 {
                    apply_auto_chop_simple_line(&scaler, obj, sx, sy, ex, ey, offset_x, offset_y)
                } else {
                    (sx, sy, ex, ey)
                };

                if obj.waypoints.len() <= 2 {
                    // Simple line - render as <path> (matching C pikchr)
                    // First render arrowhead polygon if needed (rendered before line, like C)
                    if obj.style.arrow_end {
                        if let Some(arrowhead) = render_arrowhead_dom(
                            draw_sx,
                            draw_sy,
                            draw_ex,
                            draw_ey,
                            &obj.style,
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        ) {
                            svg_children.push(SvgNode::Polygon(arrowhead));
                        }
                    }
                    if obj.style.arrow_start {
                        if let Some(arrowhead) = render_arrowhead_dom(
                            draw_ex,
                            draw_ey,
                            draw_sx,
                            draw_sy,
                            &obj.style,
                            arrow_len_px.0,
                            arrow_wid_px.0,
                        ) {
                            svg_children.push(SvgNode::Polygon(arrowhead));
                        }
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
                    let line_path_data = format!(
                        "M{},{}L{},{}",
                        fmt_num(line_sx),
                        fmt_num(line_sy),
                        fmt_num(line_ex),
                        fmt_num(line_ey)
                    );
                    let line_path = Path {
                        d: Some(PathData::parse(&line_path_data).unwrap()),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        stroke_dasharray: None,
                        style: svg_style,
                    };
                    svg_children.push(SvgNode::Path(line_path));
                } else {
                    // Multi-segment polyline - chop first and last segments
                    // For polylines, use fixed chop amount (circle radius) since we don't
                    // have endpoint detection for multi-segment paths yet
                    let mut points = obj.waypoints.clone();
                    if obj.style.chop && points.len() >= 2 {
                        let chop_amount_px = scaler.px(defaults::CIRCLE_RADIUS);
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
                                    "{}{},{}",
                                    cmd,
                                    fmt_num(scaler.px(p.x + offset_x)),
                                    fmt_num(scaler.px(p.y + offset_y))
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        // Closed polygon - add Z to close path
                        let path_data = format!("{}Z", path_str);
                        let path = Path {
                            d: Some(PathData::parse(&path_data).unwrap()),
                            fill: None,
                            stroke: None,
                            stroke_width: None,
                            stroke_dasharray: None,
                            style: svg_style,
                        };
                        svg_children.push(SvgNode::Path(path));
                    } else {
                        // TODO: Render arrowheads for polylines

                        // Chop endpoints for arrow heads (arrowht/2)
                        if (obj.style.arrow_end || obj.style.arrow_start) && points.len() >= 2 {
                            let chop_amount_px = arrow_len_px.0 / 2.0;

                            // Chop start if arrow_start
                            if obj.style.arrow_start {
                                let p0 = points[0];
                                let p1 = points[1];
                                let (new_x, new_y, _, _) =
                                    chop_line(p0.x.0, p0.y.0, p1.x.0, p1.y.0, chop_amount_px);
                                points[0] = Point::new(Inches(new_x), Inches(new_y));
                            }

                            // Chop end if arrow_end
                            if obj.style.arrow_end {
                                let n = points.len();
                                let pn1 = points[n - 2];
                                let pn = points[n - 1];
                                let (_, _, new_x, new_y) =
                                    chop_line(pn1.x.0, pn1.y.0, pn.x.0, pn.y.0, chop_amount_px);
                                points[n - 1] = Point::new(Inches(new_x), Inches(new_y));
                            }
                        }

                        // Now render the polyline
                        let path_str: String = points
                            .iter()
                            .enumerate()
                            .map(|(i, p)| {
                                let cmd = if i == 0 { "M" } else { "L" };
                                format!(
                                    "{}{},{}",
                                    cmd,
                                    fmt_num(scaler.px(p.x + offset_x)),
                                    fmt_num(scaler.px(p.y + offset_y))
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        let path = Path {
                            d: Some(PathData::parse(&path_str).unwrap()),
                            fill: None,
                            stroke: None,
                            stroke_width: None,
                            stroke_dasharray: None,
                            style: svg_style,
                        };
                        svg_children.push(SvgNode::Path(path));
                    }
                }
            }
            ObjectClass::Spline => {
                // Proper spline rendering with Bezier curves
                let spline_path_data = create_spline_path(&obj.waypoints, offset_x, offset_y);

                // Render arrowheads first (before path, like C pikchr)
                let n = obj.waypoints.len();
                if obj.style.arrow_end && n >= 2 {
                    let p1 = obj.waypoints[n - 2];
                    let p2 = obj.waypoints[n - 1];
                    if let Some(arrowhead) = render_arrowhead_dom(
                        p1.x.0 + offset_x.0,
                        p1.y.0 + offset_y.0,
                        p2.x.0 + offset_x.0,
                        p2.y.0 + offset_y.0,
                        &obj.style,
                        arrow_len_px.0,
                        arrow_wid_px.0,
                    ) {
                        svg_children.push(SvgNode::Polygon(arrowhead));
                    }
                }
                if obj.style.arrow_start && n >= 2 {
                    let p1 = obj.waypoints[0];
                    let p2 = obj.waypoints[1];
                    if let Some(arrowhead) = render_arrowhead_dom(
                        p2.x.0 + offset_x.0,
                        p2.y.0 + offset_y.0,
                        p1.x.0 + offset_x.0,
                        p1.y.0 + offset_y.0,
                        &obj.style,
                        arrow_len_px.0,
                        arrow_wid_px.0,
                    ) {
                        svg_children.push(SvgNode::Polygon(arrowhead));
                    }
                }

                let spline_path = Path {
                    d: Some(spline_path_data),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style,
                };
                svg_children.push(SvgNode::Path(spline_path));
            }
            ObjectClass::Move => {
                // Move: simple line without arrowheads
                let move_path_data = PathData::new().m(sx, sy).l(ex, ey);
                let move_path = Path {
                    d: Some(move_path_data),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style,
                };
                svg_children.push(SvgNode::Path(move_path));
            }

            ObjectClass::Arc => {
                // Proper arc rendering using curved arc paths
                let radius = scaler.px(obj.width / 2.0); // Use width as arc radius
                let arc_path_data = create_arc_path(sx, sy, ex, ey, radius);

                // Render arrowheads first (before path, like C pikchr)
                if obj.style.arrow_end {
                    if let Some(arrowhead) = render_arrowhead_dom(
                        sx,
                        sy,
                        ex,
                        ey,
                        &obj.style,
                        arrow_len_px.0,
                        arrow_wid_px.0,
                    ) {
                        svg_children.push(SvgNode::Polygon(arrowhead));
                    }
                }
                if obj.style.arrow_start {
                    if let Some(arrowhead) = render_arrowhead_dom(
                        ex,
                        ey,
                        sx,
                        sy,
                        &obj.style,
                        arrow_len_px.0,
                        arrow_wid_px.0,
                    ) {
                        svg_children.push(SvgNode::Polygon(arrowhead));
                    }
                }

                let arc_path = Path {
                    d: Some(arc_path_data),
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                    stroke_dasharray: None,
                    style: svg_style,
                };
                svg_children.push(SvgNode::Path(arc_path));
            }
            ObjectClass::Text => {
                // TODO: Implement text rendering with DOM
                for positioned_text in &obj.text {
                    let text_element = Text {
                        x: Some(tx),
                        y: Some(ty),
                        fill: None,
                        stroke: None,
                        stroke_width: None,
                        style: SvgStyle::default(),
                        text_anchor: Some("middle".to_string()),
                        dominant_baseline: Some("central".to_string()),
                        content: positioned_text.value.clone(),
                    };
                    svg_children.push(SvgNode::Text(text_element));
                }
            }

            ObjectClass::Sublist => {
                // Render sublist children with offset
                for child in &obj.children {
                    let child_tx = scaler.px(child.center.x + offset_x);
                    let child_ty = scaler.px(child.center.y + offset_y);
                    let child_svg_style = create_svg_style(&child.style, &scaler, dashwid);

                    match child.class {
                        ObjectClass::Box => {
                            let x1 = child_tx - scaler.px(child.width / 2.0);
                            let x2 = child_tx + scaler.px(child.width / 2.0);
                            let y1 = child_ty - scaler.px(child.height / 2.0);
                            let y2 = child_ty + scaler.px(child.height / 2.0);

                            let path_data = format!(
                                "M{},{}L{},{}L{},{}L{},{}Z",
                                fmt_num(x1),
                                fmt_num(y2),
                                fmt_num(x2),
                                fmt_num(y2),
                                fmt_num(x2),
                                fmt_num(y1),
                                fmt_num(x1),
                                fmt_num(y1)
                            );
                            let path = Path {
                                d: Some(PathData::parse(&path_data).unwrap()),
                                fill: None,
                                stroke: None,
                                stroke_width: None,
                                stroke_dasharray: None,
                                style: child_svg_style,
                            };
                            svg_children.push(SvgNode::Path(path));
                        }
                        ObjectClass::Circle => {
                            let r = scaler.px(child.width / 2.0);
                            let circle = Circle {
                                cx: Some(child_tx),
                                cy: Some(child_ty),
                                r: Some(r),
                                fill: None,
                                stroke: None,
                                stroke_width: None,
                                stroke_dasharray: None,
                                style: child_svg_style.clone(),
                            };
                            svg_children.push(SvgNode::Circle(circle));
                        }
                        _ => {
                            // For other shapes, just render as a simple circle placeholder
                            let r = scaler.px(child.width / 4.0);
                            let circle = Circle {
                                cx: Some(child_tx),
                                cy: Some(child_ty),
                                r: Some(r),
                                fill: None,
                                stroke: None,
                                stroke_width: None,
                                stroke_dasharray: None,
                                style: child_svg_style.clone(),
                            };
                            svg_children.push(SvgNode::Circle(circle));
                        }
                    }
                }
            }
        }

        // Render text labels inside objects
        if obj.class != ObjectClass::Text && !obj.text.is_empty() {
            for positioned_text in &obj.text {
                let text_element = Text {
                    x: Some(tx),
                    y: Some(ty),
                    fill: Some("rgb(0,0,0)".to_string()),
                    stroke: None,
                    stroke_width: None,
                    style: SvgStyle::default(),
                    text_anchor: Some("middle".to_string()),
                    dominant_baseline: Some("central".to_string()),
                    content: positioned_text.value.clone(),
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

    // Pikchr auto-chop semantics:
    // - If explicit "chop" attribute is set: chop both endpoints
    // - If line connects two objects (both attachments): chop both endpoints
    // - If line has only end attachment (to Object): chop end only
    // - If line has only start attachment (from Object): do NOT chop start
    let has_explicit_chop = obj.style.chop;
    let has_both_attachments = obj.start_attachment.is_some() && obj.end_attachment.is_some();
    let should_chop_start = has_explicit_chop || has_both_attachments;
    let should_chop_end = obj.end_attachment.is_some(); // Always chop end if attached

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
    if should_chop_start {
        if let Some(ref start_info) = obj.start_attachment {
            // Chop against start object, toward the end object's center
            if let Some(chopped) =
                chop_against_endpoint(scaler, start_info, end_center_px, offset_x, offset_y)
            {
                new_start = chopped;
            }
        }
    }

    let mut new_end = (ex, ey);
    if should_chop_end {
        if let Some(ref end_info) = obj.end_attachment {
            // Chop against end object, toward the start object's center
            if let Some(chopped) =
                chop_against_endpoint(scaler, end_info, start_center_px, offset_x, offset_y)
            {
                new_end = chopped;
            }
        }
    }

    (new_start.0, new_start.1, new_end.0, new_end.1)
}

/// Compass points for discrete attachment like C pikchr
#[derive(Debug, Clone, Copy)]
enum CompassPoint {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

/// Chop against box using discrete compass points like C pikchr
fn chop_against_box_compass_point(
    cx: f64,
    cy: f64,
    half_w: f64,
    half_h: f64,
    corner_radius: f64,
    toward: (f64, f64),
) -> Option<(f64, f64)> {
    if half_w <= 0.0 || half_h <= 0.0 {
        return None;
    }

    // Calculate direction from box center to target point
    // C pikchr scales dx by h/w to normalize the box to a square for angle calculations
    let dx = (toward.0 - cx) * half_h / half_w;
    // SVG Y increases downward; flip dy for compass math so 0° = north like C pikchr
    let dy = -(toward.1 - cy);

    // C pikchr logic: determine compass point based on angle
    // Uses slope thresholds to divide 360 degrees into 8 sectors
    let compass_point = if dx > 0.0 {
        if dy >= 2.414 * dx {
            CompassPoint::North // > 67.5 degrees
        } else if dy > 0.414 * dx {
            CompassPoint::NorthEast // 22.5 to 67.5 degrees
        } else if dy > -0.414 * dx {
            CompassPoint::East // -22.5 to 22.5 degrees
        } else if dy > -2.414 * dx {
            CompassPoint::SouthEast // -67.5 to -22.5 degrees
        } else {
            CompassPoint::South // < -67.5 degrees
        }
    } else if dx < 0.0 {
        if dy >= -2.414 * dx {
            CompassPoint::North // > 67.5 degrees
        } else if dy > -0.414 * dx {
            CompassPoint::NorthWest // 22.5 to 67.5 degrees
        } else if dy > 0.414 * dx {
            CompassPoint::West // -22.5 to 22.5 degrees
        } else if dy > 2.414 * dx {
            CompassPoint::SouthWest // -67.5 to -22.5 degrees
        } else {
            CompassPoint::South // < -67.5 degrees
        }
    } else {
        // dx == 0, vertical line
        if dy >= 0.0 {
            CompassPoint::North
        } else {
            CompassPoint::South
        }
    };

    // Calculate corner inset for rounded corners
    // This is (1 - cos(45°)) * rad = (1 - 1/√2) * rad ≈ 0.29289 * rad
    // Matches C pikchr's boxOffset function
    let rad = corner_radius.min(half_w).min(half_h);
    let rx = if rad > 0.0 {
        0.29289321881345252392 * rad
    } else {
        0.0
    };

    // Return coordinates of the specific compass point
    // For diagonal points, adjust inward by rx to account for rounded corners
    let result = match compass_point {
        CompassPoint::North => (cx, cy - half_h),
        CompassPoint::NorthEast => (cx + half_w - rx, cy - half_h + rx),
        CompassPoint::East => (cx + half_w, cy),
        CompassPoint::SouthEast => (cx + half_w - rx, cy + half_h - rx),
        CompassPoint::South => (cx, cy + half_h),
        CompassPoint::SouthWest => (cx - half_w + rx, cy + half_h - rx),
        CompassPoint::West => (cx - half_w, cy),
        CompassPoint::NorthWest => (cx - half_w + rx, cy - half_h + rx),
    };

    Some(result)
}

fn chop_against_endpoint(
    scaler: &Scaler,
    endpoint: &EndpointObject,
    toward: (f64, f64),
    offset_x: Inches,
    offset_y: Inches,
) -> Option<(f64, f64)> {
    let cx = scaler.px(endpoint.center.x + offset_x);
    let cy = scaler.px(endpoint.center.y + offset_y);
    let half_w = scaler.px(endpoint.width / 2.0);
    let half_h = scaler.px(endpoint.height / 2.0);
    let corner_radius = scaler.px(endpoint.corner_radius);

    match endpoint.class {
        ObjectClass::Circle | ObjectClass::Ellipse | ObjectClass::Oval | ObjectClass::Cylinder => {
            chop_against_ellipse(cx, cy, half_w, half_h, toward)
        }
        ObjectClass::Box | ObjectClass::File => {
            chop_against_box_compass_point(cx, cy, half_w, half_h, corner_radius, toward)
        }
        ObjectClass::Diamond => chop_against_diamond(cx, cy, half_w, half_h, toward),
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

/// Find the intersection of a ray from box center toward a target point with the box edge.
/// Returns the point on the box boundary closest to the target.
fn chop_against_box(
    cx: f64,
    cy: f64,
    half_w: f64,
    half_h: f64,
    toward: (f64, f64),
) -> Option<(f64, f64)> {
    if half_w <= 0.0 || half_h <= 0.0 {
        return None;
    }

    let dx = toward.0 - cx;
    let dy = toward.1 - cy;
    if dx.abs() < f64::EPSILON && dy.abs() < f64::EPSILON {
        return None;
    }

    // Find t values for intersection with each edge
    // Ray: P = center + t * (toward - center)
    // We want the smallest positive t that intersects an edge

    let mut t_min = f64::INFINITY;

    // Right edge: x = cx + half_w
    if dx.abs() > f64::EPSILON {
        let t = half_w / dx.abs();
        if t > 0.0 && t < t_min {
            let y_at_t = dy * t;
            if y_at_t.abs() <= half_h {
                t_min = t;
            }
        }
    }

    // Top/bottom edge: y = cy +/- half_h
    if dy.abs() > f64::EPSILON {
        let t = half_h / dy.abs();
        if t > 0.0 && t < t_min {
            let x_at_t = dx * t;
            if x_at_t.abs() <= half_w {
                t_min = t;
            }
        }
    }

    if t_min.is_finite() {
        Some((cx + dx * t_min, cy + dy * t_min))
    } else {
        None
    }
}

/// Find the intersection of a ray from diamond center toward a target point with the diamond edge.
/// A diamond is a rotated square (rhombus) with vertices at (cx±half_w, cy) and (cx, cy±half_h).
fn chop_against_diamond(
    cx: f64,
    cy: f64,
    half_w: f64,
    half_h: f64,
    toward: (f64, f64),
) -> Option<(f64, f64)> {
    if half_w <= 0.0 || half_h <= 0.0 {
        return None;
    }

    let dx = toward.0 - cx;
    let dy = toward.1 - cy;
    if dx.abs() < f64::EPSILON && dy.abs() < f64::EPSILON {
        return None;
    }

    // Diamond edges are defined by: |x - cx|/half_w + |y - cy|/half_h = 1
    // For a ray from center: P = (cx + t*dx, cy + t*dy)
    // Substituting: |t*dx|/half_w + |t*dy|/half_h = 1
    // t * (|dx|/half_w + |dy|/half_h) = 1
    // t = 1 / (|dx|/half_w + |dy|/half_h)

    let denom = dx.abs() / half_w + dy.abs() / half_h;
    if denom <= 0.0 {
        return None;
    }

    let t = 1.0 / denom;
    Some((cx + dx * t, cy + dy * t))
}

/// Render an oval (pill shape)
/// Render a rounded box as a path (matching C pikchr output)
/// Create a rounded box path using PathData fluent API (matching C pikchr output)
fn create_rounded_box_path(x1: f64, y1: f64, x2: f64, y2: f64, r: f64) -> PathData {
    // C pikchr path format for rounded box:
    // Start at bottom-left corner (after radius), go clockwise
    PathData::new()
        .m(x1 + r, y2) // M: start bottom-left after radius
        .l(x2 - r, y2) // L: line to bottom-right before radius
        .a(r, r, 0.0, false, false, x2, y2 - r) // A: arc to right edge
        .l(x2, y1 + r) // L: line up to top-right before radius
        .a(r, r, 0.0, false, false, x2 - r, y1) // A: arc to top edge
        .l(x1 + r, y1) // L: line left to top-left after radius
        .a(r, r, 0.0, false, false, x1, y1 + r) // A: arc to left edge
        .l(x1, y2 - r) // L: line down to bottom-left before radius
        .a(r, r, 0.0, false, false, x1 + r, y2) // A: arc back to start
        .z() // Z: close path
}

/// Create cylinder paths using PathData fluent API (matching C pikchr output)
fn create_cylinder_paths(cx: f64, cy: f64, width: f64, height: f64) -> (PathData, PathData) {
    let rx = width / 2.0;
    let ry = width / 8.0; // Ellipse vertical radius
    let top_y = cy - height / 2.0 + ry;
    let bottom_y = cy + height / 2.0 - ry;

    // Body path: left side down, bottom arc, right side up
    let body_path = PathData::new()
        .m(cx - rx, top_y) // Start at top-left
        .l(cx - rx, bottom_y) // Line down to bottom-left
        .a(rx, ry, 0.0, false, false, cx + rx, bottom_y) // Arc to bottom-right
        .l(cx + rx, top_y); // Line up to top-right

    // Bottom ellipse arc (only the front half, as a visible edge)
    let bottom_arc_path = PathData::new()
        .m(cx - rx, bottom_y) // Start at left of bottom ellipse
        .a(rx, ry, 0.0, false, false, cx + rx, bottom_y); // Arc to right

    (body_path, bottom_arc_path)
}

/// Create file paths using PathData fluent API (matching C pikchr output)
fn create_file_paths(cx: f64, cy: f64, width: f64, height: f64) -> (PathData, PathData) {
    let fold_size = width.min(height) * 0.2; // Fold is 20% of smaller dimension
    let x = cx - width / 2.0;
    let y = cy - height / 2.0;
    let right = cx + width / 2.0;
    let bottom = cy + height / 2.0;

    // Main outline path (going clockwise from top-left)
    let main_path = PathData::new()
        .m(x, y) // Top-left
        .l(right - fold_size, y) // Top-right minus fold
        .l(right, y + fold_size) // Fold corner (diagonal)
        .l(right, bottom) // Bottom-right
        .l(x, bottom) // Bottom-left
        .z(); // Close path

    // Fold line path (the crease)
    let fold_path = PathData::new()
        .m(right - fold_size, y) // Start at corner
        .l(right - fold_size, y + fold_size) // Down to fold
        .l(right, y + fold_size); // Across to edge

    (main_path, fold_path)
}

/// Create spline path using PathData fluent API (matching C pikchr output)
fn create_spline_path(waypoints: &[PointIn], offset_x: Inches, offset_y: Inches) -> PathData {
    if waypoints.is_empty() {
        return PathData::new();
    }

    // Convert waypoints to offset coordinates
    let points: Vec<(f64, f64)> = waypoints
        .iter()
        .map(|p| (p.x.0 + offset_x.0, p.y.0 + offset_y.0))
        .collect();

    let mut path = PathData::new().m(points[0].0, points[0].1);

    if points.len() == 2 {
        // Just a line
        path = path.l(points[1].0, points[1].1);
    } else {
        // Use quadratic bezier curves for smoothness
        // For each segment, use the midpoint as control point
        for i in 1..points.len() {
            let prev = points[i - 1];
            let curr = points[i];

            if i == 1 {
                // First segment: quadratic from start
                let mid = ((prev.0 + curr.0) / 2.0, (prev.1 + curr.1) / 2.0);
                path = path.q(prev.0, prev.1, mid.0, mid.1);
            }

            if i < points.len() - 1 {
                // Middle segments: curve through midpoints
                let next = points[i + 1];
                let mid = ((curr.0 + next.0) / 2.0, (curr.1 + next.1) / 2.0);
                path = path.q(curr.0, curr.1, mid.0, mid.1);
            } else {
                // Last segment: end at the final point
                path = path.q(
                    (prev.0 + curr.0) / 2.0,
                    (prev.1 + curr.1) / 2.0,
                    curr.0,
                    curr.1,
                );
            }
        }
    }

    path
}

/// Create arc path using PathData fluent API (matching C pikchr output)
fn create_arc_path(sx: f64, sy: f64, ex: f64, ey: f64, radius: f64) -> PathData {
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
    let sweep = true; // Default clockwise
    let large_arc = false;

    PathData::new()
        .m(sx, sy) // Move to start point
        .a(r, r, 0.0, large_arc, sweep, ex, ey) // Arc to end point
}

/// Convert a color name to rgb() format like C pikchr
fn color_to_rgb(color: &str) -> String {
    match color.to_lowercase().as_str() {
        "black" => "rgb(0,0,0)".to_string(),
        "white" => "rgb(255,255,255)".to_string(),
        "red" => "rgb(255,0,0)".to_string(),
        "green" => "rgb(0,128,0)".to_string(),
        "blue" => "rgb(0,0,255)".to_string(),
        "yellow" => "rgb(255,255,0)".to_string(),
        "cyan" => "rgb(0,255,255)".to_string(),
        "magenta" => "rgb(255,0,255)".to_string(),
        "gray" | "grey" => "rgb(128,128,128)".to_string(),
        "lightgray" | "lightgrey" => "rgb(211,211,211)".to_string(),
        "darkgray" | "darkgrey" => "rgb(169,169,169)".to_string(),
        "orange" => "rgb(255,165,0)".to_string(),
        "pink" => "rgb(255,192,203)".to_string(),
        "purple" => "rgb(128,0,128)".to_string(),
        "none" => "none".to_string(),
        _ => color.to_string(), // Pass through hex colors or unknown
    }
}

fn create_svg_style(style: &ObjectStyle, scaler: &Scaler, dashwid: Inches) -> SvgStyle {
    // Build SvgStyle for DOM-based generation
    let fill = Color::parse(&style.fill);
    let stroke = Color::parse(&style.stroke);
    let stroke_width_val = scaler.len(style.stroke_width).0;

    let stroke_dasharray = if style.dashed {
        // C pikchr: stroke-dasharray: dashwid, dashwid
        let dash = scaler.len(dashwid).0;
        Some((dash, dash))
    } else if style.dotted {
        // C pikchr: stroke-dasharray: stroke_width, dashwid
        let sw = stroke_width_val.max(2.1); // min 2.1 per C source
        let dash = scaler.len(dashwid).0;
        Some((sw, dash))
    } else {
        None
    };

    let mut svg_style = SvgStyle::new();
    svg_style
        .properties
        .insert("fill".to_string(), fill.to_string());
    svg_style
        .properties
        .insert("stroke".to_string(), stroke.to_string());
    svg_style
        .properties
        .insert("stroke-width".to_string(), fmt_num(stroke_width_val));
    if let Some((a, b)) = stroke_dasharray {
        svg_style
            .properties
            .insert("stroke-dasharray".to_string(), format!("{:.2},{:.2}", a, b));
    }

    svg_style
}

/// Render an arrowhead polygon at the end of a line
/// The arrowhead points in the direction from (sx, sy) to (ex, ey)
fn render_arrowhead_dom(
    sx: f64,
    sy: f64,
    ex: f64,
    ey: f64,
    style: &ObjectStyle,
    arrow_len: f64,
    arrow_width: f64,
) -> Option<Polygon> {
    // Calculate direction vector
    let dx = ex - sx;
    let dy = ey - sy;
    let len = (dx * dx + dy * dy).sqrt();

    if len < 0.001 {
        return None; // Zero-length line, no arrowhead
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

    let points = format!(
        "{},{} {},{} {},{}",
        fmt_num(ex),
        fmt_num(ey),
        fmt_num(p1_x),
        fmt_num(p1_y),
        fmt_num(p2_x),
        fmt_num(p2_y)
    );

    let fill_color = Color::parse(&style.stroke);

    Some(Polygon {
        points: Some(points),
        fill: None,
        stroke: None,
        stroke_width: None,
        stroke_dasharray: None,
        style: SvgStyle::new().add("fill", &fill_color.to_string()),
    })
}

/// Legacy string-based arrowhead rendering
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
        r#"  <polygon points="{},{} {},{} {},{}" style="fill:{}"/>"#,
        fmt_num(ex),
        fmt_num(ey),
        fmt_num(p1_x),
        fmt_num(p1_y),
        fmt_num(p2_x),
        fmt_num(p2_y),
        color_to_rgb(&style.stroke)
    )
    .unwrap();
}

#[cfg(test)]
mod fmt_tests {
    use super::fmt_num;

    #[test]
    fn test_fmt_num() {
        assert_eq!(fmt_num(38.16), "38.16");
        assert_eq!(fmt_num(402.528), "402.528");
        assert_eq!(fmt_num(63.6158), "63.6158");
        assert_eq!(fmt_num(2.16), "2.16");
        assert_eq!(fmt_num(0.0), "0");
        assert_eq!(fmt_num(100.0), "100");
    }
}
