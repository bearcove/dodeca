//! Core types for pikchr rendering

use crate::ast::TextAttr;
use crate::types::{BoxIn, EvalValue, Length as Inches, OffsetIn, Point, PtIn, UnitVec};

use super::defaults;
use super::shapes::Shape;

/// Generic numeric value that can be either a length (in inches) or a unitless scalar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Len(Inches),
    Scalar(f64),
}

impl Value {
    #[allow(dead_code)]
    pub fn as_len(self) -> Result<Inches, miette::Report> {
        match self {
            Value::Len(l) => Ok(l),
            Value::Scalar(_) => Err(miette::miette!("Expected length value, got scalar")),
        }
    }

    #[allow(dead_code)]
    pub fn as_scalar(self) -> Result<f64, miette::Report> {
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

/// A point in 2D space
pub type PointIn = PtIn;

/// Bounding box
pub type BoundingBox = BoxIn;

pub fn pin(x: f64, y: f64) -> PointIn {
    Point::new(Inches(x), Inches(y))
}

/// Text with optional positioning and styling attributes
#[derive(Debug, Clone)]
pub struct PositionedText {
    pub value: String,
    pub above: bool,
    pub below: bool,
    pub ljust: bool,
    pub rjust: bool,
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

    pub fn from_textposition(value: String, pos: Option<&crate::ast::TextPosition>) -> Self {
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
    pub shape: super::shapes::ShapeEnum,
    pub start_attachment: Option<EndpointObject>,
    pub end_attachment: Option<EndpointObject>,
}

impl RenderedObject {
    /// Translate this object by an offset
    pub fn translate(&mut self, offset: OffsetIn) {
        self.shape.translate(offset);
    }

    /// Calculate edge point in a given direction
    /// For round shapes, diagonal directions use the perimeter (1/√2 factor)
    pub fn edge_point(&self, dir: UnitVec) -> PointIn {
        // Convert UnitVec to EdgeDirection
        use super::shapes::EdgeDirection;
        let edge_dir = if dir == UnitVec::NORTH {
            EdgeDirection::North
        } else if dir == UnitVec::SOUTH {
            EdgeDirection::South
        } else if dir == UnitVec::EAST {
            EdgeDirection::East
        } else if dir == UnitVec::WEST {
            EdgeDirection::West
        } else if dir == UnitVec::NORTH_EAST {
            EdgeDirection::NorthEast
        } else if dir == UnitVec::NORTH_WEST {
            EdgeDirection::NorthWest
        } else if dir == UnitVec::SOUTH_EAST {
            EdgeDirection::SouthEast
        } else if dir == UnitVec::SOUTH_WEST {
            EdgeDirection::SouthWest
        } else {
            EdgeDirection::Center
        };

        self.shape.edge_point(edge_dir)
    }

    // Convenience accessors that delegate to shape
    pub fn center(&self) -> PointIn {
        self.shape.center()
    }

    pub fn width(&self) -> Inches {
        self.shape.width()
    }

    pub fn height(&self) -> Inches {
        self.shape.height()
    }

    pub fn start(&self) -> PointIn {
        self.shape.start()
    }

    pub fn end(&self) -> PointIn {
        self.shape.end()
    }

    pub fn style(&self) -> &ObjectStyle {
        self.shape.style()
    }

    pub fn text(&self) -> &[PositionedText] {
        self.shape.text()
    }

    pub fn waypoints(&self) -> Option<&[PointIn]> {
        self.shape.waypoints()
    }

    pub fn class(&self) -> ObjectClass {
        self.shape.object_class()
    }

    pub fn children(&self) -> Option<&[super::shapes::ShapeEnum]> {
        if let super::shapes::ShapeEnum::Sublist(ref s) = self.shape {
            Some(&s.children)
        } else {
            None
        }
    }
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
    pub fn from_rendered(obj: &RenderedObject) -> Self {
        Self {
            class: obj.shape.object_class(),
            center: obj.shape.center(),
            width: obj.shape.width(),
            height: obj.shape.height(),
            corner_radius: obj.shape.style().corner_radius,
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

impl ObjectClass {
    /// Returns true if this is a round shape (circle, ellipse, oval)
    pub fn is_round(self) -> bool {
        matches!(self, Self::Circle | Self::Ellipse | Self::Oval)
    }

    /// Diagonal factor for edge point calculations.
    /// Round shapes use 1/√2 so diagonal points land on the perimeter.
    /// Rectangular shapes use 1.0 so diagonal points land on bounding box corners.
    pub fn diagonal_factor(self) -> f64 {
        if self.is_round() {
            std::f64::consts::FRAC_1_SQRT_2
        } else {
            1.0
        }
    }
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
    /// For arcs: true = clockwise, false = counter-clockwise (default)
    pub clockwise: bool,
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
            clockwise: false,
        }
    }
}
