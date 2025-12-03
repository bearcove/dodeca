//! Strongly-typed numeric primitives for pikru (zero-cost newtypes).
//! Currently unused; will be wired incrementally to replace raw f64s.

use std::fmt;
use std::ops::{Add, Div, Mul, Sub, AddAssign, SubAssign};

/// Length in inches (pikchr canonical unit)
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default)]
pub struct Length(pub f64);

impl Length {
    pub fn to_px(self, r_scale: f64) -> Px {
        Px(self.0 * r_scale)
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

impl AddAssign<f64> for Length {
    fn add_assign(&mut self, rhs: f64) {
        self.0 += rhs;
    }
}

impl SubAssign for Length {
    fn sub_assign(&mut self, rhs: Length) {
        self.0 -= rhs.0;
    }
}

impl SubAssign<f64> for Length {
    fn sub_assign(&mut self, rhs: f64) {
        self.0 -= rhs;
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

/// Unitless scalar (for percentages, counts, etc.)
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default)]
pub struct Scalar(pub f64);

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
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
    pub fn new(r_scale: f64) -> Self { Scaler { r_scale } }

    pub fn len(&self, l: Length) -> Px { l.to_px(self.r_scale) }
    pub fn point(&self, p: Point<Length>) -> Point<Px> {
        Point { x: self.len(p.x), y: self.len(p.y) }
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
    pub fn new() -> Self {
        BBox {
            min: Point { x: Length(f64::MAX), y: Length(f64::MAX) },
            max: Point { x: Length(f64::MIN), y: Length(f64::MIN) },
        }
    }

    pub fn expand_point(&mut self, p: Point<Length>) {
        self.min.x = Length(self.min.x.0.min(p.x.0));
        self.min.y = Length(self.min.y.0.min(p.y.0));
        self.max.x = Length(self.max.x.0.max(p.x.0));
        self.max.y = Length(self.max.y.0.max(p.y.0));
    }

    pub fn expand_rect(&mut self, center: Point<Length>, size: Size<Length>) {
        let hw = size.w.0 / 2.0;
        let hh = size.h.0 / 2.0;
        self.expand_point(Point { x: Length(center.x.0 - hw), y: Length(center.y.0 - hh) });
        self.expand_point(Point { x: Length(center.x.0 + hw), y: Length(center.y.0 + hh) });
    }

    pub fn width(&self) -> f64 { self.max.x.0 - self.min.x.0 }
    pub fn height(&self) -> f64 { self.max.y.0 - self.min.y.0 }
}

/// Convenient aliases
pub type PtIn = Point<Length>;
pub type PtPx = Point<Px>;
pub type BoxIn = BBox<Length>;
