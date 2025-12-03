//! Strongly-typed numeric primitives for pikru (zero-cost newtypes).
//!
//! Design goals (from STRONG_TYPES.md):
//! - No raw `f64` in domain logic
//! - Illegal states unrepresentable
//! - Conversions only via Scaler

use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub, AddAssign, SubAssign};

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

    /// Create a Length from inches (const-friendly, unchecked)
    #[inline]
    pub const fn inches(val: f64) -> Length {
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
    fn add(self, rhs: Length) -> Length { Length(self.0 + rhs.0) }
}
impl Sub for Length {
    type Output = Length;
    fn sub(self, rhs: Length) -> Length { Length(self.0 - rhs.0) }
}
impl Mul<f64> for Length {
    type Output = Length;
    fn mul(self, rhs: f64) -> Length { Length(self.0 * rhs) }
}
impl Div<f64> for Length {
    type Output = Length;
    fn div(self, rhs: f64) -> Length { Length(self.0 / rhs) }
}

impl Div<Length> for Length {
    type Output = Scalar;
    /// Dividing two lengths gives a dimensionless scalar.
    /// Note: Returns Scalar(inf) if rhs is zero. Use `checked_div` for safe division.
    fn div(self, rhs: Length) -> Scalar { Scalar(self.0 / rhs.0) }
}

impl Neg for Length {
    type Output = Length;
    fn neg(self) -> Length { Length(-self.0) }
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

/// Angle in degrees
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default)]
pub struct Angle(pub f64);

impl fmt::Display for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
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
#[derive(Clone, Copy, Debug)]
pub struct Scaler {
    pub r_scale: f64,
}

impl Scaler {
    /// Create a new Scaler (unchecked)
    pub fn new(r_scale: f64) -> Self { Scaler { r_scale } }

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
    pub fn len(&self, l: Length) -> Px { l.to_px(self.r_scale) }

    /// Convert a length in inches to raw f64 pixels (convenience for SVG output).
    #[inline]
    pub fn px(&self, l: Length) -> f64 { l.0 * self.r_scale }

    /// Convert a point in inches to pixels.
    pub fn point(&self, p: Point<Length>) -> Point<Px> {
        Point { x: self.len(p.x), y: self.len(p.y) }
    }

    /// Convert a size in inches to pixels.
    pub fn size(&self, s: Size<Length>) -> Size<Px> {
        Size { w: self.len(s.w), h: self.len(s.h) }
    }

    /// Convert a bounding box in inches to pixels.
    pub fn bbox(&self, b: BBox<Length>) -> BBox<Px> {
        BBox { min: self.point(b.min), max: self.point(b.max) }
    }
}

/// Generic 2D point
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Point<T> {
    pub x: T,
    pub y: T,
}

impl<T> Point<T> {
    pub fn new(x: T, y: T) -> Self { Point { x, y } }
}

impl Point<Length> {
    /// Calculate the midpoint between two points
    pub fn midpoint(self, other: Self) -> Self {
        Point {
            x: (self.x + other.x) / 2.0,
            y: (self.y + other.y) / 2.0,
        }
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

impl BBox<Length> {
    /// Create an empty bounding box (will expand on first point)
    pub fn new() -> Self {
        BBox {
            min: Point { x: Length(f64::MAX), y: Length(f64::MAX) },
            max: Point { x: Length(f64::MIN), y: Length(f64::MIN) },
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
        self.expand_point(Point { x: center.x - hw, y: center.y - hh });
        self.expand_point(Point { x: center.x + hw, y: center.y + hh });
    }

    /// Get the width as a typed Length
    pub fn width(&self) -> Length { self.max.x - self.min.x }

    /// Get the height as a typed Length
    pub fn height(&self) -> Length { self.max.y - self.min.y }

    /// Get the size as a typed Size<Length>
    pub fn size(&self) -> Size<Length> {
        Size { w: self.width(), h: self.height() }
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
    pub const NORTH: UnitVec = UnitVec { dx: 0.0, dy: -1.0 };
    pub const SOUTH: UnitVec = UnitVec { dx: 0.0, dy: 1.0 };
    pub const EAST: UnitVec = UnitVec { dx: 1.0, dy: 0.0 };
    pub const WEST: UnitVec = UnitVec { dx: -1.0, dy: 0.0 };
    pub const NORTH_EAST: UnitVec = UnitVec { dx: FRAC_1_SQRT_2, dy: -FRAC_1_SQRT_2 };
    pub const NORTH_WEST: UnitVec = UnitVec { dx: -FRAC_1_SQRT_2, dy: -FRAC_1_SQRT_2 };
    pub const SOUTH_EAST: UnitVec = UnitVec { dx: FRAC_1_SQRT_2, dy: FRAC_1_SQRT_2 };
    pub const SOUTH_WEST: UnitVec = UnitVec { dx: -FRAC_1_SQRT_2, dy: FRAC_1_SQRT_2 };

    /// Create a normalized unit vector from components.
    /// Returns ZERO if the input has zero length.
    pub fn normalized(dx: f64, dy: f64) -> Self {
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            UnitVec::ZERO
        } else {
            UnitVec { dx: dx / len, dy: dy / len }
        }
    }

    /// Get dx component
    pub fn dx(self) -> f64 { self.dx }

    /// Get dy component
    pub fn dy(self) -> f64 { self.dy }
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

/// Convenient aliases
pub type PtIn = Point<Length>;
pub type PtPx = Point<Px>;
pub type BoxIn = BBox<Length>;
