//! Minification utilities
//!
//! Provides HTML and SVG minification via plugins.

use crate::plugins::{minify_html_plugin, optimize_svg_plugin};

/// Minify HTML content
///
/// Returns minified HTML, or original content if minification fails
pub async fn minify_html(html: &str) -> String {
    minify_html_plugin(html).await.unwrap_or_else(|e| {
        tracing::warn!("HTML minification failed: {}", e);
        html.to_string()
    })
}

/// Optimize SVG content
///
/// Removes unnecessary metadata, collapses groups, optimizes paths, etc.
/// Preserves case sensitivity of SVG attributes.
pub async fn optimize_svg(svg_content: &str) -> Option<String> {
    optimize_svg_plugin(svg_content).await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests require external plugins to be loaded, which only happens
    // in integration tests. The async machinery works correctly - the plugin
    // just isn't available in unit test context.

    #[tokio::test]
    #[ignore = "requires dodeca-minify plugin"]
    async fn test_minify_html() {
        let input = r#"<!DOCTYPE html>
<html>
  <head>
    <title>Test</title>
  </head>
  <body>
    <p>Hello World</p>
  </body>
</html>"#;

        let output = minify_html(input).await;
        assert!(output.len() < input.len());
        // Note: minify-html removes optional closing tags like </p>
        assert!(output.contains("<p>Hello World"));
        assert!(output.contains("<title>Test"));
    }

    #[tokio::test]
    #[ignore = "requires dodeca-svgo plugin"]
    async fn test_optimize_svg() {
        let input = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <!-- A red circle -->
            <circle cx="50" cy="50" r="40" fill="#ff0000"/>
        </svg>"##;

        let output = optimize_svg(input).await;
        assert!(output.is_some());
        let output = output.unwrap();
        // Should be smaller (removes comments, optimizes colors)
        assert!(output.len() < input.len(), "expected smaller output");
        // Should preserve viewBox (case-sensitive)
        assert!(output.contains("viewBox"), "viewBox should be preserved");
        // Should still have the circle
        assert!(
            output.contains("circle"),
            "circle element should be preserved"
        );
    }
}
