//! # aasvg - ASCII Art to SVG
//!
//! Convert ASCII art diagrams to SVG with automatic light/dark mode support.
//!
//! This library renders ASCII diagrams using CSS variables, so the output
//! automatically adapts to the user's color scheme preference.
//!
//! ## Example
//!
//! ```rust
//! use aasvg::render;
//!
//! let diagram = r#"
//!     +-----+
//!     |     |
//!     +-----+
//! "#;
//!
//! let svg = render(diagram);
//! println!("{}", svg);
//! ```
//!
//! ## Supported Elements
//!
//! - **Lines**: `-`, `|`, `/`, `\`, `=`, `~`, `_`
//! - **Vertices**: `+`, `.`, `'`, `,`, `` ` ``
//! - **Arrows**: `>`, `<`, `^`, `v`, `V`
//! - **Points**: `o`, `*`, `●`, `○`, `◍`, `◌`, `⊕`
//! - **Jumps**: `(`, `)` for line crossings
//! - **Text**: Any other characters are rendered as text
//!
//! ## Light/Dark Mode
//!
//! The generated SVG includes CSS that uses `prefers-color-scheme` to
//! automatically switch colors based on the user's system preference.

mod chars;
mod decoration;
mod finder;
mod grid;
mod path;
mod svg;

pub use svg::RenderOptions;

use decoration::DecorationSet;
use finder::{find_decorations, find_paths};
use grid::Grid;
use path::PathSet;
use svg::generate_svg;

/// Render an ASCII art diagram to SVG.
///
/// The output SVG uses CSS variables for colors, so it automatically
/// adapts to light and dark color schemes.
///
/// # Example
///
/// ```rust
/// use aasvg::render;
///
/// let svg = render("+--+\n|  |\n+--+");
/// assert!(svg.contains("<svg"));
/// assert!(svg.contains("prefers-color-scheme"));
/// ```
pub fn render(input: &str) -> String {
    render_with_options(input, &RenderOptions::default())
}

/// Render an ASCII art diagram to SVG with custom options.
///
/// # Example
///
/// ```rust
/// use aasvg::{render_with_options, RenderOptions};
///
/// let options = RenderOptions::new()
///     .with_backdrop(true)
///     .with_spaces(0);
///
/// let svg = render_with_options("+--+\n|  |\n+--+", &options);
/// assert!(svg.contains("var(--aasvg-bg)"));
/// ```
pub fn render_with_options(input: &str, options: &RenderOptions) -> String {
    let mut grid = Grid::new(input);
    let mut paths = PathSet::new();
    let mut decorations = DecorationSet::new();

    find_paths(&mut grid, &mut paths);
    find_decorations(&mut grid, &paths, &mut decorations);

    generate_svg(&mut grid, &paths, &decorations, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_box() {
        let svg = render("+--+\n|  |\n+--+");
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("path"));
    }

    #[test]
    fn test_arrow() {
        let svg = render("-->");
        assert!(svg.contains("polygon"));
    }

    #[test]
    fn test_css_variables() {
        let svg = render("-");
        assert!(svg.contains("--aasvg-stroke"));
        assert!(svg.contains("--aasvg-fill"));
        assert!(svg.contains("--aasvg-bg"));
        assert!(svg.contains("--aasvg-text"));
    }

    #[test]
    fn test_dark_mode_support() {
        let svg = render("-");
        assert!(svg.contains("prefers-color-scheme: dark"));
    }

    #[test]
    fn test_with_backdrop() {
        let options = RenderOptions::new().with_backdrop(true);
        let svg = render_with_options("-", &options);
        assert!(svg.contains(r#"fill="var(--aasvg-bg)"#));
    }

    #[test]
    fn test_text_rendering() {
        let svg = render("Hello");
        assert!(svg.contains("<text"));
        assert!(svg.contains("Hello"));
    }

    #[test]
    fn test_disable_text() {
        let options = RenderOptions::new().with_disable_text(true);
        let svg = render_with_options("Hello", &options);
        assert!(!svg.contains("Hello"));
    }
}
