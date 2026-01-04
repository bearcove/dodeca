//! Dodeca minify cell (cell-minify)
//!
//! This cell handles HTML minification.

use cell_minify_proto::{Minifier, MinifierServer, MinifyResult};

/// Minifier implementation
pub struct MinifierImpl;

impl Minifier for MinifierImpl {
    async fn minify_html(&self, html: String) -> MinifyResult {
        // TODO: Use facet-html for minification instead
        // For now, just return the input unchanged (no-op)
        MinifyResult::Success { content: html }
    }
}

rapace_cell::cell_service!(MinifierServer<MinifierImpl>, MinifierImpl);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run(CellService::from(MinifierImpl)).await?;
    Ok(())
}
