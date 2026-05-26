//! RPC protocol for the Dodeca authoring LSP cell.

use facet::Facet;

/// Startup arguments for the authoring language server.
#[derive(Debug, Clone, Facet)]
pub struct AuthoringLspStartupArgs {
    pub content: Option<String>,
    pub output: Option<String>,
}

/// Result of running the authoring language server.
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum AuthoringLspRunResult {
    Success,
    Error { message: String },
}

/// Authoring LSP service implemented by the cell.
#[allow(async_fn_in_trait)]
#[vox::service]
pub trait AuthoringLsp {
    /// Run the LSP server over process stdio.
    async fn run(&self, args: AuthoringLspStartupArgs) -> AuthoringLspRunResult;
}
