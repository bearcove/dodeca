//! Strongly-typed numeric primitives for pikru (zero-cost newtypes).
//!
//! Design goals (from STRONG_TYPES.md):
//! - No raw `f64` in domain logic
//! - Illegal states unrepresentable
//! - Conversions only via Scaler

use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use glam::DVec2;
use miette::SourceSpan;

// ==================== Source Tracking ====================

/// A location in source code (byte offsets)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    /// Byte offset of start (inclusive)
    pub start: usize,
    /// Byte offset of end (exclusive)
    pub end: usize,
}

impl Span {
    /// Create a new span
    #[inline]
    pub const fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    /// Create a zero-width span at a position
    #[inline]
    pub const fn at(pos: usize) -> Self {
        Span {
            start: pos,
            end: pos,
        }
    }

    /// Merge two spans to cover both
    #[inline]
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Length in bytes
    #[inline]
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Check if span is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

impl From<Span> for SourceSpan {
    fn from(s: Span) -> Self {
        SourceSpan::new(s.start.into(), s.len())
    }
}

/// A value with its source location
#[derive(Clone, Debug)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Create a new spanned value
    #[inline]
    pub fn new(node: T, span: Span) -> Self {
        Spanned { node, span }
    }

    /// Map the inner value while preserving span
    #[inline]
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Spanned<U> {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }

    /// Get a reference to the inner value
    #[inline]
    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            node: &self.node,
            span: self.span,
        }
    }

    /// Unwrap the inner value, discarding span
    #[inline]
    pub fn into_inner(self) -> T {
        self.node
    }
}

impl<T: PartialEq> PartialEq for Spanned<T> {
    fn eq(&self, other: &Self) -> bool {
        // Compare only the node, not the span (useful for testing)
        self.node == other.node
    }
}

impl<T: Copy> Copy for Spanned<T> {}

/// Error type for invalid numeric values
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumericError {
    /// Value is NaN
    NaN,
    /// Value is infinite
    Infinite,
    /// Value is zero when non-zero required
    Zero,
    /// Value is negative when positive required
    Negative,
}

impl fmt::Display for NumericError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NumericError::NaN => write!(f, "value is NaN"),
            NumericError::Infinite => write!(f, "value is infinite"),
            NumericError::Zero => write!(f, "value is zero"),
            NumericError::Negative => write!(f, "value is negative"),
        }
    }
}

impl std::error::Error for NumericError {}

/// Length in inches (pikchr canonical unit)
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default)]
#[repr(transparent)]
pub struct Length(pub f64);

impl Length {
    pub const ZERO: Length = Length(0.0);

    /// Create a Length from inches (const-friendly, unchecked).
    /// Use `try_new` for user-provided values.
    #[inline]
    pub(crate) const fn inches(val: f64) -> Length {
        Length(val)
    }

    /// Create a Length with validation (rejects NaN/infinite)
    #[inline]
    pub fn try_new(val: f64) -> Result<Length, NumericError> {
        if val.is_nan() {
            Err(NumericError::NaN)
        } else if val.is_infinite() {
            Err(NumericError::Infinite)
        } else {
            Ok(Length(val))
        }
    }

    /// Create a non-negative Length with validation
    #[inline]
    pub fn try_non_negative(val: f64) -> Result<Length, NumericError> {
        if val.is_nan() {
            Err(NumericError::NaN)
        } else if val.is_infinite() {
            Err(NumericError::Infinite)
        } else if val < 0.0 {
            Err(NumericError::Negative)
        } else {
            Ok(Length(val))
        }
    }

    pub fn to_px(self, r_scale: f64) -> Px {
        Px(self.0 * r_scale)
    }

    /// Get the absolute value
    #[inline]
    pub fn abs(self) -> Length {
        Length(self.0.abs())
    }

    /// Get the minimum of two lengths
    #[inline]
    pub fn min(self, other: Length) -> Length {
        Length(self.0.min(other.0))
    }

    /// Get the maximum of two lengths
    #[inline]
    pub fn max(self, other: Length) -> Length {
        Length(self.0.max(other.0))
    }

    /// Get the raw value (use sparingly, prefer typed operations)
    #[inline]
    pub fn raw(self) -> f64 {
        self.0
    }

    /// Checked division returning None if divisor is zero
    #[inline]
    pub fn checked_div(self, rhs: Length) -> Option<Scalar> {
        if rhs.0 == 0.0 {
            None
        } else {
            Some(Scalar(self.0 / rhs.0))
        }
    }

    /// Check if this length is finite (not NaN or infinite)
    #[inline]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

impl Add for Length {
    type Output = Length;
    fn add(self, rhs: Length) -> Length {
        Length(self.0 + rhs.0)
    }
}
impl Sub for Length {
    type Output = Length;
    fn sub(self, rhs: Length) -> Length {
        Length(self.0 - rhs.0)
    }
}
impl Mul<f64> for Length {
    type Output = Length;
    fn mul(self, rhs: f64) -> Length {
        Length(self.0 * rhs)
    }
}
impl Div<f64> for Length {
    type Output = Length;
    fn div(self, rhs: f64) -> Length {
        Length(self.0 / rhs)
    }
}

// NOTE: Length / Length is intentionally NOT implemented as a trait.
// Use Length::checked_div() which returns Option<Scalar> and handles zero divisor.
// This prevents silent infinity from division by zero in layout math.

impl Neg for Length {
    type Output = Length;
    fn neg(self) -> Length {
        Length(-self.0)
    }
}

impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AddAssign for Length {
    fn add_assign(&mut self, rhs: Length) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Length {
    fn sub_assign(&mut self, rhs: Length) {
        self.0 -= rhs.0;
    }
}

/// Pixels after applying r_scale
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default)]
pub struct Px(pub f64);

impl fmt::Display for Px {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unitless scalar (for percentages, counts, ratios, etc.)
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default)]
#[repr(transparent)]
pub struct Scalar(pub f64);

impl Scalar {
    pub const ZERO: Scalar = Scalar(0.0);
    pub const ONE: Scalar = Scalar(1.0);

    /// Get the raw value
    #[inline]
    pub fn raw(self) -> f64 {
        self.0
    }

    /// Check if finite
    #[inline]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

/// A typed value from expression evaluation.
/// Used in the variable map to preserve type information.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EvalValue {
    /// A length in inches
    Length(Length),
    /// A unitless scalar (scale factors, fill opacity, etc.)
    Scalar(f64),
    /// A color as 24-bit RGB value
    Color(u32),
}

impl EvalValue {
    /// Extract as Length, or None if not a length
    pub fn as_length(self) -> Option<Length> {
        match self {
            EvalValue::Length(l) => Some(l),
            _ => None,
        }
    }

    /// Extract as scalar f64, converting Length to its raw value
    pub fn as_scalar(self) -> f64 {
        match self {
            EvalValue::Length(l) => l.raw(),
            EvalValue::Scalar(s) => s,
            EvalValue::Color(c) => c as f64,
        }
    }

    /// Extract as color, or None if not a color
    pub fn as_color(self) -> Option<u32> {
        match self {
            EvalValue::Color(c) => Some(c),
            _ => None,
        }
    }

    /// Check if finite (not NaN or infinite)
    pub fn is_finite(self) -> bool {
        match self {
            EvalValue::Length(l) => l.is_finite(),
            EvalValue::Scalar(s) => s.is_finite(),
            EvalValue::Color(_) => true,
        }
    }
}

impl fmt::Display for EvalValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalValue::Length(l) => write!(f, "{}in", l),
            EvalValue::Scalar(s) => write!(f, "{}", s),
            EvalValue::Color(c) => write!(f, "#{:06x}", c),
        }
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Scalar * Length = Length (scaling a length)
impl Mul<Length> for Scalar {
    type Output = Length;
    fn mul(self, rhs: Length) -> Length {
        Length(self.0 * rhs.0)
    }
}

/// Length * Scalar = Length (scaling a length)
impl Mul<Scalar> for Length {
    type Output = Length;
    fn mul(self, rhs: Scalar) -> Length {
        Length(self.0 * rhs.0)
    }
}

/// Angle in degrees (pikchr uses degrees with 0 = north, clockwise)
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default)]
#[repr(transparent)]
pub struct Angle(f64);

impl Angle {
    /// Create an Angle with validation (rejects NaN/infinite)
    #[inline]
    pub fn try_new(degrees: f64) -> Result<Angle, NumericError> {
        if degrees.is_nan() {
            Err(NumericError::NaN)
        } else if degrees.is_infinite() {
            Err(NumericError::Infinite)
        } else {
            Ok(Angle(degrees))
        }
    }

    /// Create an Angle from degrees (const-friendly, unchecked).
    /// Use `try_new` for user-provided values.
    #[inline]
    pub const fn degrees(val: f64) -> Angle {
        Angle(val)
    }

    /// Create an Angle from radians with validation
    #[inline]
    pub fn from_radians(radians: f64) -> Result<Angle, NumericError> {
        if radians.is_nan() {
            Err(NumericError::NaN)
        } else if radians.is_infinite() {
            Err(NumericError::Infinite)
        } else {
            Ok(Angle(radians.to_degrees()))
        }
    }

    /// Convert to radians
    #[inline]
    pub fn to_radians(self) -> f64 {
        self.0.to_radians()
    }

    /// Get the raw degrees value
    #[inline]
    pub fn raw(self) -> f64 {
        self.0
    }

    /// Check if finite
    #[inline]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }

    /// Normalize angle to [0, 360) range
    #[inline]
    pub fn normalized(self) -> Angle {
        let mut d = self.0 % 360.0;
        if d < 0.0 {
            d += 360.0;
        }
        Angle(d)
    }
}

impl fmt::Display for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}°", self.0)
    }
}

/// Simple color model; keep raw string for now to avoid regressions.
#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    Named(String),
    Rgb(u8, u8, u8),
    Rgba(u8, u8, u8, u8),
    Raw(String),
}

impl Color {
    /// Convert color to RGB string format for SVG output.
    /// Named colors are converted to their rgb() equivalents.
    pub fn to_rgb_string(&self) -> String {
        match self {
            Color::Rgb(r, g, b) => format!("rgb({},{},{})", r, g, b),
            Color::Rgba(r, g, b, _) => format!("rgb({},{},{})", r, g, b), // SVG doesn't use alpha here
            Color::Named(name) | Color::Raw(name) => {
                // Convert named colors to rgb() format like C pikchr
                match name.to_lowercase().as_str() {
                    "aliceblue" => "rgb(240,248,255)".to_string(),
                    "antiquewhite" => "rgb(250,235,215)".to_string(),
                    "aqua" => "rgb(0,255,255)".to_string(),
                    "aquamarine" => "rgb(127,255,212)".to_string(),
                    "azure" => "rgb(240,255,255)".to_string(),
                    "beige" => "rgb(245,245,220)".to_string(),
                    "bisque" => "rgb(255,228,196)".to_string(),
                    "black" => "rgb(0,0,0)".to_string(),
                    "blanchedalmond" => "rgb(255,235,205)".to_string(),
                    "blue" => "rgb(0,0,255)".to_string(),
                    "blueviolet" => "rgb(138,43,226)".to_string(),
                    "brown" => "rgb(165,42,42)".to_string(),
                    "burlywood" => "rgb(222,184,135)".to_string(),
                    "cadetblue" => "rgb(95,158,160)".to_string(),
                    "chartreuse" => "rgb(127,255,0)".to_string(),
                    "chocolate" => "rgb(210,105,30)".to_string(),
                    "coral" => "rgb(255,127,80)".to_string(),
                    "cornflowerblue" => "rgb(100,149,237)".to_string(),
                    "cornsilk" => "rgb(255,248,220)".to_string(),
                    "crimson" => "rgb(220,20,60)".to_string(),
                    "cyan" => "rgb(0,255,255)".to_string(),
                    "darkblue" => "rgb(0,0,139)".to_string(),
                    "darkcyan" => "rgb(0,139,139)".to_string(),
                    "darkgoldenrod" => "rgb(184,134,11)".to_string(),
                    "darkgray" | "darkgrey" => "rgb(169,169,169)".to_string(),
                    "darkgreen" => "rgb(0,100,0)".to_string(),
                    "darkkhaki" => "rgb(189,183,107)".to_string(),
                    "darkmagenta" => "rgb(139,0,139)".to_string(),
                    "darkolivegreen" => "rgb(85,107,47)".to_string(),
                    "darkorange" => "rgb(255,140,0)".to_string(),
                    "darkorchid" => "rgb(153,50,204)".to_string(),
                    "darkred" => "rgb(139,0,0)".to_string(),
                    "darksalmon" => "rgb(233,150,122)".to_string(),
                    "darkseagreen" => "rgb(143,188,143)".to_string(),
                    "darkslateblue" => "rgb(72,61,139)".to_string(),
                    "darkslategray" | "darkslategrey" => "rgb(47,79,79)".to_string(),
                    "darkturquoise" => "rgb(0,206,209)".to_string(),
                    "darkviolet" => "rgb(148,0,211)".to_string(),
                    "deeppink" => "rgb(255,20,147)".to_string(),
                    "deepskyblue" => "rgb(0,191,255)".to_string(),
                    "dimgray" | "dimgrey" => "rgb(105,105,105)".to_string(),
                    "dodgerblue" => "rgb(30,144,255)".to_string(),
                    "firebrick" => "rgb(178,34,34)".to_string(),
                    "floralwhite" => "rgb(255,250,240)".to_string(),
                    "forestgreen" => "rgb(34,139,34)".to_string(),
                    "fuchsia" => "rgb(255,0,255)".to_string(),
                    "gainsboro" => "rgb(220,220,220)".to_string(),
                    "ghostwhite" => "rgb(248,248,255)".to_string(),
                    "gold" => "rgb(255,215,0)".to_string(),
                    "goldenrod" => "rgb(218,165,32)".to_string(),
                    "gray" | "grey" => "rgb(128,128,128)".to_string(),
                    "green" => "rgb(0,128,0)".to_string(),
                    "greenyellow" => "rgb(173,255,47)".to_string(),
                    "honeydew" => "rgb(240,255,240)".to_string(),
                    "hotpink" => "rgb(255,105,180)".to_string(),
                    "indianred" => "rgb(205,92,92)".to_string(),
                    "indigo" => "rgb(75,0,130)".to_string(),
                    "ivory" => "rgb(255,255,240)".to_string(),
                    "khaki" => "rgb(240,230,140)".to_string(),
                    "lavender" => "rgb(230,230,250)".to_string(),
                    "lavenderblush" => "rgb(255,240,245)".to_string(),
                    "lawngreen" => "rgb(124,252,0)".to_string(),
                    "lemonchiffon" => "rgb(255,250,205)".to_string(),
                    "lightblue" => "rgb(173,216,230)".to_string(),
                    "lightcoral" => "rgb(240,128,128)".to_string(),
                    "lightcyan" => "rgb(224,255,255)".to_string(),
                    "lightgoldenrodyellow" => "rgb(250,250,210)".to_string(),
                    "lightgray" | "lightgrey" => "rgb(211,211,211)".to_string(),
                    "lightgreen" => "rgb(144,238,144)".to_string(),
                    "lightpink" => "rgb(255,182,193)".to_string(),
                    "lightsalmon" => "rgb(255,160,122)".to_string(),
                    "lightseagreen" => "rgb(32,178,170)".to_string(),
                    "lightskyblue" => "rgb(135,206,250)".to_string(),
                    "lightslategray" | "lightslategrey" => "rgb(119,136,153)".to_string(),
                    "lightsteelblue" => "rgb(176,196,222)".to_string(),
                    "lightyellow" => "rgb(255,255,224)".to_string(),
                    "lime" => "rgb(0,255,0)".to_string(),
                    "limegreen" => "rgb(50,205,50)".to_string(),
                    "linen" => "rgb(250,240,230)".to_string(),
                    "magenta" => "rgb(255,0,255)".to_string(),
                    "maroon" => "rgb(128,0,0)".to_string(),
                    "mediumaquamarine" => "rgb(102,205,170)".to_string(),
                    "mediumblue" => "rgb(0,0,205)".to_string(),
                    "mediumorchid" => "rgb(186,85,211)".to_string(),
                    "mediumpurple" => "rgb(147,112,219)".to_string(),
                    "mediumseagreen" => "rgb(60,179,113)".to_string(),
                    "mediumslateblue" => "rgb(123,104,238)".to_string(),
                    "mediumspringgreen" => "rgb(0,250,154)".to_string(),
                    "mediumturquoise" => "rgb(72,209,204)".to_string(),
                    "mediumvioletred" => "rgb(199,21,133)".to_string(),
                    "midnightblue" => "rgb(25,25,112)".to_string(),
                    "mintcream" => "rgb(245,255,250)".to_string(),
                    "mistyrose" => "rgb(255,228,225)".to_string(),
                    "moccasin" => "rgb(255,228,181)".to_string(),
                    "navajowhite" => "rgb(255,222,173)".to_string(),
                    "navy" => "rgb(0,0,128)".to_string(),
                    "oldlace" => "rgb(253,245,230)".to_string(),
                    "olive" => "rgb(128,128,0)".to_string(),
                    "olivedrab" => "rgb(107,142,35)".to_string(),
                    "orange" => "rgb(255,165,0)".to_string(),
                    "orangered" => "rgb(255,69,0)".to_string(),
                    "orchid" => "rgb(218,112,214)".to_string(),
                    "palegoldenrod" => "rgb(238,232,170)".to_string(),
                    "palegreen" => "rgb(152,251,152)".to_string(),
                    "paleturquoise" => "rgb(175,238,238)".to_string(),
                    "palevioletred" => "rgb(219,112,147)".to_string(),
                    "papayawhip" => "rgb(255,239,213)".to_string(),
                    "peachpuff" => "rgb(255,218,185)".to_string(),
                    "peru" => "rgb(205,133,63)".to_string(),
                    "pink" => "rgb(255,192,203)".to_string(),
                    "plum" => "rgb(221,160,221)".to_string(),
                    "powderblue" => "rgb(176,224,230)".to_string(),
                    "purple" => "rgb(128,0,128)".to_string(),
                    "rebeccapurple" => "rgb(102,51,153)".to_string(),
                    "red" => "rgb(255,0,0)".to_string(),
                    "rosybrown" => "rgb(188,143,143)".to_string(),
                    "royalblue" => "rgb(65,105,225)".to_string(),
                    "saddlebrown" => "rgb(139,69,19)".to_string(),
                    "salmon" => "rgb(250,128,114)".to_string(),
                    "sandybrown" => "rgb(244,164,96)".to_string(),
                    "seagreen" => "rgb(46,139,87)".to_string(),
                    "seashell" => "rgb(255,245,238)".to_string(),
                    "sienna" => "rgb(160,82,45)".to_string(),
                    "silver" => "rgb(192,192,192)".to_string(),
                    "skyblue" => "rgb(135,206,235)".to_string(),
                    "slateblue" => "rgb(106,90,205)".to_string(),
                    "slategray" | "slategrey" => "rgb(112,128,144)".to_string(),
                    "snow" => "rgb(255,250,250)".to_string(),
                    "springgreen" => "rgb(0,255,127)".to_string(),
                    "steelblue" => "rgb(70,130,180)".to_string(),
                    "tan" => "rgb(210,180,140)".to_string(),
                    "teal" => "rgb(0,128,128)".to_string(),
                    "thistle" => "rgb(216,191,216)".to_string(),
                    "tomato" => "rgb(255,99,71)".to_string(),
                    "turquoise" => "rgb(64,224,208)".to_string(),
                    "violet" => "rgb(238,130,238)".to_string(),
                    "wheat" => "rgb(245,222,179)".to_string(),
                    "white" => "rgb(255,255,255)".to_string(),
                    "whitesmoke" => "rgb(245,245,245)".to_string(),
                    "yellow" => "rgb(255,255,0)".to_string(),
                    "yellowgreen" => "rgb(154,205,50)".to_string(),
                    "none" | "off" => "none".to_string(),
                    _ => name.clone(),
                }
            }
        }
    }
}

impl std::str::FromStr for Color {
    type Err = std::convert::Infallible;

    /// Parse a color from a string. Always succeeds - unknown colors become Raw.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Normalize common color names (case-insensitive, handle aliases)
        let normalized = match s.to_lowercase().as_str() {
            "red" => "red",
            "blue" => "blue",
            "green" => "green",
            "yellow" => "yellow",
            "orange" => "orange",
            "purple" => "purple",
            "pink" => "pink",
            "black" => "black",
            "white" => "white",
            "gray" | "grey" => "gray",
            "lightgray" | "lightgrey" => "lightgray",
            "darkgray" | "darkgrey" => "darkgray",
            "cyan" => "cyan",
            "magenta" => "magenta",
            "none" | "off" => "none",
            _ => return Ok(Color::Raw(s.to_string())),
        };
        Ok(Color::Named(normalized.to_string()))
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::Named(s) | Color::Raw(s) => write!(f, "{}", s),
            Color::Rgb(r, g, b) => write!(f, "rgb({},{},{})", r, g, b),
            Color::Rgba(r, g, b, a) => write!(f, "rgba({},{},{},{})", r, g, b, a),
        }
    }
}

/// Convert inches → px with a given scale (C uses 144.0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scaler {
    pub r_scale: f64,
}

impl Scaler {
    /// Create a Scaler with validation (rejects NaN, infinite, zero, negative)
    pub fn try_new(r_scale: f64) -> Result<Self, NumericError> {
        if r_scale.is_nan() {
            Err(NumericError::NaN)
        } else if r_scale.is_infinite() {
            Err(NumericError::Infinite)
        } else if r_scale == 0.0 {
            Err(NumericError::Zero)
        } else if r_scale < 0.0 {
            Err(NumericError::Negative)
        } else {
            Ok(Scaler { r_scale })
        }
    }

    /// Convert a length in inches to pixels.
    pub fn len(&self, l: Length) -> Px {
        l.to_px(self.r_scale)
    }

    /// Convert a length in inches to raw f64 pixels (convenience for SVG output).
    #[inline]
    pub fn px(&self, l: Length) -> f64 {
        l.0 * self.r_scale
    }

    /// Convert a point in inches to pixels.
    pub fn point(&self, p: Point<Length>) -> Point<Px> {
        Point {
            x: self.len(p.x),
            y: self.len(p.y),
        }
    }

    /// Convert a size in inches to pixels.
    pub fn size(&self, s: Size<Length>) -> Size<Px> {
        Size {
            w: self.len(s.w),
            h: self.len(s.h),
        }
    }

    /// Convert a bounding box in inches to pixels.
    pub fn bbox(&self, b: BBox<Length>) -> BBox<Px> {
        BBox {
            min: self.point(b.min),
            max: self.point(b.max),
        }
    }
}

/// Generic 2D point
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Point<T> {
    pub x: T,
    pub y: T,
}

impl<T> Point<T> {
    pub fn new(x: T, y: T) -> Self {
        Point { x, y }
    }
}

impl Point<Length> {
    /// Origin point (0, 0)
    pub const ORIGIN: Self = Point {
        x: Length::ZERO,
        y: Length::ZERO,
    };

    /// Calculate the midpoint between two points
    pub fn midpoint(self, other: Self) -> Self {
        Point {
            x: (self.x + other.x) / 2.0,
            y: (self.y + other.y) / 2.0,
        }
    }

    /// Convert from pikchr coordinates (Y-up) to SVG pixels (Y-down).
    ///
    /// C pikchr stores coordinates with Y-up (mathematical convention) but
    /// flips Y during rendering with `y = bbox.ne.y - y`. This method does
    /// the same flip, converting from internal coordinates to SVG pixel space.
    ///
    /// # Arguments
    /// * `scaler` - Converts inches to pixels
    /// * `offset_x` - Horizontal offset to apply (usually `-bounds.min.x`)
    /// * `max_y` - The maximum Y value (`bounds.max.y`) for the Y-flip
    #[inline]
    pub fn to_svg(&self, scaler: &Scaler, offset_x: Length, max_y: Length) -> DVec2 {
        DVec2::new(
            scaler.px(self.x + offset_x),
            scaler.px(max_y - self.y), // Y-flip like C pikchr
        )
    }
}

/// 2D size
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Size<T> {
    pub w: T,
    pub h: T,
}

/// Axis-aligned bounding box
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BBox<T> {
    pub min: Point<T>,
    pub max: Point<T>,
}

impl Default for BBox<Length> {
    fn default() -> Self {
        Self::new()
    }
}

impl BBox<Length> {
    /// Create an empty bounding box (will expand on first point)
    pub fn new() -> Self {
        BBox {
            min: Point {
                x: Length(f64::MAX),
                y: Length(f64::MAX),
            },
            max: Point {
                x: Length(f64::MIN),
                y: Length(f64::MIN),
            },
        }
    }

    /// Check if the bbox is empty (never expanded)
    pub fn is_empty(&self) -> bool {
        self.min.x.0 > self.max.x.0 || self.min.y.0 > self.max.y.0
    }

    /// Expand to include a point
    pub fn expand_point(&mut self, p: Point<Length>) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    /// Expand to include a rectangle defined by center and size
    pub fn expand_rect(&mut self, center: Point<Length>, size: Size<Length>) {
        let hw = size.w / 2.0;
        let hh = size.h / 2.0;
        self.expand_point(Point {
            x: center.x - hw,
            y: center.y - hh,
        });
        self.expand_point(Point {
            x: center.x + hw,
            y: center.y + hh,
        });
    }

    /// Get the width as a typed Length
    pub fn width(&self) -> Length {
        self.max.x - self.min.x
    }

    /// Get the height as a typed Length
    pub fn height(&self) -> Length {
        self.max.y - self.min.y
    }

    /// Get the size as a typed `Size<Length>`
    pub fn size(&self) -> Size<Length> {
        Size {
            w: self.width(),
            h: self.height(),
        }
    }

    /// Get the center point
    pub fn center(&self) -> Point<Length> {
        Point {
            x: (self.min.x + self.max.x) / 2.0,
            y: (self.min.y + self.max.y) / 2.0,
        }
    }
}

/// A displacement/offset vector (not an absolute position)
/// Use this for translations; Point + Offset = Point
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Offset<T> {
    pub dx: T,
    pub dy: T,
}

impl<T> Offset<T> {
    pub fn new(dx: T, dy: T) -> Self {
        Offset { dx, dy }
    }
}

impl Offset<Length> {
    /// Zero offset
    pub const ZERO: Self = Offset {
        dx: Length::ZERO,
        dy: Length::ZERO,
    };
}

/// Alias for offset in inch space
pub type OffsetIn = Offset<Length>;

/// A unit direction vector (dimensionless, normalized)
/// Used for edge point offsets and directional calculations
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct UnitVec {
    dx: f64,
    dy: f64,
}

// 1/√2 for diagonal directions
const FRAC_1_SQRT_2: f64 = std::f64::consts::FRAC_1_SQRT_2;

impl UnitVec {
    pub const ZERO: UnitVec = UnitVec { dx: 0.0, dy: 0.0 };
    // Y-up convention: North = +Y, South = -Y
    pub const NORTH: UnitVec = UnitVec { dx: 0.0, dy: 1.0 };
    pub const SOUTH: UnitVec = UnitVec { dx: 0.0, dy: -1.0 };
    pub const EAST: UnitVec = UnitVec { dx: 1.0, dy: 0.0 };
    pub const WEST: UnitVec = UnitVec { dx: -1.0, dy: 0.0 };
    pub const NORTH_EAST: UnitVec = UnitVec {
        dx: FRAC_1_SQRT_2,
        dy: FRAC_1_SQRT_2,
    };
    pub const NORTH_WEST: UnitVec = UnitVec {
        dx: -FRAC_1_SQRT_2,
        dy: FRAC_1_SQRT_2,
    };
    pub const SOUTH_EAST: UnitVec = UnitVec {
        dx: FRAC_1_SQRT_2,
        dy: -FRAC_1_SQRT_2,
    };
    pub const SOUTH_WEST: UnitVec = UnitVec {
        dx: -FRAC_1_SQRT_2,
        dy: -FRAC_1_SQRT_2,
    };

    /// Create a normalized unit vector from components.
    /// Returns None if the input has zero length.
    pub fn normalized(dx: f64, dy: f64) -> Option<Self> {
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            None
        } else {
            Some(UnitVec {
                dx: dx / len,
                dy: dy / len,
            })
        }
    }

    /// Get dx component
    pub fn dx(self) -> f64 {
        self.dx
    }

    /// Get dy component
    pub fn dy(self) -> f64 {
        self.dy
    }

    /// Scale by different amounts in x and y (for non-square shapes)
    /// Returns an offset with dx scaled by `sx` and dy scaled by `sy`
    pub fn scale_xy(self, sx: Length, sy: Length) -> Offset<Length> {
        Offset {
            dx: sx * self.dx,
            dy: sy * self.dy,
        }
    }
}

/// Multiply a unit vector by a length to get an offset (not a point!)
impl Mul<Length> for UnitVec {
    type Output = Offset<Length>;
    fn mul(self, len: Length) -> Offset<Length> {
        Offset {
            dx: Length(self.dx * len.0),
            dy: Length(self.dy * len.0),
        }
    }
}

/// Add an offset to a point to get a new point
impl Add<Offset<Length>> for Point<Length> {
    type Output = Point<Length>;
    fn add(self, rhs: Offset<Length>) -> Point<Length> {
        Point {
            x: self.x + rhs.dx,
            y: self.y + rhs.dy,
        }
    }
}

/// Subtract two points to get an offset
impl Sub<Point<Length>> for Point<Length> {
    type Output = Offset<Length>;
    fn sub(self, rhs: Point<Length>) -> Offset<Length> {
        Offset {
            dx: self.x - rhs.x,
            dy: self.y - rhs.y,
        }
    }
}

/// Add two offsets
impl Add<Offset<Length>> for Offset<Length> {
    type Output = Offset<Length>;
    fn add(self, rhs: Offset<Length>) -> Offset<Length> {
        Offset {
            dx: self.dx + rhs.dx,
            dy: self.dy + rhs.dy,
        }
    }
}

/// AddAssign for Offset accumulation
impl AddAssign<Offset<Length>> for Offset<Length> {
    fn add_assign(&mut self, rhs: Offset<Length>) {
        self.dx += rhs.dx;
        self.dy += rhs.dy;
    }
}

/// AddAssign offset to point (translate in place)
impl AddAssign<Offset<Length>> for Point<Length> {
    fn add_assign(&mut self, rhs: Offset<Length>) {
        self.x += rhs.dx;
        self.y += rhs.dy;
    }
}

/// Subtract offset from point to get a new point
impl Sub<Offset<Length>> for Point<Length> {
    type Output = Point<Length>;
    fn sub(self, rhs: Offset<Length>) -> Point<Length> {
        Point {
            x: self.x - rhs.dx,
            y: self.y - rhs.dy,
        }
    }
}

/// SubAssign offset from point (translate in place)
impl SubAssign<Offset<Length>> for Point<Length> {
    fn sub_assign(&mut self, rhs: Offset<Length>) {
        self.x -= rhs.dx;
        self.y -= rhs.dy;
    }
}

/// Subtract two offsets
impl Sub<Offset<Length>> for Offset<Length> {
    type Output = Offset<Length>;
    fn sub(self, rhs: Offset<Length>) -> Offset<Length> {
        Offset {
            dx: self.dx - rhs.dx,
            dy: self.dy - rhs.dy,
        }
    }
}

/// Negate an offset
impl Neg for Offset<Length> {
    type Output = Offset<Length>;
    fn neg(self) -> Offset<Length> {
        Offset {
            dx: -self.dx,
            dy: -self.dy,
        }
    }
}

/// Scale an offset by a scalar
impl Mul<f64> for Offset<Length> {
    type Output = Offset<Length>;
    fn mul(self, rhs: f64) -> Offset<Length> {
        Offset {
            dx: self.dx * rhs,
            dy: self.dy * rhs,
        }
    }
}

/// Scale an offset by a Scalar
impl Mul<Scalar> for Offset<Length> {
    type Output = Offset<Length>;
    fn mul(self, rhs: Scalar) -> Offset<Length> {
        Offset {
            dx: self.dx * rhs.0,
            dy: self.dy * rhs.0,
        }
    }
}

/// Convenient aliases
pub type PtIn = Point<Length>;
pub type PtPx = Point<Px>;
pub type BoxIn = BBox<Length>;

// ==================== Semantic Aliases (from TYPES.md) ====================

/// Absolute position in inch space.
///
/// This is the semantic name for `Point<Length>`.
/// Use `Pos2` when you mean "a location in the diagram".
pub type Pos2 = Point<Length>;

/// Relative offset/displacement in inch space.
///
/// This is the semantic name for `Offset<Length>`.
/// Use `Vec2` when you mean "a distance and direction" (not a location).
///
/// Key operations:
/// - `Pos2 + Vec2 = Pos2` (translate a position)
/// - `Pos2 - Pos2 = Vec2` (displacement between positions)
/// - `Vec2 + Vec2 = Vec2` (combine offsets)
pub type Vec2 = Offset<Length>;

/// Size in inches (always non-negative conceptually).
pub type Size2 = Size<Length>;

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Length tests ====================

    #[test]
    fn length_try_new_valid() {
        assert!(Length::try_new(1.0).is_ok());
        assert!(Length::try_new(0.0).is_ok());
        assert!(Length::try_new(-1.0).is_ok());
    }

    #[test]
    fn length_try_new_rejects_nan() {
        assert_eq!(Length::try_new(f64::NAN), Err(NumericError::NaN));
    }

    #[test]
    fn length_try_new_rejects_infinity() {
        assert_eq!(Length::try_new(f64::INFINITY), Err(NumericError::Infinite));
        assert_eq!(
            Length::try_new(f64::NEG_INFINITY),
            Err(NumericError::Infinite)
        );
    }

    #[test]
    fn length_try_non_negative_valid() {
        assert!(Length::try_non_negative(1.0).is_ok());
        assert!(Length::try_non_negative(0.0).is_ok());
    }

    #[test]
    fn length_try_non_negative_rejects_negative() {
        assert_eq!(Length::try_non_negative(-1.0), Err(NumericError::Negative));
    }

    #[test]
    fn length_arithmetic() {
        let a = Length(3.0);
        let b = Length(2.0);

        assert_eq!(a + b, Length(5.0));
        assert_eq!(a - b, Length(1.0));
        assert_eq!(a * 2.0, Length(6.0));
        assert_eq!(a / 2.0, Length(1.5));
        assert_eq!(-a, Length(-3.0));
    }

    #[test]
    fn length_min_max() {
        let a = Length(3.0);
        let b = Length(5.0);

        assert_eq!(a.min(b), Length(3.0));
        assert_eq!(a.max(b), Length(5.0));
    }

    #[test]
    fn length_checked_div_valid() {
        let a = Length(6.0);
        let b = Length(2.0);
        assert_eq!(a.checked_div(b), Some(Scalar(3.0)));
    }

    #[test]
    fn length_checked_div_by_zero() {
        let a = Length(6.0);
        let b = Length(0.0);
        assert_eq!(a.checked_div(b), None);
    }

    #[test]
    fn length_is_finite() {
        assert!(Length(1.0).is_finite());
        assert!(!Length(f64::INFINITY).is_finite());
        assert!(!Length(f64::NAN).is_finite());
    }

    // ==================== Scalar tests ====================

    #[test]
    fn scalar_mul_length() {
        let s = Scalar(2.0);
        let l = Length(3.0);
        assert_eq!(s * l, Length(6.0));
        assert_eq!(l * s, Length(6.0));
    }

    #[test]
    fn scalar_is_finite() {
        assert!(Scalar(1.0).is_finite());
        assert!(!Scalar(f64::INFINITY).is_finite());
    }

    // ==================== Scaler tests ====================

    #[test]
    fn scaler_try_new_valid() {
        assert!(Scaler::try_new(144.0).is_ok());
        assert!(Scaler::try_new(1.0).is_ok());
    }

    #[test]
    fn scaler_try_new_rejects_zero() {
        assert_eq!(Scaler::try_new(0.0), Err(NumericError::Zero));
    }

    #[test]
    fn scaler_try_new_rejects_negative() {
        assert_eq!(Scaler::try_new(-1.0), Err(NumericError::Negative));
    }

    #[test]
    fn scaler_try_new_rejects_nan() {
        assert_eq!(Scaler::try_new(f64::NAN), Err(NumericError::NaN));
    }

    #[test]
    fn scaler_try_new_rejects_infinity() {
        assert_eq!(Scaler::try_new(f64::INFINITY), Err(NumericError::Infinite));
    }

    #[test]
    fn scaler_converts_length_to_px() {
        let scaler = Scaler::try_new(144.0).unwrap();
        let len = Length(1.0); // 1 inch
        assert_eq!(scaler.px(len), 144.0); // 144 pixels
    }

    // ==================== UnitVec tests ====================

    #[test]
    fn unitvec_cardinal_directions_are_unit_length() {
        let dirs = [UnitVec::NORTH, UnitVec::SOUTH, UnitVec::EAST, UnitVec::WEST];
        for dir in dirs {
            let len = (dir.dx() * dir.dx() + dir.dy() * dir.dy()).sqrt();
            assert!(
                (len - 1.0).abs() < 1e-10,
                "cardinal direction should have unit length"
            );
        }
    }

    #[test]
    fn unitvec_diagonal_directions_are_unit_length() {
        let dirs = [
            UnitVec::NORTH_EAST,
            UnitVec::NORTH_WEST,
            UnitVec::SOUTH_EAST,
            UnitVec::SOUTH_WEST,
        ];
        for dir in dirs {
            let len = (dir.dx() * dir.dx() + dir.dy() * dir.dy()).sqrt();
            assert!(
                (len - 1.0).abs() < 1e-10,
                "diagonal direction should have unit length"
            );
        }
    }

    #[test]
    fn unitvec_normalized_valid() {
        let v = UnitVec::normalized(3.0, 4.0);
        assert!(v.is_some());
        let v = v.unwrap();
        let len = (v.dx() * v.dx() + v.dy() * v.dy()).sqrt();
        assert!((len - 1.0).abs() < 1e-10);
        assert!((v.dx() - 0.6).abs() < 1e-10);
        assert!((v.dy() - 0.8).abs() < 1e-10);
    }

    #[test]
    fn unitvec_normalized_zero_returns_none() {
        assert_eq!(UnitVec::normalized(0.0, 0.0), None);
    }

    #[test]
    fn unitvec_mul_length_gives_offset() {
        let dir = UnitVec::EAST;
        let len = Length(5.0);
        let offset = dir * len;
        assert_eq!(offset.dx, Length(5.0));
        assert_eq!(offset.dy, Length(0.0));
    }

    // ==================== Point/Offset tests ====================

    #[test]
    fn point_plus_offset_gives_point() {
        let p = Point::new(Length(1.0), Length(2.0));
        let o = Offset::new(Length(3.0), Length(4.0));
        let result = p + o;
        assert_eq!(result.x, Length(4.0));
        assert_eq!(result.y, Length(6.0));
    }

    #[test]
    fn point_minus_point_gives_offset() {
        let p1 = Point::new(Length(5.0), Length(7.0));
        let p2 = Point::new(Length(2.0), Length(3.0));
        let offset = p1 - p2;
        assert_eq!(offset.dx, Length(3.0));
        assert_eq!(offset.dy, Length(4.0));
    }

    #[test]
    fn point_midpoint() {
        let p1 = Point::new(Length(0.0), Length(0.0));
        let p2 = Point::new(Length(4.0), Length(6.0));
        let mid = p1.midpoint(p2);
        assert_eq!(mid.x, Length(2.0));
        assert_eq!(mid.y, Length(3.0));
    }

    // ==================== BBox tests ====================

    #[test]
    fn bbox_new_is_empty() {
        let bb = BBox::<Length>::new();
        assert!(bb.is_empty());
    }

    #[test]
    fn bbox_expand_point() {
        let mut bb = BBox::<Length>::new();
        bb.expand_point(Point::new(Length(1.0), Length(2.0)));
        bb.expand_point(Point::new(Length(3.0), Length(4.0)));

        assert!(!bb.is_empty());
        assert_eq!(bb.min.x, Length(1.0));
        assert_eq!(bb.min.y, Length(2.0));
        assert_eq!(bb.max.x, Length(3.0));
        assert_eq!(bb.max.y, Length(4.0));
    }

    #[test]
    fn bbox_width_height() {
        let mut bb = BBox::<Length>::new();
        bb.expand_point(Point::new(Length(1.0), Length(2.0)));
        bb.expand_point(Point::new(Length(5.0), Length(8.0)));

        assert_eq!(bb.width(), Length(4.0));
        assert_eq!(bb.height(), Length(6.0));
    }

    #[test]
    fn bbox_center() {
        let mut bb = BBox::<Length>::new();
        bb.expand_point(Point::new(Length(0.0), Length(0.0)));
        bb.expand_point(Point::new(Length(4.0), Length(6.0)));

        let center = bb.center();
        assert_eq!(center.x, Length(2.0));
        assert_eq!(center.y, Length(3.0));
    }

    #[test]
    fn bbox_expand_rect() {
        let mut bb = BBox::<Length>::new();
        bb.expand_rect(
            Point::new(Length(5.0), Length(5.0)),
            Size {
                w: Length(4.0),
                h: Length(2.0),
            },
        );

        // center (5,5), width 4, height 2 -> min (3,4), max (7,6)
        assert_eq!(bb.min.x, Length(3.0));
        assert_eq!(bb.min.y, Length(4.0));
        assert_eq!(bb.max.x, Length(7.0));
        assert_eq!(bb.max.y, Length(6.0));
    }

    // ==================== Angle tests ====================

    #[test]
    fn angle_try_new_valid() {
        assert!(Angle::try_new(0.0).is_ok());
        assert!(Angle::try_new(90.0).is_ok());
        assert!(Angle::try_new(-45.0).is_ok());
        assert!(Angle::try_new(360.0).is_ok());
    }

    #[test]
    fn angle_try_new_rejects_nan() {
        assert_eq!(Angle::try_new(f64::NAN), Err(NumericError::NaN));
    }

    #[test]
    fn angle_try_new_rejects_infinity() {
        assert_eq!(Angle::try_new(f64::INFINITY), Err(NumericError::Infinite));
        assert_eq!(
            Angle::try_new(f64::NEG_INFINITY),
            Err(NumericError::Infinite)
        );
    }

    #[test]
    fn angle_from_radians_valid() {
        let a = Angle::from_radians(std::f64::consts::PI).unwrap();
        assert!((a.raw() - 180.0).abs() < 1e-10);
    }

    #[test]
    fn angle_from_radians_rejects_nan() {
        assert_eq!(Angle::from_radians(f64::NAN), Err(NumericError::NaN));
    }

    #[test]
    fn angle_to_radians() {
        let a = Angle::degrees(180.0);
        assert!((a.to_radians() - std::f64::consts::PI).abs() < 1e-10);
    }

    #[test]
    fn angle_normalized() {
        assert!((Angle::degrees(450.0).normalized().raw() - 90.0).abs() < 1e-10);
        assert!((Angle::degrees(-90.0).normalized().raw() - 270.0).abs() < 1e-10);
        assert!((Angle::degrees(0.0).normalized().raw() - 0.0).abs() < 1e-10);
        assert!((Angle::degrees(360.0).normalized().raw() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn angle_is_finite() {
        assert!(Angle::degrees(45.0).is_finite());
        assert!(!Angle::degrees(f64::INFINITY).is_finite());
        assert!(!Angle::degrees(f64::NAN).is_finite());
    }
}
