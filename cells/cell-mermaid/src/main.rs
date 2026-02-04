//! Dodeca Mermaid cell (cell-mermaid)
//!
//! This cell renders Mermaid diagrams to SVG using mermaid-rs-renderer.

use cell_mermaid_proto::{MermaidRenderer, MermaidRendererDispatcher};
use dodeca_cell_runtime::run_cell;

/// Mermaid renderer implementation
#[derive(Clone)]
pub struct MermaidRendererImpl;

impl MermaidRenderer for MermaidRendererImpl {
    async fn render(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        diagram: String,
    ) -> Result<String, String> {
        mermaid_rs_renderer::render(&diagram).map_err(|e| e.to_string())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("mermaid", |_handle| MermaidRendererDispatcher::new(
        MermaidRendererImpl
    ))
}
