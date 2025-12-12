//! Dodeca syntax highlighting plugin using rapace
//!
//! This binary implements the SyntaxHighlightService protocol and provides
//! syntax highlighting functionality via arborium/tree-sitter.

use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::Result;
use dodeca_plugin_runtime::{PluginTracing, add_tracing_service};
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};

use mod_arborium_proto::SyntaxHighlightServiceServer;

mod syntax_highlight;

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// SHM configuration - must match host's config
const SHM_CONFIG: ShmSessionConfig = ShmSessionConfig {
    ring_capacity: 256,
    slot_size: 65536,
    slot_count: 128,
};

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

    // Expect SHM path as first argument from host
    let shm_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| color_eyre::eyre::eyre!("SHM path argument required"))?;

    // Open the SHM session (plugin side)
    let shm_session = ShmSession::open_file(&shm_path, SHM_CONFIG)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to open SHM: {:?}", e))?;

    // Create SHM transport
    let transport: Arc<PluginTransport> = Arc::new(ShmTransport::new(shm_session));

    // Plugin uses even channel IDs (2, 4, 6, ...)
    // Host uses odd channel IDs (1, 3, 5, ...)
    let session = Arc::new(RpcSession::with_channel_start(transport, 2));

    // Initialize tracing to forward logs to host via RapaceTracingLayer
    // The host controls the filter level via TracingConfig RPC
    let PluginTracing { tracing_config, .. } = dodeca_plugin_runtime::init_tracing(session.clone());

    // Build SyntaxHighlight service
    let server = SyntaxHighlightServiceServer::new(syntax_highlight::SyntaxHighlightImpl);
    let wrapper = SyntaxHighlightServerWrapper(Arc::new(server));

    // Wrap services with rapace-plugin multi-service dispatcher
    let dispatcher = DispatcherBuilder::new();
    let dispatcher = add_tracing_service(dispatcher, tracing_config);
    let dispatcher = dispatcher.add_service(wrapper);
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
        return Err(color_eyre::eyre::eyre!("Plugin error: {:?}", e));
    }

    Ok(())
}
