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
    pub fn point(&self, p: crate::render::Point) -> (f64, f64) {
        ((p.x.0) * self.r_scale, (p.y.0) * self.r_scale)
    }
}
