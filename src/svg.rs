//! Minification utilities using minify-html
//!
//! Provides HTML and SVG minification with safe defaults.

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

/// Get a minification config for SVG files
fn svg_cfg() -> Cfg {
    Cfg {
        // SVG-safe options
        keep_closing_tags: true, // SVG requires explicit closing tags
        keep_spaces_between_attributes: true, // Some SVG attributes need spacing
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

/// Minify SVG content
///
/// Returns minified SVG, or original content if minification fails
pub fn minify_svg(svg: &str) -> Option<String> {
    // Only process if it looks like valid SVG
    if !svg.contains("<svg") {
        return None;
    }

    let result = minify(svg.as_bytes(), &svg_cfg());
    String::from_utf8(result).ok()
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
    fn test_minify_svg() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <circle cx="50" cy="50" r="40" fill="red"/>
        </svg>"#;

        let output = minify_svg(input);
        assert!(output.is_some());
        let output = output.unwrap();
        // Output should still be valid SVG
        assert!(output.contains("<svg"));
        assert!(output.contains("circle"));
    }

    #[test]
    fn test_invalid_svg_returns_none() {
        let input = "not an svg at all";
        assert!(minify_svg(input).is_none());
    }
}
