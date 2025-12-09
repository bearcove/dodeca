//! Default sizes and settings (all in inches to mirror C implementation)

use crate::types::Length as Inches;

pub const LINE_WIDTH: Inches = Inches::inches(0.5);
pub const BOX_WIDTH: Inches = Inches::inches(0.75);
pub const BOX_HEIGHT: Inches = Inches::inches(0.5);
pub const FILE_WIDTH: Inches = Inches::inches(0.5);
pub const FILE_HEIGHT: Inches = Inches::inches(0.75);
pub const FILE_RAD: Inches = Inches::inches(0.15);
pub const OVAL_WIDTH: Inches = Inches::inches(1.0);
pub const OVAL_HEIGHT: Inches = Inches::inches(0.5);
pub const DIAMOND_WIDTH: Inches = Inches::inches(1.0);
pub const DIAMOND_HEIGHT: Inches = Inches::inches(0.75);
pub const CIRCLE_RADIUS: Inches = Inches::inches(0.25);
pub const STROKE_WIDTH: Inches = Inches::inches(0.015);
pub const FONT_SIZE: f64 = 0.14;
pub const MARGIN: f64 = 0.0;
pub const CHARWID: f64 = 0.08;
