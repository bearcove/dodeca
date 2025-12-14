//! Tokyo Night color theme for the TUI

#![allow(dead_code)]

use ratatui::style::Color;

// Background colors
pub const BG: Color = Color::Rgb(0x1a, 0x1b, 0x26);
pub const BG_DARK: Color = Color::Rgb(0x16, 0x16, 0x1e);
pub const BG_HIGHLIGHT: Color = Color::Rgb(0x29, 0x2e, 0x42);

// Foreground colors
pub const FG: Color = Color::Rgb(0xa9, 0xb1, 0xd6);
pub const FG_DARK: Color = Color::Rgb(0x56, 0x5f, 0x89);
pub const FG_GUTTER: Color = Color::Rgb(0x3b, 0x40, 0x61);

// Accent colors
pub const BLUE: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
pub const CYAN: Color = Color::Rgb(0x7d, 0xcf, 0xff);
pub const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);
pub const MAGENTA: Color = Color::Rgb(0xbb, 0x9a, 0xf7);
pub const RED: Color = Color::Rgb(0xf7, 0x76, 0x8e);
pub const YELLOW: Color = Color::Rgb(0xe0, 0xaf, 0x68);
pub const ORANGE: Color = Color::Rgb(0xff, 0x9e, 0x64);
pub const PURPLE: Color = Color::Rgb(0x9d, 0x7c, 0xd8);
pub const TEAL: Color = Color::Rgb(0x73, 0xda, 0xca);
pub const PINK: Color = Color::Rgb(0xff, 0x75, 0xa0);

/// Get the color for an HTTP status code
pub fn http_status_color(status: u16) -> Color {
    match status {
        500..=599 => RED,
        400..=499 => YELLOW,
        300..=399 => CYAN,
        200..=299 => GREEN,
        100..=199 => BLUE,
        _ => FG_DARK,
    }
}
