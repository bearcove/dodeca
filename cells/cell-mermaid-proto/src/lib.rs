//! RPC protocol for dodeca Mermaid cell
//!
//! Defines services for rendering Mermaid diagrams to SVG.

/// Mermaid rendering service implemented by the cell.
///
/// The host calls these methods to render Mermaid diagrams.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait MermaidRenderer {
    /// Render a Mermaid diagram to SVG.
    ///
    /// Takes Mermaid diagram source code and returns rendered SVG.
    async fn render(&self, diagram: String) -> Result<String, String>;
}
