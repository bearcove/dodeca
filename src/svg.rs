//! Minification utilities
//!
//! Provides HTML minification via minify-html and SVG optimization via oxvg.

use minify_html::{Cfg, minify};
use oxvg_ast::{
    implementations::{roxmltree::parse, shared::Element},
    serialize::{Node, Options},
    visitor::Info,
};
use oxvg_optimiser::Jobs;
use typed_arena::Arena;

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

/// Optimize an SVG using oxvg then minify-html
///
/// Applies safe optimizations including:
/// - Path simplification and conversion (oxvg)
/// - Attribute cleanup and minification (oxvg)
/// - Color optimization (oxvg)
/// - Unused element removal (oxvg)
/// - Transform application (oxvg)
/// - Style consolidation (oxvg)
/// - Whitespace removal (minify-html)
///
/// Returns None if the SVG cannot be parsed or optimization fails
pub fn optimize_svg(svg_content: &str) -> Option<String> {
    let arena = Arena::new();
    let dom = parse(svg_content, &arena).ok()?;

    let jobs = Jobs::default();
    let info = Info::<Element>::new(&arena);
    jobs.run(&dom, &info).ok()?;

    let optimized = dom.serialize_with_options(Options::default()).ok()?;

    // Second pass: minify whitespace with minify-html
    let cfg = Cfg {
        keep_closing_tags: true,
        keep_spaces_between_attributes: true,
        ..Cfg::default()
    };
    let minified = minify(optimized.as_bytes(), &cfg);
    String::from_utf8(minified).ok()
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
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <circle cx="50" cy="50" r="40" fill="red"/>
        </svg>"#;

        let output = optimize_svg(input);
        assert!(output.is_some());
        let output = output.unwrap();
        // Output should still be valid SVG
        assert!(output.contains("<svg"));
        assert!(output.contains("circle"));
        // Should be smaller (whitespace removed at minimum)
        assert!(output.len() <= input.len());
    }

    #[test]
    fn test_optimize_svg_with_unnecessary_elements() {
        // SVG with editor metadata that should be removed
        let input = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <!-- This is a comment -->
            <defs></defs>
            <circle cx="50" cy="50" r="40" fill="#ff0000"/>
        </svg>"##;

        let output = optimize_svg(input);
        assert!(output.is_some());
        let output = output.unwrap();
        // Comments and empty defs should be removed
        assert!(!output.contains("<!--"));
    }

    #[test]
    fn test_invalid_svg_returns_none() {
        let input = "not an svg at all";
        assert!(optimize_svg(input).is_none());
    }
}
