//! SVG generation with CSS variables for light/dark mode support.

use std::fmt::Write;

use crate::decoration::DecorationSet;
use crate::grid::{unhide_markers, Grid};
use crate::path::{PathSet, ASPECT, SCALE};

/// CSS style block for light/dark mode support
const CSS_VARIABLES: &str = r#"<style>
  :root {
    --aasvg-stroke: #000;
    --aasvg-fill: #000;
    --aasvg-bg: #fff;
    --aasvg-text: #000;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      --aasvg-stroke: #fff;
      --aasvg-fill: #fff;
      --aasvg-bg: #1a1a1a;
      --aasvg-text: #fff;
    }
  }
</style>
"#;

/// Render options
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// Add a background rectangle
    pub backdrop: bool,
    /// Skip text rendering
    pub disable_text: bool,
    /// Minimum consecutive spaces to end a text run (0 = each char separate)
    pub spaces: u32,
    /// Stretch text to fit character cells (uses textLength/lengthAdjust)
    pub stretch: bool,
}

impl RenderOptions {
    pub fn new() -> Self {
        Self {
            backdrop: false,
            disable_text: false,
            spaces: 2,
            stretch: false,
        }
    }

    pub fn with_backdrop(mut self, backdrop: bool) -> Self {
        self.backdrop = backdrop;
        self
    }

    pub fn with_disable_text(mut self, disable_text: bool) -> Self {
        self.disable_text = disable_text;
        self
    }

    pub fn with_spaces(mut self, spaces: u32) -> Self {
        self.spaces = spaces;
        self
    }

    pub fn with_stretch(mut self, stretch: bool) -> Self {
        self.stretch = stretch;
        self
    }
}

/// Generate complete SVG from paths, decorations, and remaining text
pub fn generate_svg(
    grid: &mut Grid,
    paths: &PathSet,
    decorations: &DecorationSet,
    options: &RenderOptions,
) -> String {
    let width = ((grid.width + 1) as f64 * SCALE) as u32;
    let height = ((grid.height + 1) as f64 * SCALE * ASPECT) as u32;

    let mut svg = String::new();

    // SVG header
    let _ = write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="{}" height="{}" viewBox="0 0 {} {}" class="diagram" text-anchor="middle" font-family="monospace" font-size="13px" stroke-linecap="round">
"#,
        width, height, width, height
    );

    // CSS variables for light/dark mode
    svg.push_str(CSS_VARIABLES);

    // Backdrop
    if options.backdrop {
        let _ = write!(
            svg,
            r#"<rect x="0" y="0" width="{}" height="{}" fill="var(--aasvg-bg)"/>
"#,
            width, height
        );
    }

    // Paths
    svg.push_str(&paths.to_svg());

    // Decorations
    svg.push_str(&decorations.to_svg());

    // Text
    if !options.disable_text {
        svg.push_str(&extract_text(grid, options.spaces, options.stretch));
    }

    // Close SVG
    svg.push_str("</svg>");

    svg
}

/// Extract remaining text from the grid and generate SVG text elements
fn extract_text(grid: &mut Grid, spaces: u32, stretch: bool) -> String {
    let mut result = String::new();
    result.push_str("<g fill=\"var(--aasvg-text)\">\n");

    for y in 0..grid.height as i32 {
        let mut x = 0;
        while x < grid.width as i32 {
            if let Some(start_x) = grid.text_start(x, y, spaces) {
                let text = grid.extract_text(start_x, y, spaces);
                if !text.is_empty() {
                    // Restore hidden markers (o, v, V that were part of text)
                    let text = unhide_markers(&text);
                    let char_count = text.chars().count();
                    let px = (start_x as f64 + 1.0 + (char_count as f64 - 1.0) / 2.0) * SCALE;
                    let py = (y as f64 + 1.0) * SCALE * ASPECT + 4.0;

                    let escaped = escape_xml(&text);

                    if stretch {
                        let text_length = char_count as f64 * SCALE;
                        let _ = write!(
                            result,
                            "<text x=\"{}\" y=\"{}\" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\">{}</text>\n",
                            px, py, text_length, escaped
                        );
                    } else {
                        let _ = write!(
                            result,
                            "<text x=\"{}\" y=\"{}\">{}</text>\n",
                            px, py, escaped
                        );
                    }
                }
                x = start_x + text.chars().count() as i32;
            } else {
                break;
            }
        }
    }

    result.push_str("</g>\n");
    result
}

/// Escape special XML characters
fn escape_xml(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#39;"),
            _ => result.push(c),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder::{find_decorations, find_paths};

    #[test]
    fn test_svg_generation() {
        let mut grid = Grid::new("+--+\n|  |\n+--+");
        let mut paths = PathSet::new();
        let mut decorations = DecorationSet::new();

        find_paths(&mut grid, &mut paths);
        find_decorations(&mut grid, &paths, &mut decorations);

        let options = RenderOptions::new();
        let svg = generate_svg(&mut grid, &paths, &decorations, &options);

        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("--aasvg-stroke"));
        assert!(svg.contains("prefers-color-scheme: dark"));
    }

    #[test]
    fn test_svg_with_backdrop() {
        let mut grid = Grid::new("--");
        let mut paths = PathSet::new();
        let decorations = DecorationSet::new();

        find_paths(&mut grid, &mut paths);

        let options = RenderOptions::new().with_backdrop(true);
        let svg = generate_svg(&mut grid, &paths, &decorations, &options);

        assert!(svg.contains(r#"fill="var(--aasvg-bg)"#));
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("a<b>c"), "a&lt;b&gt;c");
        assert_eq!(escape_xml("a&b"), "a&amp;b");
        assert_eq!(escape_xml("\"test\""), "&quot;test&quot;");
    }
}
