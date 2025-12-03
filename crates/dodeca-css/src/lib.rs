//! CSS URL rewriting and minification plugin for dodeca
//!
//! Uses lightningcss to parse CSS, rewrite url() values, and minify output.

use facet::Facet;
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::visitor::Visit;
use plugcard::{plugcard, PlugResult};
use std::collections::HashMap;

plugcard::export_plugin!();

/// Input for CSS URL rewriting
#[derive(Facet)]
pub struct CssRewriteInput {
    /// The CSS source code
    pub css: String,
    /// Map of old URLs to new URLs
    pub path_map: HashMap<String, String>,
}

/// Rewrite URLs in CSS and minify
///
/// Parses CSS, rewrites url() values according to path_map, and returns minified CSS.
#[plugcard]
pub fn rewrite_urls_in_css(input: CssRewriteInput) -> PlugResult<String> {
    // Parse the CSS
    let mut stylesheet = match StyleSheet::parse(&input.css, ParserOptions::default()) {
        Ok(s) => s,
        Err(e) => {
            return PlugResult::Err(format!("Failed to parse CSS: {:?}", e));
        }
    };

    // Visit and rewrite URLs
    let mut visitor = UrlRewriter {
        path_map: &input.path_map,
    };
    if let Err(e) = stylesheet.visit(&mut visitor) {
        return PlugResult::Err(format!("Failed to visit CSS: {:?}", e));
    }

    // Serialize back to string with minification enabled
    let printer_options = PrinterOptions {
        minify: true,
        ..Default::default()
    };
    match stylesheet.to_css(printer_options) {
        Ok(result) => PlugResult::Ok(result.code),
        Err(e) => PlugResult::Err(format!("Failed to serialize CSS: {:?}", e)),
    }
}

/// Visitor that rewrites URLs in CSS
struct UrlRewriter<'a> {
    path_map: &'a HashMap<String, String>,
}

impl<'i, 'a> lightningcss::visitor::Visitor<'i> for UrlRewriter<'a> {
    type Error = std::convert::Infallible;

    fn visit_types(&self) -> lightningcss::visitor::VisitTypes {
        lightningcss::visit_types!(URLS)
    }

    fn visit_url(
        &mut self,
        url: &mut lightningcss::values::url::Url<'i>,
    ) -> Result<(), Self::Error> {
        let url_str = url.url.as_ref();
        if let Some(new_url) = self.path_map.get(url_str) {
            url.url = new_url.clone().into();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_css_urls() {
        let css = r#"
            @font-face {
                font-family: "Inter";
                src: url("/fonts/Inter.woff2") format("woff2");
            }
            body {
                background: url("/images/bg.png");
            }
        "#;

        let mut path_map = HashMap::new();
        path_map.insert(
            "/fonts/Inter.woff2".to_string(),
            "/fonts/Inter.abc123.woff2".to_string(),
        );
        path_map.insert(
            "/images/bg.png".to_string(),
            "/images/bg.def456.png".to_string(),
        );

        let input = CssRewriteInput {
            css: css.to_string(),
            path_map,
        };

        let result = rewrite_urls_in_css(input);
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got Err");
        };

        assert!(output.contains("/fonts/Inter.abc123.woff2"));
        assert!(output.contains("/images/bg.def456.png"));
        assert!(!output.contains("\"/fonts/Inter.woff2\""));
        assert!(!output.contains("\"/images/bg.png\""));
    }

    #[test]
    fn test_minification() {
        let css = r#"
            body {
                color: red;
                background: blue;
            }
        "#;

        let input = CssRewriteInput {
            css: css.to_string(),
            path_map: HashMap::new(),
        };

        let result = rewrite_urls_in_css(input);
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got Err");
        };

        // Should be minified (no unnecessary whitespace)
        assert!(output.len() < css.len());
        assert!(output.contains("color:red") || output.contains("color: red"));
    }
}
