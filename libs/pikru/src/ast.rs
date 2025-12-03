//! Abstract Syntax Tree types for pikchr
//!
//! These types represent the parsed structure of a pikchr diagram.

/// A complete pikchr program
#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Statement>,
}

/// A pikchr statement
#[derive(Debug, Clone)]
pub enum Statement {
    /// Direction change: up, down, left, right
    Direction(Direction),
    /// Variable assignment: $x = 10, fill = Red
    Assignment(Assignment),
    /// Macro definition: define foo { ... }
    Define(Define),
    /// Macro invocation: foo(args) or bare foo
    MacroCall(MacroCall),
    /// Assert statement: assert(x == y)
    Assert(Assert),
    /// Print statement: print "hello", value
    Print(Print),
    /// Labeled statement: A: box "hello"
    Labeled(LabeledStatement),
    /// Object statement: box "hello" width 2
    Object(ObjectStatement),
}

/// Direction: up, down, left, right
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Variable assignment
#[derive(Debug, Clone)]
pub struct Assignment {
    pub lvalue: LValue,
    pub op: AssignOp,
    pub rvalue: RValue,
}

/// Left-hand side of assignment
#[derive(Debug, Clone)]
pub enum LValue {
    Variable(String),
    Fill,
    Color,
    Thickness,
}

/// Assignment operator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,    // =
    AddAssign, // +=
    SubAssign, // -=
    MulAssign, // *=
    DivAssign, // /=
}

/// Right-hand side of assignment
#[derive(Debug, Clone)]
pub enum RValue {
    Expr(Expr),
    PlaceName(String), // Color names like Red, Blue
}

/// Macro definition
#[derive(Debug, Clone)]
pub struct Define {
    pub name: String,
    pub body: String, // Raw code block content
}

/// Macro invocation
#[derive(Debug, Clone)]
pub struct MacroCall {
    pub name: String,
    pub args: Vec<MacroArg>,
}

/// Macro argument
#[derive(Debug, Clone)]
pub enum MacroArg {
    String(String),
    Expr(Expr),
    Ident(String),
}

/// Assert statement
#[derive(Debug, Clone)]
pub struct Assert {
    pub condition: AssertCondition,
}

/// Assert condition
#[derive(Debug, Clone)]
pub enum AssertCondition {
    ExprEqual(Expr, Expr),
    PositionEqual(Position, Position),
}

/// Print statement
#[derive(Debug, Clone)]
pub struct Print {
    pub args: Vec<PrintArg>,
}

/// Print argument
#[derive(Debug, Clone)]
pub enum PrintArg {
    String(String),
    Expr(Expr),
    PlaceName(String),
}

/// Labeled statement: A: box or A: position
#[derive(Debug, Clone)]
pub struct LabeledStatement {
    pub label: String,
    pub content: LabeledContent,
}

/// Content after a label
#[derive(Debug, Clone)]
pub enum LabeledContent {
    Position(Position),
    Object(ObjectStatement),
}

/// Object statement: basetype with attributes
#[derive(Debug, Clone)]
pub struct ObjectStatement {
    pub basetype: BaseType,
    pub attributes: Vec<Attribute>,
}

/// Base type of an object
#[derive(Debug, Clone)]
pub enum BaseType {
    /// Primitive class: box, circle, line, arrow, etc.
    Class(ClassName),
    /// String text object
    Text(StringLit, Option<TextPosition>),
    /// Sublist: [ statements ]
    Sublist(Vec<Statement>),
}

/// Primitive class names
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassName {
    Arc,
    Arrow,
    Box,
    Circle,
    Cylinder,
    Diamond,
    Dot,
    Ellipse,
    File,
    Line,
    Move,
    Oval,
    Spline,
    Text,
}

/// Object attribute
#[derive(Debug, Clone)]
pub enum Attribute {
    /// Numeric property: width 2, height 3cm
    NumProperty(NumProperty, RelExpr),
    /// Dash property: dotted, dashed 0.1
    DashProperty(DashProperty, Option<Expr>),
    /// Color property: fill Red, color Blue
    ColorProperty(ColorProperty, RValue),
    /// Boolean property: cw, ccw, invis, ->
    BoolProperty(BoolProperty),
    /// Direction with optional distance: right 2cm
    DirectionMove(Option<bool>, Direction, Option<RelExpr>), // go?, direction, distance
    /// Direction even with: right even with B
    DirectionEven(Option<bool>, Direction, Position),
    /// Direction until even with: left until even with B
    DirectionUntilEven(Option<bool>, Direction, Position),
    /// Heading: go 1.5 heading 45
    Heading(Option<RelExpr>, Expr),
    /// Close the path
    Close,
    /// Chop endpoint
    Chop,
    /// From position
    From(Position),
    /// To position
    To(Position),
    /// Then continuation
    Then(Option<ThenClause>),
    /// At position
    At(Position),
    /// With clause: with .n at B
    With(WithClause),
    /// Same as object
    Same(Option<Object>),
    /// Text string with position
    StringAttr(StringLit, Option<TextPosition>),
    /// Fit to content
    Fit,
    /// Behind object
    Behind(Object),
    /// Bare expression (default direction movement)
    BareExpr(RelExpr),
}

/// Then clause content
#[derive(Debug, Clone)]
pub enum ThenClause {
    To(Position),
    DirectionEven(Direction, Position),
    DirectionUntilEven(Direction, Position),
    DirectionMove(Direction, Option<RelExpr>),
    Heading(Option<RelExpr>, Expr),
    EdgePoint(Option<RelExpr>, EdgePoint),
}

/// Numeric property names
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumProperty {
    Height,
    Width,
    Radius,
    Diameter,
    Thickness,
}

/// Dash property names
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashProperty {
    Dotted,
    Dashed,
}

/// Color property names
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorProperty {
    Fill,
    Color,
}

/// Boolean property values
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolProperty {
    Clockwise,
    CounterClockwise,
    Invisible,
    Thick,
    Thin,
    Solid,
    ArrowBoth,   // <->
    ArrowRight,  // ->
    ArrowLeft,   // <-
}

/// With clause: .edge at position
#[derive(Debug, Clone)]
pub struct WithClause {
    pub edge: WithEdge,
    pub position: Position,
}

/// Edge specification in with clause
#[derive(Debug, Clone)]
pub enum WithEdge {
    DotEdge(EdgePoint),
    EdgePoint(EdgePoint),
}

/// Text position attributes
#[derive(Debug, Clone)]
pub struct TextPosition {
    pub attrs: Vec<TextAttr>,
}

/// Text attribute
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAttr {
    Above,
    Below,
    Center,
    LJust,
    RJust,
    Bold,
    Italic,
    Mono,
    Big,
    Small,
    Aligned,
}

/// A relative expression (expr with optional %)
#[derive(Debug, Clone)]
pub struct RelExpr {
    pub expr: Expr,
    pub is_percent: bool,
}

/// Expression
#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64), // stored in inches already
    Variable(String),
    PlaceName(String),
    ParenExpr(Box<Expr>),
    BuiltinVar(BuiltinVar),
    FuncCall(FuncCall),
    DistCall(Box<Position>, Box<Position>),
    ObjectProp(Object, NumProperty),
    ObjectCoord(Object, Coord),
    ObjectEdgeCoord(Object, EdgePoint, Coord),
    VertexCoord(Nth, Object, Coord),
    BinaryOp(Box<Expr>, BinaryOp, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
}

/// Built-in variables
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinVar {
    Fill,
    Color,
    Thickness,
}

/// Coordinate: x or y
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coord {
    X,
    Y,
}

/// Function call
#[derive(Debug, Clone)]
pub struct FuncCall {
    pub func: Function,
    pub args: Vec<Expr>,
}

/// Built-in functions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    Abs,
    Cos,
    Sin,
    Int,
    Sqrt,
    Max,
    Min,
}

/// Binary operator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Unary operator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
}

/// Position
#[derive(Debug, Clone)]
pub enum Position {
    /// (x, y) coordinate pair
    Coords(Expr, Expr),
    /// (pos, pos) - tuple extraction
    Tuple(Box<Position>, Box<Position>),
    /// Place reference
    Place(Place),
    /// Place with offset: B + (1, 2)
    PlaceOffset(Place, BinaryOp, Expr, Expr),
    /// Between positions: 0.5 between A and B
    Between(Expr, Box<Position>, Box<Position>),
    /// Angle bracket: 0.5 <A, B>
    Bracket(Expr, Box<Position>, Box<Position>),
    /// Above/below: 1cm above B
    AboveBelow(Expr, AboveBelow, Box<Position>),
    /// Left/right of: 1cm left of B
    LeftRightOf(Expr, LeftRight, Box<Position>),
    /// Heading: 1cm heading 45 from B
    Heading(Expr, HeadingDir, Box<Position>),
    /// Edge point of: 1cm ne of B
    EdgePointOf(Expr, EdgePoint, Box<Position>),
}

/// Above or below
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AboveBelow {
    Above,
    Below,
}

/// Left or right
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftRight {
    Left,
    Right,
}

/// Heading direction
#[derive(Debug, Clone)]
pub enum HeadingDir {
    EdgePoint(EdgePoint),
    Expr(Expr),
}

/// Place - a reference to a location
#[derive(Debug, Clone)]
pub enum Place {
    /// Vertex of object: 2nd vertex of spline
    Vertex(Nth, Object),
    /// Edge point of object: north of B
    EdgePointOf(EdgePoint, Object),
    /// Object with edge: B.n
    ObjectEdge(Object, EdgePoint),
    /// Bare object: B
    Object(Object),
}

/// Object reference
#[derive(Debug, Clone)]
pub enum Object {
    /// Named object: B, Main.Sub
    Named(ObjectName),
    /// Nth object: 1st box, last circle
    Nth(Nth),
}

/// Named object
#[derive(Debug, Clone)]
pub struct ObjectName {
    pub base: ObjectNameBase,
    pub path: Vec<String>, // dot-separated path
}

/// Base of object name
#[derive(Debug, Clone)]
pub enum ObjectNameBase {
    This,
    PlaceName(String),
}

/// Nth reference
#[derive(Debug, Clone)]
pub enum Nth {
    /// Ordinal: 1st, 2nd, 3rd, etc.
    Ordinal(u32, bool, Option<NthClass>), // number, is_last, classname
    /// First
    First(Option<NthClass>),
    /// Last
    Last(Option<NthClass>),
    /// Previous
    Previous,
}

/// Class for nth reference
#[derive(Debug, Clone)]
pub enum NthClass {
    ClassName(ClassName),
    Sublist,
}

/// Edge point names
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgePoint {
    North,
    South,
    East,
    West,
    Start,
    End,
    Center,
    Bottom,
    Top,
    Left,
    Right,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
    N,
    S,
    E,
    W,
    C,
    T,
}

/// String literal
#[derive(Debug, Clone)]
pub struct StringLit {
    pub value: String,
}
