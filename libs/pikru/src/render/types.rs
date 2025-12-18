//! Core types for pikchr rendering

use crate::ast::TextAttr;
use crate::types::{BoxIn, EvalValue, Length as Inches, OffsetIn, Point, PtIn, UnitVec};

use super::defaults;
use super::shapes::Shape;

/// Generic numeric value that can be either a length (in inches), a unitless scalar, or a color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Len(Inches),
    Scalar(f64),
    Color(u32), // RGB color packed as 0xRRGGBB
}

impl Value {
    #[allow(dead_code)]
    pub fn as_len(self) -> Result<Inches, miette::Report> {
        match self {
            Value::Len(l) => Ok(l),
            Value::Scalar(_) => Err(miette::miette!("Expected length value, got scalar")),
            Value::Color(_) => Err(miette::miette!("Expected length value, got color")),
        }
    }

    #[allow(dead_code)]
    pub fn as_scalar(self) -> Result<f64, miette::Report> {
        match self {
            Value::Scalar(s) => Ok(s),
            Value::Len(_) => Err(miette::miette!("Expected scalar value, got length")),
            Value::Color(_) => Err(miette::miette!("Expected scalar value, got color")),
        }
    }
}

impl From<EvalValue> for Value {
    fn from(ev: EvalValue) -> Self {
        match ev {
            EvalValue::Length(l) => Value::Len(l),
            EvalValue::Scalar(s) => Value::Scalar(s),
            EvalValue::Color(c) => Value::Color(c),
        }
    }
}

impl From<Value> for EvalValue {
    fn from(v: Value) -> Self {
        match v {
            Value::Len(l) => EvalValue::Length(l),
            Value::Scalar(s) => EvalValue::Scalar(s),
            Value::Color(c) => EvalValue::Color(c),
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
    pub xtra: bool,  // Amplify big or small (for double big/small)
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
            xtra: false,
        }
    }

    pub fn from_textposition(value: String, pos: Option<&crate::ast::TextPosition>) -> Self {
        let mut pt = Self::new(value);
        if let Some(pos) = pos {
            // cref: pik_txt_token (pikchr.c:6262-6265)
            // If we see a second Big or Small, set xtra flag
            for attr in &pos.attrs {
                match attr {
                    TextAttr::Above => pt.above = true,
                    TextAttr::Below => pt.below = true,
                    TextAttr::LJust => pt.ljust = true,
                    TextAttr::RJust => pt.rjust = true,
                    TextAttr::Bold => pt.bold = true,
                    TextAttr::Italic => pt.italic = true,
                    TextAttr::Mono => pt.mono = true,
                    TextAttr::Big => {
                        if pt.big {
                            // Second occurrence of Big - set xtra
                            pt.xtra = true;
                        } else {
                            pt.big = true;
                        }
                    }
                    TextAttr::Small => {
                        if pt.small {
                            // Second occurrence of Small - set xtra
                            pt.xtra = true;
                        } else {
                            pt.small = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        pt
    }

    /// Font scale factor: 1.25 for big, 0.8 for small, 1.0 otherwise
    /// If xtra is true, square the scale (for double big/small)
    // cref: pik_font_scale (pikchr.c:5065-5071)
    pub fn font_scale(&self) -> f64 {
        let mut scale = 1.0;
        if self.big {
            scale *= 1.25;
        }
        if self.small {
            scale *= 0.8;
        }
        if self.xtra {
            scale *= scale;  // Square the scale for double big/small
        }
        scale
    }

    /// Calculate text width in inches, accounting for font properties.
    /// Uses monospace width (82 units/char) or proportional width table.
    /// Applies font scale and bold multiplier.
    // cref: pik_append_txt (pikchr.c:5165-5171)
    pub fn width_inches(&self, charwid: f64) -> f64 {
        let length_hundredths = if self.mono {
            super::monospace_text_length(&self.value)
        } else {
            super::proportional_text_length(&self.value)
        };

        let mut width = length_hundredths as f64 * charwid * self.font_scale() * 0.01;

        // Bold (without mono) text is wider
        if self.bold && !self.mono {
            width *= 1.1;
        }

        width
    }

    /// Height contribution for this text line
    // cref: pik_append_txt (pikchr.c:5107-5108)
    pub fn height(&self, charht: f64) -> f64 {
        self.font_scale() * charht
    }
}

/// A rendered object with its properties
#[derive(Debug, Clone)]
pub struct RenderedObject {
    pub name: Option<String>,
    pub shape: super::shapes::ShapeEnum,
    pub start_attachment: Option<EndpointObject>,
    pub end_attachment: Option<EndpointObject>,
    /// Layer for z-ordering. Lower layers render first (behind).
    /// Default is 1000. Set via "layer" variable.
    // cref: pik_elem_new (pikchr.c:2960)
    pub layer: i32,
    /// The layout direction when this object was created.
    /// Used to resolve .start and .end edge points.
    // cref: pObj->inDir, pObj->outDir in C pikchr
    pub direction: crate::ast::Direction,
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

    pub fn class(&self) -> ClassName {
        self.shape.class()
    }

    pub fn children(&self) -> Option<&[RenderedObject]> {
        if let super::shapes::ShapeEnum::Sublist(ref s) = self.shape {
            Some(&s.children)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct EndpointObject {
    pub class: ClassName,
    pub center: PointIn,
    pub width: Inches,
    pub height: Inches,
    pub corner_radius: Inches,
}

impl EndpointObject {
    pub fn from_rendered(obj: &RenderedObject) -> Self {
        Self {
            class: obj.shape.class(),
            center: obj.shape.center(),
            width: obj.shape.width(),
            height: obj.shape.height(),
            corner_radius: obj.shape.style().corner_radius,
        }
    }
}

/// Re-export ClassName as the object class type
pub use crate::ast::ClassName;

impl ClassName {
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
    /// Dashed line style. Some(width) = dashed with that dash width, None = not dashed.
    /// The width is stored directly from the attribute (e.g., `dashed 0.25` stores 0.25).
    pub dashed: Option<Inches>,
    /// Dotted line style. Some(gap) = dotted with that gap width, None = not dotted.
    pub dotted: Option<Inches>,
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
            dashed: None,
            dotted: None,
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
