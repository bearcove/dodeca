//! SVG optimization plugin for dodeca
//!
//! Removes unnecessary metadata, collapses groups, optimizes paths, etc.
//! Preserves case sensitivity of SVG attributes.

use plugcard::{plugcard, PlugResult};

plugcard::export_plugin!();

/// Optimize SVG content
///
/// Returns optimized SVG, or an error if optimization fails.
#[plugcard]
pub fn optimize_svg(svg: String) -> PlugResult<String> {
    match svag::minify(&svg) {
        Ok(optimized) => PlugResult::Ok(optimized),
        Err(e) => PlugResult::Err(format!("SVG optimization failed: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimize_svg() {
        let input = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <!-- A red circle -->
            <circle cx="50" cy="50" r="40" fill="#ff0000"/>
        </svg>"##;

        let result = optimize_svg(input.to_string());
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got Err");
        };
        // Should be smaller (removes comments, optimizes colors)
        assert!(output.len() < input.len(), "expected smaller output");
        // Should preserve viewBox (case-sensitive)
        assert!(output.contains("viewBox"), "viewBox should be preserved");
        // Should still have the circle
        assert!(output.contains("circle"), "circle element should be preserved");
    }

    #[test]
    fn test_optimize_svg_with_groups() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <g>
                <g>
                    <rect x="0" y="0" width="10" height="10"/>
                </g>
            </g>
        </svg>"#;

        let result = optimize_svg(input.to_string());
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got Err");
        };
        assert!(output.contains("rect"), "rect element should be preserved");
    }
}
