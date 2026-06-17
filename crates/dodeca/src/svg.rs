//! Minification utilities
//!
//! Provides HTML and SVG minification utilities.

use cell_svgo_proto::SvgoResult;

/// Minify HTML content
///
/// HTML minification is currently a no-op. Keep this boundary in place so the
/// render path can switch to a real in-process minifier without changing
/// callers.
pub async fn minify_html(html: &str) -> String {
    html.to_string()
}

/// Optimize SVG content
///
/// Removes unnecessary metadata, collapses groups, optimizes paths, etc.
/// Preserves case sensitivity of SVG attributes.
pub async fn optimize_svg(svg_content: &str) -> Option<String> {
    match crate::cells::optimize_svg(svg_content.to_string()).await {
        Ok(SvgoResult::Success { svg }) => Some(svg),
        Ok(SvgoResult::Error { message }) => {
            tracing::warn!("SVG optimization failed: {}", message);
            None
        }
        Err(e) => {
            tracing::warn!("SVG optimization failed: {}", e);
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
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
        assert_eq!(output, input);
    }

    #[tokio::test]
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
