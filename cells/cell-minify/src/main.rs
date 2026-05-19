//! Dodeca minify cell (cell-minify)
//!
//! This cell handles HTML minification.

use cell_minify_proto::{Minifier, MinifierDispatcher, MinifyResult};

/// Minifier implementation
#[derive(Clone)]
pub struct MinifierImpl;

impl Minifier for MinifierImpl {
    async fn minify_html(&self, html: String) -> MinifyResult {
        // TODO: Use facet-html for minification instead
        // For now, just return the input unchanged (no-op)
        MinifyResult::Success { content: html }
    }
}

dodeca_cell_runtime::declare_cell!("minify", |_host| MinifierDispatcher::new(MinifierImpl));
