//! Dodeca syntax highlighting cell using rapace
//!
//! This binary implements the SyntaxHighlightService protocol and provides
//! syntax highlighting functionality via arborium/tree-sitter.

use cell_arborium_proto::SyntaxHighlightServiceServer;

mod syntax_highlight;

rapace_cell::cell_service!(
    SyntaxHighlightServiceServer<syntax_highlight::SyntaxHighlightImpl>,
    syntax_highlight::SyntaxHighlightImpl
);

#[expect(
    clippy::disallowed_methods,
    reason = "tokio::main uses block_on internally"
)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run(CellService::from(syntax_highlight::SyntaxHighlightImpl)).await?;
    Ok(())
}
