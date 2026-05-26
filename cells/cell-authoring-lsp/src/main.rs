//! Dodeca authoring LSP cell.

pub mod authoring_lsp;

use cell_authoring_lsp_proto::{
    AuthoringLsp, AuthoringLspDispatcher, AuthoringLspRunResult, AuthoringLspStartupArgs,
};

#[derive(Clone)]
pub struct AuthoringLspImpl;

impl AuthoringLsp for AuthoringLspImpl {
    async fn run(&self, args: AuthoringLspStartupArgs) -> AuthoringLspRunResult {
        match authoring_lsp::run(args.content, args.output).await {
            Ok(()) => AuthoringLspRunResult::Success,
            Err(error) => AuthoringLspRunResult::Error {
                message: error.to_string(),
            },
        }
    }
}

dodeca_cell_runtime::declare_cell!("authoring-lsp", |_host| {
    AuthoringLspDispatcher::new(AuthoringLspImpl)
});
