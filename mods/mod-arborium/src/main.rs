//! Dodeca syntax highlighting plugin using rapace
//!
//! This binary implements the SyntaxHighlightService protocol and provides
//! syntax highlighting functionality via arborium/tree-sitter.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use color_eyre::Result;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace::{Frame, RpcError, RpcSession};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use mod_arborium_proto::SyntaxHighlightServiceServer;

mod syntax_highlight;

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// SHM configuration - must match host's config
const SHM_CONFIG: ShmSessionConfig = ShmSessionConfig {
    ring_capacity: 256, // 256 descriptors in flight
    slot_size: 65536,   // 64KB per slot (fits most HTML pages)
    slot_count: 128,    // 128 slots = 8MB total
};

/// CLI arguments
struct Args {
    /// SHM file path for zero-copy communication with host
    shm_path: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // Parse CLI arguments
    let args = parse_args()?;

    // Setup tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Wait for host to create SHM file
    while !args.shm_path.exists() {
        tracing::debug!("Waiting for SHM file: {}", args.shm_path.display());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Open the SHM session (plugin side)
    let shm_session = ShmSession::open_file(&args.shm_path, SHM_CONFIG)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to open SHM: {:?}", e))?;

    // Create SHM transport
    let transport: Arc<PluginTransport> = Arc::new(ShmTransport::new(shm_session));

    // Plugin uses even channel IDs (2, 4, 6, ...)
    // Host uses odd channel IDs (1, 3, 5, ...)
    let session = Arc::new(RpcSession::with_channel_start(transport, 2));

    tracing::info!("Connected to host via SHM");

    // Create combined dispatcher and set it on the session
    let dispatcher = create_dispatcher(syntax_highlight::SyntaxHighlightImpl);
    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop
    session.run().await?;

    Ok(())
}

/// Parse command line arguments
fn parse_args() -> Result<Args> {
    let mut args = std::env::args();
    args.next(); // Skip program name

    let shm_path = match args.next() {
        Some(path) => PathBuf::from(path),
        None => {
            return Err(color_eyre::eyre::eyre!(
                "Usage: dodeca-syntax-highlight-rapace <shm_path>"
            ));
        }
    };

    Ok(Args { shm_path })
}

/// Create a combined dispatcher for the syntax highlight service.
#[allow(clippy::type_complexity)]
fn create_dispatcher(
    syntax_highlight_impl: syntax_highlight::SyntaxHighlightImpl,
) -> impl Fn(
    u32,
    u32,
    Vec<u8>,
) -> Pin<Box<dyn std::future::Future<Output = Result<Frame, RpcError>> + Send>>
+ Send
+ Sync
+ 'static {
    move |_channel_id, method_id, payload| {
        let syntax_highlight_impl = syntax_highlight_impl.clone();
        Box::pin(async move {
            let server = SyntaxHighlightServiceServer::new(syntax_highlight_impl);
            server.dispatch(method_id, &payload).await
        })
    }
}
