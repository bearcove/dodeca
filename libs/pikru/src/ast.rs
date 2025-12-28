//! Abstract Syntax Tree types for pikchr
//!
//! These types represent the parsed structure of a pikchr diagram.

use crate::types::{Angle, Length, OffsetIn, UnitVec};
use glam::DVec2;

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
    /// Error statement: error "message" - produces an error
    Error(ErrorStmt),
    /// Labeled statement: A: box "hello"
    Labeled(LabeledStatement),
    /// Object statement: box "hello" width 2
    Object(ObjectStatement),
}

/// Error statement - produces an intentional error
#[derive(Debug, Clone)]
pub struct ErrorStmt {
    pub message: String,
}

/// Direction: up, down, left, right
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    /// Unit vector for this direction in internal coordinate space (Y-up).
    /// Like C pikchr, we use Y-up internally:
    /// - Up = (0, +1)
    /// - Down = (0, -1)
    /// - Right = (+1, 0)
    /// - Left = (-1, 0)
    ///
    /// The Y-flip to SVG (Y-down) happens in `Point::to_svg()`.
    #[inline]
    pub fn unit_vector(self) -> DVec2 {
        match self {
            Direction::Right => DVec2::X,
            Direction::Left => DVec2::NEG_X,
            Direction::Up => DVec2::Y,       // Y-up: Up increases Y
            Direction::Down => DVec2::NEG_Y, // Y-up: Down decreases Y
        }
    }

    /// Get offset for moving `distance` in this direction.
    /// Uses Y-up internal coordinates (like C pikchr).
    #[inline]
    pub fn offset(self, distance: Length) -> OffsetIn {
        let v = self.unit_vector() * distance.0;
        OffsetIn::new(Length(v.x), Length(v.y))
    }

    /// Get the opposite direction
    #[inline]
    pub fn opposite(self) -> Direction {
        match self {
            Direction::Right => Direction::Left,
            Direction::Left => Direction::Right,
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
        }
    }

    /// Rotate direction 90 degrees for arc exit direction.
    /// cref: pikchr.c:4333 - pObj->outDir = (pObj->inDir + (pObj->cw ? 1 : 3))%4
    ///
    /// DIR_RIGHT=0, DIR_DOWN=1, DIR_LEFT=2, DIR_UP=3
    /// CCW (cw=false): +3 mod 4 → rotates counter-clockwise (RIGHT→UP→LEFT→DOWN)
    /// CW (cw=true): +1 mod 4 → rotates clockwise (RIGHT→DOWN→LEFT→UP)
    #[inline]
    pub fn arc_exit(self, clockwise: bool) -> Direction {
        // Map our enum to C pikchr's numeric values
        let dir_num = match self {
            Direction::Right => 0,
            Direction::Down => 1,
            Direction::Left => 2,
            Direction::Up => 3,
        };

        // Apply rotation: CW adds 1, CCW adds 3 (equivalent to -1 mod 4)
        let out_num = (dir_num + if clockwise { 1 } else { 3 }) % 4;

        // Map back to our enum
        match out_num {
            0 => Direction::Right,
            1 => Direction::Down,
            2 => Direction::Left,
            _ => Direction::Up, // 3
        }
    }

    /// Try to get a cardinal direction from an edge point.
    /// Returns None for diagonal edge points (nw, ne, sw, se) or center.
    /// cref: C pikchr uses pik_hdg_angle to convert edge points to angles,
    /// then maps angles to directions (pikchr.c:3350-3360)
    pub fn from_edge_point(ep: &EdgePoint) -> Option<Direction> {
        match ep {
            EdgePoint::North | EdgePoint::Top | EdgePoint::N => Some(Direction::Up),
            EdgePoint::South | EdgePoint::Bottom | EdgePoint::S => Some(Direction::Down),
            EdgePoint::East | EdgePoint::Right | EdgePoint::E => Some(Direction::Right),
            EdgePoint::West | EdgePoint::Left | EdgePoint::W => Some(Direction::Left),
            // Diagonal directions - use dominant component like C pikchr
            // cref: pikchr.c:3350-3360 maps angle ranges to directions
            EdgePoint::NorthEast => Some(Direction::Up), // 45° → Up
            EdgePoint::NorthWest => Some(Direction::Up), // 315° → Up
            EdgePoint::SouthEast => Some(Direction::Down), // 135° → Down
            EdgePoint::SouthWest => Some(Direction::Down), // 225° → Down
            _ => None, // Center, Start, End don't have a direction
        }
    }
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
    PositionEqual(Box<Position>, Box<Position>),
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
    /// Sublist: \[ statements \]
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
    Sublist,
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
    ArrowBoth,  // <->
    ArrowRight, // ->
    ArrowLeft,  // <-
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

impl EdgePoint {
    /// Get the unit offset direction for this edge point
    pub fn to_unit_vec(self) -> UnitVec {
        match self {
            Self::North | Self::N | Self::Top | Self::T => UnitVec::NORTH,
            Self::South | Self::S | Self::Bottom => UnitVec::SOUTH,
            Self::East | Self::E | Self::Right => UnitVec::EAST,
            Self::West | Self::W | Self::Left => UnitVec::WEST,
            Self::NorthEast => UnitVec::NORTH_EAST,
            Self::NorthWest => UnitVec::NORTH_WEST,
            Self::SouthEast => UnitVec::SOUTH_EAST,
            Self::SouthWest => UnitVec::SOUTH_WEST,
            Self::Center | Self::C => UnitVec::ZERO,
            Self::Start => UnitVec::WEST, // Start is typically the entry direction
            Self::End => UnitVec::EAST,   // End is typically the exit direction
        }
    }

    /// Convert to an angle (0° = north, clockwise)
    pub fn to_angle(self) -> Angle {
        Angle::degrees(match self {
            Self::North | Self::N | Self::Top | Self::T => 0.0,
            Self::NorthEast => 45.0,
            Self::East | Self::E | Self::Right => 90.0,
            Self::SouthEast => 135.0,
            Self::South | Self::S | Self::Bottom => 180.0,
            Self::SouthWest => 225.0,
            Self::West | Self::W | Self::Left => 270.0,
            Self::NorthWest => 315.0,
            _ => 0.0,
        })
    }
}

/// String literal
#[derive(Debug, Clone)]
pub struct StringLit {
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direction_offset_svg_coordinates() {
        let d = Length::inches(1.0);

        // Right increases X
        let r = Direction::Right.offset(d);
        assert!(r.dx > Length::ZERO, "Right should increase X");
        assert_eq!(r.dy, Length::ZERO, "Right should not change Y");

        // Left decreases X
        let l = Direction::Left.offset(d);
        assert!(l.dx < Length::ZERO, "Left should decrease X");
        assert_eq!(l.dy, Length::ZERO, "Left should not change Y");

        // Up increases Y (Y-up internal coordinates, like C pikchr)
        let u = Direction::Up.offset(d);
        assert_eq!(u.dx, Length::ZERO, "Up should not change X");
        assert!(
            u.dy > Length::ZERO,
            "Up should increase Y (Y-up internally)"
        );

        // Down decreases Y (Y-up internal coordinates)
        let down = Direction::Down.offset(d);
        assert_eq!(down.dx, Length::ZERO, "Down should not change X");
        assert!(
            down.dy < Length::ZERO,
            "Down should decrease Y (Y-up internally)"
        );
    }

    #[test]
    fn test_direction_unit_vector() {
        // Right = (1, 0)
        let r = Direction::Right.unit_vector();
        assert_eq!(r.x, 1.0);
        assert_eq!(r.y, 0.0);

        // Left = (-1, 0)
        let l = Direction::Left.unit_vector();
        assert_eq!(l.x, -1.0);
        assert_eq!(l.y, 0.0);

        // Up = (0, 1) in Y-up coordinates (like C pikchr)
        let u = Direction::Up.unit_vector();
        assert_eq!(u.x, 0.0);
        assert_eq!(u.y, 1.0);

        // Down = (0, -1) in Y-up coordinates
        let d = Direction::Down.unit_vector();
        assert_eq!(d.x, 0.0);
        assert_eq!(d.y, -1.0);
    }

    #[test]
    fn test_direction_opposite() {
        assert_eq!(Direction::Right.opposite(), Direction::Left);
        assert_eq!(Direction::Left.opposite(), Direction::Right);
        assert_eq!(Direction::Up.opposite(), Direction::Down);
        assert_eq!(Direction::Down.opposite(), Direction::Up);
    }
}
