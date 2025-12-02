//! Tokyo Night color theme for the TUI
//!
//! Based on https://github.com/enkia/tokyo-night-vscode-theme
//!
//! Use the `TokyoNight` extension trait for semantic styling:

#![allow(dead_code)] // Theme palette - not all colors used yet
//! ```ignore
//! use crate::theme::TokyoNight;
//! Span::raw("/path/to/file").tn_path()
//! Span::raw("https://example.com").tn_url()
//! ```

use ratatui::style::{Color, Stylize};
use ratatui::text::Span;

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

/// Extension trait for Tokyo Night semantic styling
pub trait TokyoNight<'a> {
    // Base colors
    fn tn_blue(self) -> Span<'a>;
    fn tn_cyan(self) -> Span<'a>;
    fn tn_green(self) -> Span<'a>;
    fn tn_magenta(self) -> Span<'a>;
    fn tn_red(self) -> Span<'a>;
    fn tn_yellow(self) -> Span<'a>;
    fn tn_orange(self) -> Span<'a>;
    fn tn_purple(self) -> Span<'a>;
    fn tn_teal(self) -> Span<'a>;
    fn tn_pink(self) -> Span<'a>;

    // Foreground shades
    fn tn_fg(self) -> Span<'a>;
    fn tn_fg_dark(self) -> Span<'a>;
    fn tn_muted(self) -> Span<'a>;

    // Semantic styles
    fn tn_path(self) -> Span<'a>;
    fn tn_url(self) -> Span<'a>;
    fn tn_file_change(self) -> Span<'a>;
    fn tn_reload(self) -> Span<'a>;
    fn tn_patch(self) -> Span<'a>;
    fn tn_search(self) -> Span<'a>;
    fn tn_server(self) -> Span<'a>;
    fn tn_build(self) -> Span<'a>;
    fn tn_timing(self) -> Span<'a>;
    fn tn_success(self) -> Span<'a>;
    fn tn_warning(self) -> Span<'a>;
    fn tn_error(self) -> Span<'a>;
    fn tn_info(self) -> Span<'a>;
    fn tn_hint(self) -> Span<'a>;
}

impl<'a> TokyoNight<'a> for Span<'a> {
    fn tn_blue(self) -> Span<'a> { self.fg(BLUE) }
    fn tn_cyan(self) -> Span<'a> { self.fg(CYAN) }
    fn tn_green(self) -> Span<'a> { self.fg(GREEN) }
    fn tn_magenta(self) -> Span<'a> { self.fg(MAGENTA) }
    fn tn_red(self) -> Span<'a> { self.fg(RED) }
    fn tn_yellow(self) -> Span<'a> { self.fg(YELLOW) }
    fn tn_orange(self) -> Span<'a> { self.fg(ORANGE) }
    fn tn_purple(self) -> Span<'a> { self.fg(PURPLE) }
    fn tn_teal(self) -> Span<'a> { self.fg(TEAL) }
    fn tn_pink(self) -> Span<'a> { self.fg(PINK) }

    fn tn_fg(self) -> Span<'a> { self.fg(FG) }
    fn tn_fg_dark(self) -> Span<'a> { self.fg(FG_DARK) }
    fn tn_muted(self) -> Span<'a> { self.fg(FG_GUTTER) }

    fn tn_path(self) -> Span<'a> { self.fg(MAGENTA) }
    fn tn_url(self) -> Span<'a> { self.fg(BLUE) }
    fn tn_file_change(self) -> Span<'a> { self.fg(ORANGE) }
    fn tn_reload(self) -> Span<'a> { self.fg(YELLOW) }
    fn tn_patch(self) -> Span<'a> { self.fg(GREEN) }
    fn tn_search(self) -> Span<'a> { self.fg(CYAN) }
    fn tn_server(self) -> Span<'a> { self.fg(BLUE) }
    fn tn_build(self) -> Span<'a> { self.fg(PURPLE) }
    fn tn_timing(self) -> Span<'a> { self.fg(ORANGE) }
    fn tn_success(self) -> Span<'a> { self.fg(GREEN) }
    fn tn_warning(self) -> Span<'a> { self.fg(YELLOW) }
    fn tn_error(self) -> Span<'a> { self.fg(RED) }
    fn tn_info(self) -> Span<'a> { self.fg(BLUE) }
    fn tn_hint(self) -> Span<'a> { self.fg(TEAL) }
}

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

/// Get symbol and color for an HTTP status code
pub fn http_status_style(status: u16) -> (&'static str, Color) {
    match status {
        500..=599 => ("⚠", RED),
        400..=499 => ("✗", YELLOW),
        300..=399 => ("↪", CYAN),
        200..=299 => ("→", GREEN),
        _ => ("•", FG_DARK),
    }
}
