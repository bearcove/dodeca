//! Plugin server for rapace RPC communication
//!
//! This module handles:
//! - Creating a Unix socket for the plugin to connect to
//! - Spawning the plugin process
//! - Serving ContentService RPCs from the plugin

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use color_eyre::Result;
use rapace::StreamTransport;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command;

use dodeca_serve_protocol::ContentServiceServer;

use crate::content_service::HostContentService;
use crate::serve::SiteServer;

/// Type alias for our transport
type HostTransport = StreamTransport<ReadHalf<UnixStream>, WriteHalf<UnixStream>>;

/// Start the plugin server
///
/// This:
/// 1. Creates a Unix socket
/// 2. Spawns the plugin process with --host-socket and --bind args
/// 3. Accepts the plugin connection
/// 4. Serves ContentService RPCs
pub async fn start_plugin_server(
    server: Arc<SiteServer>,
    plugin_path: PathBuf,
    bind_addr: std::net::SocketAddr,
) -> Result<()> {
    // Create temp socket path
    let socket_path = format!("/tmp/dodeca-{}.sock", std::process::id());

    // Clean up any stale socket
    let _ = std::fs::remove_file(&socket_path);

    // Bind the socket
    let listener = UnixListener::bind(&socket_path)?;
    tracing::info!("RPC socket: {}", socket_path);

    // Spawn the plugin process
    let mut child = Command::new(&plugin_path)
        .arg(format!("--host-socket={}", socket_path))
        .arg(format!("--bind={}", bind_addr))
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    tracing::info!("Spawned plugin: {}", plugin_path.display());

    // Accept the plugin connection
    let (stream, _) = listener.accept().await?;
    tracing::info!("Plugin connected");

    // Create the rapace transport
    let transport: HostTransport = StreamTransport::new(stream);
    let transport = Arc::new(transport);

    // Create the ContentService implementation
    let content_service = HostContentService::new(server);

    // Create the server
    let rpc_server = ContentServiceServer::new(content_service);

    // Spawn RPC serving in background
    let transport_clone = transport.clone();
    tokio::spawn(async move {
        if let Err(e) = rpc_server.serve(transport_clone).await {
            tracing::error!("RPC server error: {:?}", e);
        }
    });

    // Wait for the child process
    let status = child.wait().await?;
    tracing::info!("Plugin exited with status: {}", status);

    // Cleanup
    let _ = std::fs::remove_file(&socket_path);

    Ok(())
}
