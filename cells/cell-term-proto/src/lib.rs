//! RPC protocol for dodeca term cell
//!
//! Defines services for terminal session recording with ANSI color support.

use facet::Facet;
use std::fmt::Write;

/// Configuration for recording a terminal session.
#[derive(Debug, Clone, Facet)]
pub struct RecordConfig {
    /// Shell to use (if None, uses $SHELL or /bin/sh)
    pub shell: Option<String>,
}

/// Result of a terminal recording operation.
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum TermResult {
    /// Successfully recorded session
    Success {
        /// HTML output with <t-*> tags for styling
        html: String,
    },
    /// Error during recording
    Error { message: String },
}

/// Terminal recording service implemented by the cell.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait TermRecorder {
    /// Record a terminal session interactively.
    /// User controls the session, exits when done.
    async fn record_interactive(&self, config: RecordConfig) -> TermResult;

    /// Record with an auto-executed command.
    /// Spawns shell, runs command, captures output until prompt returns.
    async fn record_command(&self, command: String, config: RecordConfig) -> TermResult;
}

/// Generate CSS for terminal styling with <t-*> custom tags.
///
/// Includes:
/// - Attribute tags: t-b (bold), t-l (light), t-i (italic), t-u (underline), t-st (strikethrough)
/// - 16 classic foreground colors: t-fblk, t-fred, t-fgrn, etc.
/// - 16 classic background colors: t-bblk, t-bred, t-bgrn, etc.
/// - 256 ANSI palette: t-f0..t-f255, t-b0..t-b255
/// - RGB support via CSS custom property: t-f, t-b with --c variable
pub fn generate_css() -> String {
    let mut css = String::new();

    writeln!(css, "/* Terminal styling for <t-*> custom tags */").unwrap();
    writeln!(css).unwrap();

    // Attribute tags
    writeln!(css, "/* Attributes */").unwrap();
    writeln!(css, "t-b {{ font-weight: bold; }}").unwrap();
    writeln!(css, "t-l {{ opacity: 0.7; }}").unwrap();
    writeln!(css, "t-i {{ font-style: italic; }}").unwrap();
    writeln!(css, "t-u {{ text-decoration: underline; }}").unwrap();
    writeln!(css, "t-st {{ text-decoration: line-through; }}").unwrap();
    writeln!(css).unwrap();

    // Classic 16 foreground colors with light-dark()
    writeln!(css, "/* Classic 16 foreground colors */").unwrap();
    let classic_fg = [
        ("fblk", "#292929", "#6e6e6e"),
        ("fred", "#c41a16", "#ff6b6b"),
        ("fgrn", "#007400", "#5af78e"),
        ("fylw", "#826b00", "#f3f99d"),
        ("fblu", "#0000d6", "#57c7ff"),
        ("fmag", "#a90d91", "#ff6ac1"),
        ("fcyn", "#007482", "#9aedfe"),
        ("fwht", "#6e6e6e", "#f1f1f0"),
        // Light variants
        ("flblk", "#5c5c5c", "#8e8e8e"),
        ("flred", "#ff6b6b", "#ff9d9d"),
        ("flgrn", "#5af78e", "#98ffbd"),
        ("flylw", "#f3f99d", "#ffffb3"),
        ("flblu", "#57c7ff", "#9dd9ff"),
        ("flmag", "#ff6ac1", "#ff9dd9"),
        ("flcyn", "#9aedfe", "#c0f5ff"),
        ("flwht", "#f1f1f0", "#ffffff"),
    ];
    for (name, light, dark) in classic_fg {
        writeln!(css, "t-{name} {{ color: light-dark({light}, {dark}); }}").unwrap();
    }
    writeln!(css).unwrap();

    // Classic 16 background colors
    writeln!(css, "/* Classic 16 background colors */").unwrap();
    let classic_bg = [
        ("bblk", "#292929", "#6e6e6e"),
        ("bred", "#c41a16", "#ff6b6b"),
        ("bgrn", "#007400", "#5af78e"),
        ("bylw", "#826b00", "#f3f99d"),
        ("bblu", "#0000d6", "#57c7ff"),
        ("bmag", "#a90d91", "#ff6ac1"),
        ("bcyn", "#007482", "#9aedfe"),
        ("bwht", "#6e6e6e", "#f1f1f0"),
        // Light variants
        ("blblk", "#5c5c5c", "#8e8e8e"),
        ("blred", "#ff6b6b", "#ff9d9d"),
        ("blgrn", "#5af78e", "#98ffbd"),
        ("blylw", "#f3f99d", "#ffffb3"),
        ("blblu", "#57c7ff", "#9dd9ff"),
        ("blmag", "#ff6ac1", "#ff9dd9"),
        ("blcyn", "#9aedfe", "#c0f5ff"),
        ("blwht", "#f1f1f0", "#ffffff"),
    ];
    for (name, light, dark) in classic_bg {
        writeln!(
            css,
            "t-{name} {{ background-color: light-dark({light}, {dark}); }}"
        )
        .unwrap();
    }
    writeln!(css).unwrap();

    // 256-color palette
    writeln!(css, "/* 256-color ANSI palette */").unwrap();
    for i in 0..=255u8 {
        let (r, g, b) = ansi_256_to_rgb(i);
        let hex = format!("#{r:02x}{g:02x}{b:02x}");

        // Adjust for light/dark mode
        let (light_hex, dark_hex) = if i < 16 {
            // Use classic colors for 0-15
            (hex.clone(), hex.clone())
        } else {
            // Darken for light mode, brighten for dark mode
            let (lr, lg, lb) = darken(r, g, b, 0.7);
            let (dr, dg, db) = brighten(r, g, b, 1.2);
            (
                format!("#{lr:02x}{lg:02x}{lb:02x}"),
                format!("#{dr:02x}{dg:02x}{db:02x}"),
            )
        };

        writeln!(
            css,
            "t-f{i} {{ color: light-dark({light_hex}, {dark_hex}); }}"
        )
        .unwrap();
        writeln!(
            css,
            "t-b{i} {{ background-color: light-dark({light_hex}, {dark_hex}); }}"
        )
        .unwrap();
    }
    writeln!(css).unwrap();

    // RGB via CSS custom property
    writeln!(css, "/* RGB colors via --c custom property */").unwrap();
    writeln!(css, "t-f {{ color: var(--c); }}").unwrap();
    writeln!(css, "t-b {{ background-color: var(--c); }}").unwrap();

    css
}

/// Convert ANSI 256-color index to RGB
fn ansi_256_to_rgb(code: u8) -> (u8, u8, u8) {
    match code {
        // Standard colors (0-15)
        0 => (0, 0, 0),        // Black
        1 => (128, 0, 0),      // Red
        2 => (0, 128, 0),      // Green
        3 => (128, 128, 0),    // Yellow
        4 => (0, 0, 128),      // Blue
        5 => (128, 0, 128),    // Magenta
        6 => (0, 128, 128),    // Cyan
        7 => (192, 192, 192),  // White
        8 => (128, 128, 128),  // Bright Black
        9 => (255, 0, 0),      // Bright Red
        10 => (0, 255, 0),     // Bright Green
        11 => (255, 255, 0),   // Bright Yellow
        12 => (0, 0, 255),     // Bright Blue
        13 => (255, 0, 255),   // Bright Magenta
        14 => (0, 255, 255),   // Bright Cyan
        15 => (255, 255, 255), // Bright White

        // 216 color cube (16-231)
        16..=231 => {
            let n = code - 16;
            let r = (n / 36) % 6;
            let g = (n / 6) % 6;
            let b = n % 6;
            let to_component = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
            (to_component(r), to_component(g), to_component(b))
        }

        // Grayscale (232-255)
        232..=255 => {
            let gray = 8 + (code - 232) * 10;
            (gray, gray, gray)
        }
    }
}

/// Darken a color by a factor (0.0-1.0)
fn darken(r: u8, g: u8, b: u8, factor: f32) -> (u8, u8, u8) {
    (
        (r as f32 * factor).min(255.0) as u8,
        (g as f32 * factor).min(255.0) as u8,
        (b as f32 * factor).min(255.0) as u8,
    )
}

/// Brighten a color by a factor (>1.0 to brighten)
fn brighten(r: u8, g: u8, b: u8, factor: f32) -> (u8, u8, u8) {
    (
        (r as f32 * factor).min(255.0) as u8,
        (g as f32 * factor).min(255.0) as u8,
        (b as f32 * factor).min(255.0) as u8,
    )
}
