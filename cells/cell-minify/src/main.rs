//! Dodeca minify cell (cell-minify)
//!
//! This cell handles HTML minification.

use cell_minify_proto::{Minifier, MinifierDispatcher, MinifyResult};
use dodeca_cell_runtime::run_cell;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("minify", |_handle| MinifierDispatcher::new(MinifierImpl))
}
