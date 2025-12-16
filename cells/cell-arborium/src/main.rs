//! Dodeca syntax highlighting cell using rapace
//!
//! This binary implements the SyntaxHighlightService protocol and provides
//! syntax highlighting functionality via arborium/tree-sitter.

use cell_arborium_proto::SyntaxHighlightServiceServer;

mod syntax_highlight;

use std::sync::Arc;

struct CellService(Arc<SyntaxHighlightServiceServer<syntax_highlight::SyntaxHighlightImpl>>);

impl rapace_cell::ServiceDispatch for CellService {
    fn dispatch(
        &self,
        method_id: u32,
        payload: &[u8],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<rapace::Frame, rapace::RpcError>>
                + Send
                + 'static,
        >,
    > {
        let server = self.0.clone();
        let payload = payload.to_vec();
        Box::pin(async move { server.dispatch(method_id, &payload).await })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = SyntaxHighlightServiceServer::new(syntax_highlight::SyntaxHighlightImpl);
    rapace_cell::run(CellService(Arc::new(server))).await?;
    Ok(())
}
