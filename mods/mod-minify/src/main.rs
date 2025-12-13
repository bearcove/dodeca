//! Dodeca minify plugin (dodeca-mod-minify)
//!
//! This plugin handles HTML minification.

use mod_minify_proto::{Minifier, MinifyResult, MinifierServer};

/// Minifier implementation
pub struct MinifierImpl;

impl Minifier for MinifierImpl {
    async fn minify_html(&self, html: String) -> MinifyResult {
        let cfg = minify_html::Cfg {
            minify_css: true,
            minify_js: true,
            // Preserve template syntax for compatibility
            preserve_brace_template_syntax: true,
            ..minify_html::Cfg::default()
        };

        let result = minify_html::minify(html.as_bytes(), &cfg);
        match String::from_utf8(result) {
            Ok(minified) => MinifyResult::Success { content: minified },
            Err(_) => MinifyResult::Error {
                message: "minification produced invalid UTF-8".to_string(),
            },
        }
    }
}

dodeca_plugin_runtime::plugin_service!(
    MinifierServer<MinifierImpl>,
    MinifierImpl
);

dodeca_plugin_runtime::run_plugin!(MinifierImpl);
