//! Minification utilities
//!
//! Provides HTML minification via minify-html.

use minify_html::{Cfg, minify};

/// Get a minification config optimized for production HTML
fn html_cfg() -> Cfg {
    Cfg {
        minify_css: true,
        minify_js: true,
        // Preserve template syntax for compatibility
        preserve_brace_template_syntax: true,
        ..Cfg::default()
    }
}

/// Minify HTML content
///
/// Returns minified HTML, or original content if minification fails
pub fn minify_html(html: &str) -> String {
    let result = minify(html.as_bytes(), &html_cfg());
    String::from_utf8(result).unwrap_or_else(|_| html.to_string())
}

/// Optimize SVG content using svag
///
/// Removes unnecessary metadata, collapses groups, optimizes paths, etc.
/// Preserves case sensitivity of SVG attributes (unlike minify-html).
pub fn optimize_svg(svg_content: &str) -> Option<String> {
    svag::minify(svg_content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minify_html() {
        let input = r#"<!DOCTYPE html>
<html>
  <head>
    <title>Test</title>
  </head>
  <body>
    <p>Hello World</p>
  </body>
</html>"#;

        let output = minify_html(input);
        assert!(output.len() < input.len());
        // Note: minify-html removes optional closing tags like </p>
        assert!(output.contains("<p>Hello World"));
        assert!(output.contains("<title>Test"));
    }

    #[test]
    fn test_optimize_svg() {
        let input = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <!-- A red circle -->
            <circle cx="50" cy="50" r="40" fill="#ff0000"/>
        </svg>"##;

        let output = optimize_svg(input);
        assert!(output.is_some());
        let output = output.unwrap();
        // Should be smaller (removes comments, optimizes colors)
        assert!(output.len() < input.len(), "expected smaller output");
        // Should preserve viewBox (case-sensitive)
        assert!(output.contains("viewBox"), "viewBox should be preserved");
        // Should still have the circle
        assert!(output.contains("circle"), "circle element should be preserved");
    }
}
