//! Dodeca minify cell (cell-minify)
//!
//! This cell handles HTML minification.

use cell_minify_proto::{Minifier, MinifierServer, MinifyResult};

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

rapace_cell::cell_service!(MinifierServer<MinifierImpl>, MinifierImpl);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run(CellService::from(MinifierImpl)).await?;
    Ok(())
}
