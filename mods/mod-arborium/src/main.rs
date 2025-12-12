//! Dodeca syntax highlighting plugin using rapace
//!
//! This binary implements the SyntaxHighlightService protocol and provides
//! syntax highlighting functionality via arborium/tree-sitter.

use std::sync::Arc;

use color_eyre::Result;
use rapace_plugin::ServiceDispatch;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use mod_arborium_proto::SyntaxHighlightServiceServer;

mod syntax_highlight;

struct SyntaxHighlightServerWrapper(
    Arc<SyntaxHighlightServiceServer<syntax_highlight::SyntaxHighlightImpl>>,
);

impl ServiceDispatch for SyntaxHighlightServerWrapper {
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
        let bytes = payload.to_vec();
        Box::pin(async move { server.dispatch(method_id, &bytes).await })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let server = SyntaxHighlightServiceServer::new(syntax_highlight::SyntaxHighlightImpl);
    let wrapper = SyntaxHighlightServerWrapper(Arc::new(server));

    rapace_plugin::run(wrapper)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Plugin error: {}", e))
}
