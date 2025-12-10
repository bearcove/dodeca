# Pikru Type System Design

## What is pikchr fundamentally?

Pikchr takes a textual description and produces an SVG. The core concepts are:

1. **Shapes** - things you can draw (box, circle, line, etc.)
2. **Positions** - where things are located
3. **Connections** - how shapes relate to each other (lines between them)
4. **Style** - how things look (color, stroke, fill)

## The Coordinate System

Everything happens in a 2D plane measured in **inches**. But we need to distinguish:

```rust
/// An absolute position in the diagram
struct Pos2 {
    x: Inches,
    y: Inches,
}

/// A relative displacement (offset/delta)
struct Vec2 {
    x: Inches,
    y: Inches,
}

/// A size (always non-negative)
struct Size2 {
    w: Inches,
    h: Inches,
}
```

Why separate `Pos2` and `Vec2`?
- `Pos2 + Vec2 = Pos2` (move a position by an offset)
- `Pos2 - Pos2 = Vec2` (displacement between two positions)
- `Vec2 + Vec2 = Vec2` (combine offsets)
- `Pos2 + Pos2 = ???` (nonsensical - you can't add positions!)

This is the same distinction glam makes with `Vec2` vs `Affine2`, or game engines make with points vs vectors.

## Shapes: What do they have in common?

Every shape has:
- A **center** position
- A **bounding box** (for layout and hit testing)
- **Edge points** (where lines can attach: north, south, east, west, corners)
- **Style** (stroke, fill, etc.)
- Optional **text labels**

But shapes differ in their *intrinsic properties*:
- Circle: center + radius
- Box: center + size + corner_radius
- Line: waypoints (a path through space)
- Oval: center + size (radius derived from size)

## The Shape Enum

Rather than a trait with `Box<dyn Shape>`, an enum is simpler and faster:

```rust
enum Shape {
    Box(BoxShape),
    Circle(CircleShape),
    Ellipse(EllipseShape),
    Oval(OvalShape),
    Diamond(DiamondShape),
    Cylinder(CylinderShape),
    File(FileShape),
    Line(LineShape),
    Arc(ArcShape),
    Spline(SplineShape),
    Dot(DotShape),
    Text(TextShape),
}
```

Each variant holds exactly the data it needs - no more wasted fields.

## Closed Shapes vs Open Shapes (Paths)

There's a natural split:

**Closed shapes** have a center and a boundary:
- Box, Circle, Ellipse, Oval, Diamond, Cylinder, File, Dot

**Open shapes** are paths through space:
- Line, Arc, Spline

```rust
/// A path is a sequence of points
struct Path {
    points: Vec<Pos2>,
}

impl Path {
    fn start(&self) -> Pos2 { ... }
    fn end(&self) -> Pos2 { ... }
    fn length(&self) -> Inches { ... }
}
```

Lines, arcs, and splines are all paths - they just render differently (straight segments, circular arc, bezier curves).

## Edge Points

Shapes have named attachment points:

```rust
enum Edge {
    // Cardinal
    North, South, East, West,
    // Corners
    NorthEast, NorthWest, SouthEast, SouthWest,
    // Special
    Center,
    Start,  // For paths
    End,    // For paths
}
```

For closed shapes, edge points are on the boundary.
For paths, `Start` and `End` are the endpoints.

## Style

Style is mostly orthogonal to shape:

```rust
struct Style {
    stroke: Color,
    fill: Color,
    stroke_width: Inches,
    line_style: LineStyle,  // Solid, Dashed, Dotted
    visibility: Visibility, // Visible, Invisible
}

enum LineStyle {
    Solid,
    Dashed { width: Inches },
    Dotted { width: Inches },
}
```

But some properties are shape-specific:
- `corner_radius` only makes sense for Box
- `arrow_start`/`arrow_end` only make sense for paths

## Arrows

Arrows are a property of paths, not a separate shape:

```rust
struct PathStyle {
    base: Style,
    arrow_start: bool,
    arrow_end: bool,
    close: bool,  // For polygons
}
```

An "Arrow" in pikchr is just a Line with `arrow_end: true`.

## Text

Text can appear:
1. As labels on shapes (positioned relative to the shape)
2. As standalone text objects

```rust
struct TextSpan {
    content: String,
    position: TextPosition,
    style: TextStyle,
}

enum TextPosition {
    Above,
    Below,
    Center,
    // ...
}

struct TextStyle {
    bold: bool,
    italic: bool,
    mono: bool,
    size: TextSize,  // Big, Normal, Small
    align: TextAlign, // Left, Center, Right
}
```

## The Rendered Object

After evaluation, we have fully-resolved shapes:

```rust
struct Object {
    name: Option<String>,  // Label for referencing
    shape: Shape,
    style: Style,
    text: Vec<TextSpan>,
}
```

The `Shape` enum holds the geometry. `Style` holds the appearance. `text` holds labels.

## Connections and Attachments

When a line connects to a shape, we need to know:
- Which shape it connects to
- Which edge point
- Whether to "chop" (stop at the boundary, not the center)

```rust
struct Attachment {
    target: ObjectRef,  // Which object
    edge: Edge,         // Which edge point
}

/// Reference to an object (by name or index)
enum ObjectRef {
    Named(String),
    Index(usize),
    Last,
    LastOfClass(ShapeClass),
}
```

## The Diagram

The top-level structure:

```rust
struct Diagram {
    objects: Vec<Object>,
    bounds: BBox,
}

impl Diagram {
    fn render_svg(&self, scale: f64) -> String { ... }
}
```

## Rendering Parameters

When rendering to SVG, we need:

```rust
struct RenderParams {
    scale: f64,           // inches to pixels (typically 144)
    offset: Vec2,         // translation
    arrow_size: Size2,    // arrowhead dimensions
    dash_width: Inches,   // for dashed lines
}
```

This could be a method parameter or bundled into a context.

## Summary of Key Types

**Geometry:**
- `Inches` - the unit of measurement
- `Pos2` - absolute position
- `Vec2` - relative offset
- `Size2` - width × height
- `BBox` - bounding box (min + max corners)

**Shapes:**
- `Shape` enum with variants for each shape type
- Each variant holds its specific geometry
- `Path` for line-like shapes (sequence of points)

**Appearance:**
- `Style` - stroke, fill, line style
- `Color` - named or RGB
- `TextSpan` - text with position and style

**Structure:**
- `Object` - shape + style + text + name
- `Diagram` - collection of objects
- `Attachment` - connection between objects

**Rendering:**
- `RenderParams` - scale, offset, etc.
- `Scaler` - converts inches to pixels

## Expressions and Evaluation

Pikchr has a full expression language. Expressions appear everywhere:
- Dimensions: `box width 2cm`
- Positions: `at (A.x + 0.5, B.y)`
- Colors: `fill 0xff0000`
- Conditions: `assert A.x == B.x`

### The Expression AST

```rust
enum Expr {
    // Literals
    Number(f64),              // Already in inches (parser converts units)

    // References
    Variable(String),         // User-defined variable
    Builtin(Builtin),         // fill, color, thickness

    // Object property access
    Property(ObjectRef, Property),      // A.width, A.height
    Coordinate(ObjectRef, Axis),        // A.x, A.y
    EdgeCoord(ObjectRef, Edge, Axis),   // A.n.x, A.ne.y

    // Operations
    Binary(Box<Expr>, BinOp, Box<Expr>),
    Unary(UnaryOp, Box<Expr>),

    // Functions
    Call(Func, Vec<Expr>),

    // Special
    Distance(Box<Position>, Box<Position>),  // distance(A, B)
}

enum BinOp { Add, Sub, Mul, Div }
enum UnaryOp { Neg, Pos }
enum Func { Abs, Sin, Cos, Sqrt, Min, Max, Int }
enum Axis { X, Y }
enum Builtin { Fill, Color, Thickness }
```

### Evaluated Values

Expressions evaluate to typed values:

```rust
enum Value {
    Length(Inches),    // Distances, dimensions
    Scalar(f64),       // Unitless numbers, percentages
    Position(Pos2),    // 2D coordinates
    Color(Color),      // Colors
}
```

Why typed values?
- `1 + 2` = `Scalar(3)`
- `1cm + 2cm` = `Length(3cm)`
- `A.center` = `Position(...)`
- Prevents nonsense like `1cm + red`

### The Evaluation Context

Evaluation needs context:

```rust
struct EvalContext {
    // Named objects we can reference
    objects: HashMap<String, Object>,

    // Object list for ordinal references ("2nd box")
    object_list: Vec<Object>,

    // User-defined variables
    variables: HashMap<String, Value>,

    // Current "this" object being constructed
    current: Option<Object>,

    // Global settings
    direction: Direction,

    // Builtin defaults (thickness, fill, color)
    defaults: Defaults,
}

impl EvalContext {
    fn eval(&self, expr: &Expr) -> Result<Value, EvalError> {
        match expr {
            Expr::Number(n) => Ok(Value::Length(Inches(*n))),
            Expr::Variable(name) => self.lookup_variable(name),
            Expr::Property(obj, prop) => {
                let obj = self.resolve_object(obj)?;
                obj.get_property(prop)
            }
            Expr::Binary(lhs, op, rhs) => {
                let lhs = self.eval(lhs)?;
                let rhs = self.eval(rhs)?;
                self.apply_binop(lhs, *op, rhs)
            }
            // ...
        }
    }
}
```

### Position Expressions

Positions are more complex than simple expressions:

```rust
enum PositionExpr {
    // Literal coordinates
    Coords(Expr, Expr),           // (x, y)

    // Object references
    Object(ObjectRef),            // A (center)
    Edge(ObjectRef, Edge),        // A.north

    // Relative positions
    Offset(Box<PositionExpr>, Vec2Expr),     // A + (1, 2)
    Between(Expr, Box<PositionExpr>, Box<PositionExpr>),  // 0.5 between A and B

    // Directional
    Above(Expr, Box<PositionExpr>),    // 1cm above B
    Below(Expr, Box<PositionExpr>),
    LeftOf(Expr, Box<PositionExpr>),
    RightOf(Expr, Box<PositionExpr>),
    Heading(Expr, Angle, Box<PositionExpr>),  // 1cm heading 45 from B
}

impl EvalContext {
    fn eval_position(&self, pos: &PositionExpr) -> Result<Pos2, EvalError> {
        match pos {
            PositionExpr::Coords(x, y) => {
                let x = self.eval(x)?.as_length()?;
                let y = self.eval(y)?.as_length()?;
                Ok(Pos2 { x, y })
            }
            PositionExpr::Object(obj_ref) => {
                let obj = self.resolve_object(obj_ref)?;
                Ok(obj.center())
            }
            PositionExpr::Between(t, a, b) => {
                let t = self.eval(t)?.as_scalar()?;
                let a = self.eval_position(a)?;
                let b = self.eval_position(b)?;
                Ok(a.lerp(b, t))
            }
            // ...
        }
    }
}
```

### Object References

How do we refer to objects?

```rust
enum ObjectRef {
    // By name
    Named(String),                    // A, B, MyBox

    // By ordinal
    Nth(u32, Option<ShapeKind>),      // 2nd box, 3rd circle
    First(Option<ShapeKind>),         // first box
    Last(Option<ShapeKind>),          // last circle
    Previous,                         // previous (any)

    // Relative
    This,                             // Current object being built
}

impl EvalContext {
    fn resolve_object(&self, obj_ref: &ObjectRef) -> Result<&Object, EvalError> {
        match obj_ref {
            ObjectRef::Named(name) => {
                self.objects.get(name)
                    .ok_or(EvalError::UnknownObject(name.clone()))
            }
            ObjectRef::Nth(n, kind) => {
                self.object_list.iter()
                    .filter(|o| kind.map(|k| o.shape.kind() == k).unwrap_or(true))
                    .nth(*n as usize - 1)
                    .ok_or(EvalError::OrdinalOutOfRange(*n))
            }
            ObjectRef::Last(kind) => {
                self.object_list.iter().rev()
                    .find(|o| kind.map(|k| o.shape.kind() == k).unwrap_or(true))
                    .ok_or(EvalError::NoSuchObject)
            }
            ObjectRef::This => {
                self.current.as_ref()
                    .ok_or(EvalError::NoCurrentObject)
            }
            // ...
        }
    }
}
```

### Type Coercion

Some operations need type coercion:

```rust
impl Value {
    fn as_length(&self) -> Result<Inches, EvalError> {
        match self {
            Value::Length(l) => Ok(*l),
            Value::Scalar(s) => Ok(Inches(*s)),  // Treat scalar as inches
            _ => Err(EvalError::TypeMismatch),
        }
    }

    fn as_scalar(&self) -> Result<f64, EvalError> {
        match self {
            Value::Scalar(s) => Ok(*s),
            Value::Length(l) => Ok(l.raw()),  // Extract raw value
            _ => Err(EvalError::TypeMismatch),
        }
    }
}
```

### Binary Operation Type Rules

```rust
impl EvalContext {
    fn apply_binop(&self, lhs: Value, op: BinOp, rhs: Value) -> Result<Value, EvalError> {
        use Value::*;
        use BinOp::*;

        match (lhs, op, rhs) {
            // Length ± Length = Length
            (Length(a), Add, Length(b)) => Ok(Length(a + b)),
            (Length(a), Sub, Length(b)) => Ok(Length(a - b)),

            // Length × Scalar = Length
            (Length(a), Mul, Scalar(b)) => Ok(Length(a * b)),
            (Scalar(a), Mul, Length(b)) => Ok(Length(b * a)),

            // Length ÷ Scalar = Length
            (Length(a), Div, Scalar(b)) => Ok(Length(a / b)),

            // Length ÷ Length = Scalar (ratio)
            (Length(a), Div, Length(b)) => Ok(Scalar(a.raw() / b.raw())),

            // Scalar ± Scalar = Scalar
            (Scalar(a), Add, Scalar(b)) => Ok(Scalar(a + b)),
            (Scalar(a), Sub, Scalar(b)) => Ok(Scalar(a - b)),
            (Scalar(a), Mul, Scalar(b)) => Ok(Scalar(a * b)),
            (Scalar(a), Div, Scalar(b)) => Ok(Scalar(a / b)),

            // Position + Position = Error
            (Position(_), Add, Position(_)) => Err(EvalError::CannotAddPositions),

            // Position - Position = Vec2 (displacement)
            (Position(a), Sub, Position(b)) => Ok(Value::Vec2(a - b)),

            _ => Err(EvalError::TypeMismatch),
        }
    }
}
```

## Statements and Execution

Beyond expressions, there are statements:

```rust
enum Statement {
    // Object creation
    Object(ObjectDef),

    // Assignment
    Assign(String, Expr),

    // Direction change
    Direction(Direction),

    // Control
    Assert(Condition),
    Print(Vec<PrintArg>),

    // Macros
    Define(String, String),  // name, body
    MacroCall(String, Vec<Expr>),
}

struct ObjectDef {
    label: Option<String>,
    base: ShapeKind,
    attributes: Vec<Attribute>,
}
```

### The Execution Context

Executing a program builds up a diagram:

```rust
struct ExecContext {
    eval: EvalContext,
    diagram: Diagram,
    position: Pos2,      // "cursor" - where next object goes
    direction: Direction, // current movement direction
}

impl ExecContext {
    fn execute(&mut self, stmt: &Statement) -> Result<(), ExecError> {
        match stmt {
            Statement::Object(def) => {
                let obj = self.build_object(def)?;
                self.diagram.add(obj.clone());
                if let Some(name) = &obj.name {
                    self.eval.objects.insert(name.clone(), obj.clone());
                }
                self.position = obj.end();  // Move cursor
            }
            Statement::Assign(name, expr) => {
                let value = self.eval.eval(expr)?;
                self.eval.variables.insert(name.clone(), value);
            }
            Statement::Direction(dir) => {
                self.direction = *dir;
            }
            // ...
        }
        Ok(())
    }
}
```

## Parsing and Error Reporting

The parser should track source locations for beautiful error messages.

### Spans

```rust
/// A location in source code
#[derive(Clone, Copy, Debug)]
struct Span {
    /// Byte offset of start
    start: usize,
    /// Byte offset of end (exclusive)
    end: usize,
}

impl Span {
    fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}
```

### Spanned AST Nodes

Every AST node carries its source location:

```rust
/// A value with its source location
#[derive(Clone, Debug)]
struct Spanned<T> {
    node: T,
    span: Span,
}

impl<T> Spanned<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned { node: f(self.node), span: self.span }
    }
}

// The expression AST becomes:
type SpannedExpr = Spanned<Expr>;

enum Expr {
    Number(f64),
    Variable(String),
    Binary(Box<SpannedExpr>, BinOp, Box<SpannedExpr>),
    // ...
}
```

### Error Types with miette

Using miette for gorgeous error output:

```rust
use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

/// Convert our Span to miette's SourceSpan
impl From<Span> for SourceSpan {
    fn from(s: Span) -> Self {
        SourceSpan::new(s.start.into(), (s.end - s.start).into())
    }
}

#[derive(Error, Diagnostic, Debug)]
pub enum ParseError {
    #[error("unexpected token")]
    #[diagnostic(code(pikru::parse::unexpected_token))]
    UnexpectedToken {
        #[source_code]
        src: NamedSource<String>,
        #[label("found this")]
        span: SourceSpan,
        expected: String,
    },

    #[error("unterminated string")]
    #[diagnostic(code(pikru::parse::unterminated_string))]
    UnterminatedString {
        #[source_code]
        src: NamedSource<String>,
        #[label("string starts here")]
        span: SourceSpan,
    },
}

#[derive(Error, Diagnostic, Debug)]
pub enum EvalError {
    #[error("cannot add two positions")]
    #[diagnostic(
        code(pikru::eval::cannot_add_positions),
        help("use `pos - pos` to get displacement, or `pos + vec` to offset")
    )]
    CannotAddPositions {
        #[source_code]
        src: NamedSource<String>,
        #[label("this is a position")]
        lhs: SourceSpan,
        #[label("this is also a position")]
        rhs: SourceSpan,
    },

    #[error("unknown object '{name}'")]
    #[diagnostic(code(pikru::eval::unknown_object))]
    UnknownObject {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not found")]
        span: SourceSpan,
        #[help]
        suggestion: Option<String>,  // "did you mean 'Box1'?"
    },

    #[error("type mismatch: expected {expected}, got {got}")]
    #[diagnostic(code(pikru::eval::type_mismatch))]
    TypeMismatch {
        expected: &'static str,
        got: &'static str,
        #[source_code]
        src: NamedSource<String>,
        #[label("this expression")]
        span: SourceSpan,
    },

    #[error("division by zero")]
    #[diagnostic(code(pikru::eval::division_by_zero))]
    DivisionByZero {
        #[source_code]
        src: NamedSource<String>,
        #[label("divisor is zero")]
        span: SourceSpan,
    },
}
```

### What This Looks Like

```
Error: pikru::eval::cannot_add_positions

  × cannot add two positions
   ╭─[input.pikchr:3:12]
 3 │ box at A.center + B.center
   │        ────┬───   ────┬───
   │            │          ╰── this is also a position
   │            ╰── this is a position
   ╰────
  help: use `pos - pos` to get displacement, or `pos + vec` to offset
```

### The Source Context

Evaluation needs access to source for error reporting:

```rust
struct SourceContext {
    name: String,      // filename or "<input>"
    source: String,    // the full source text
}

impl SourceContext {
    fn named_source(&self) -> NamedSource<String> {
        NamedSource::new(&self.name, self.source.clone())
    }
}

// EvalContext gains a reference to source
struct EvalContext<'src> {
    source: &'src SourceContext,
    objects: HashMap<String, Object>,
    // ...
}
```

### Parse Result

The parser returns a program with preserved spans:

```rust
struct Program {
    statements: Vec<Spanned<Statement>>,
}

fn parse(source: &str) -> Result<Program, ParseError> {
    // ...
}

fn evaluate(
    program: &Program,
    source: &SourceContext
) -> Result<Diagram, EvalError> {
    // ...
}
```

## What's Missing?

1. **Arc geometry** - center, radius, start angle, end angle
2. **Spline control points** - how to represent bezier curves
3. **Sublist/grouping** - nested diagrams

## Design Decisions

1. **Style lives on `Object`, not `Shape`**
   - Style is orthogonal to geometry
   - A circle is a circle whether red or blue
   - Enables theming, cleaner serialization

2. **Text lives on `Object`, not `Shape`**
   - Text is *attached to* shapes, not *part of* their geometry
   - A box's boundary doesn't change based on its label
   - Text positioning is relative to shape bounds, computed after shape exists

3. **Direction lives in `ExecContext`, not `EvalContext`**
   - Direction affects *where next object goes*, not expression evaluation
   - `EvalContext` stays pure: expression in, value out, no side effects
   - `ExecContext` holds cursor position + direction

4. **One `Shape` enum, not separate closed/path enums**
   - Splitting adds complexity without benefit
   - Use `fn is_path(&self) -> bool` and `fn waypoints(&self) -> Option<&Waypoints>`
   - Most code treats all shapes uniformly anyway
