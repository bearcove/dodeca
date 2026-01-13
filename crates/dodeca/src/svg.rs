//! Minification utilities
//!
//! Provides HTML and SVG minification via cells.

use crate::cells::{minify_html_cell, optimize_svg_cell};
use cell_minify_proto::MinifyResult;
use cell_svgo_proto::SvgoResult;

/// Minify HTML content
///
/// Returns minified HTML, or original content if minification fails
pub async fn minify_html(html: &str) -> String {
    match minify_html_cell(html.to_string()).await {
        Ok(MinifyResult::Success { content }) => content,
        Ok(MinifyResult::Error { message }) => {
            tracing::warn!("HTML minification failed: {}", message);
            html.to_string()
        }
        Err(e) => {
            tracing::warn!("HTML minification RPC failed: {}", e);
            html.to_string()
        }
    }
}

/// Optimize SVG content
///
/// Removes unnecessary metadata, collapses groups, optimizes paths, etc.
/// Preserves case sensitivity of SVG attributes.
pub async fn optimize_svg(svg_content: &str) -> Option<String> {
    match optimize_svg_cell(svg_content.to_string()).await {
        Ok(SvgoResult::Success { content }) => Some(content),
        Ok(SvgoResult::Error { message }) => {
            tracing::warn!("SVG optimization failed: {}", message);
            None
        }
        Err(e) => {
            tracing::warn!("SVG optimization RPC failed: {}", e);
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    // These tests require external cells to be loaded, which only happens
    // in integration tests. The async machinery works correctly - the cell
    // just isn't available in unit test context.

    #[tokio::test]
    #[ignore = "requires dodeca-minify cell"]
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
    #[ignore = "requires dodeca-svgo cell"]
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
