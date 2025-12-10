//! Dodeca syntax highlighting plugin using rapace
//!
//! This binary implements the SyntaxHighlightService protocol and provides
//! syntax highlighting functionality via arborium/tree-sitter.

use std::path::PathBuf;
use std::pin::Pin;

use color_eyre::Result;
use rapace::{Frame, RpcError};
use rapace_testkit::RpcSession;
use rapace_transport_shm::{ShmSession, ShmSessionConfig, ShmTransport};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use dodeca_syntax_highlight_protocol::{HighlightResult, SyntaxHighlightService};

mod syntax_highlight;

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// Plugin context shared across handlers
pub struct PluginContext {
    /// RPC session for bidirectional communication with host
    pub session: Arc<RpcSession<PluginTransport>>,
}

impl PluginContext {
    /// Create a SyntaxHighlightService server for handling RPC calls
    pub fn syntax_highlight_server(&self) -> SyntaxHighlightServer<PluginTransport> {
        SyntaxHighlightServer::new(syntax_highlight::SyntaxHighlightImpl)
    }
}

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

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // Parse CLI arguments
    let args = parse_args()?;

    // Setup tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Create SHM session
    let session = ShmSession::new(&args.shm_path, SHM_CONFIG).await?;

    // Create RPC session
    let session = RpcSession::new(session);

    // Create plugin context
    let context = PluginContext {
        session: Arc::new(session),
    };

    // Create syntax highlight server
    let syntax_highlight_server = context.syntax_highlight_server();

    // Create combined dispatcher
    let dispatcher = create_dispatcher(syntax_highlight_server);

    // Run the RPC server
    context.session.run(dispatcher).await?;

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
fn create_dispatcher(
    syntax_highlight_server: SyntaxHighlightServer<PluginTransport>,
) -> impl Fn(
    u32,
    u32,
    Vec<u8>,
) -> Pin<Box<dyn std::future::Future<Output = Result<Frame, RpcError>> + Send>>
+ Send
+ Sync
+ 'static {
    move |_channel_id, method_id, payload| {
        let syntax_highlight_server = syntax_highlight_server.clone();
        Box::pin(async move { syntax_highlight_server.dispatch(method_id, &payload).await })
    }
}
