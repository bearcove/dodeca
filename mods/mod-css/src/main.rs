//! Dodeca CSS plugin (dodeca-mod-css)
//!
//! This plugin handles CSS URL rewriting and minification via lightningcss.

use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::visitor::Visit;

use mod_css_proto::{CssProcessor, CssProcessorServer};

/// CSS processor implementation
pub struct CssProcessorImpl;

impl CssProcessor for CssProcessorImpl {
    async fn rewrite_and_minify(&self, css: String, path_map: std::collections::HashMap<String, String>) -> mod_css_proto::CssResult {
        // Parse the CSS
        let mut stylesheet = match StyleSheet::parse(&css, ParserOptions::default()) {
            Ok(s) => s,
            Err(e) => {
                return mod_css_proto::CssResult::Error {
                    message: format!("Failed to parse CSS: {:?}", e),
                };
            }
        };

        // Visit and rewrite URLs
        let mut visitor = UrlRewriter {
            path_map: &path_map,
        };
        if let Err(e) = stylesheet.visit(&mut visitor) {
            return mod_css_proto::CssResult::Error {
                message: format!("Failed to visit CSS: {:?}", e),
            };
        }

        // Serialize back to string with minification enabled
        let printer_options = PrinterOptions {
            minify: true,
            ..Default::default()
        };
        match stylesheet.to_css(printer_options) {
            Ok(result) => mod_css_proto::CssResult::Success { css: result.code },
            Err(e) => mod_css_proto::CssResult::Error {
                message: format!("Failed to serialize CSS: {:?}", e),
            },
        }
    }
}

/// Visitor that rewrites URLs in CSS
struct UrlRewriter<'a> {
    path_map: &'a std::collections::HashMap<String, String>,
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

dodeca_plugin_runtime::plugin_service!(
    CssProcessorServer<CssProcessorImpl>,
    CssProcessorImpl
);

dodeca_plugin_runtime::run_plugin!(CssProcessorImpl);
