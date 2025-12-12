//! Plugin server for rapace RPC communication
//!
//! This module handles:
//! - Creating a shared memory segment for zero-copy RPC
//! - Spawning the plugin process
//! - Serving ContentService RPCs from the plugin
//! - Handling TCP connections from browsers via TcpTunnel
//! - Forwarding tracing events from plugin to host

use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use color_eyre::Result;
use futures::stream::{self, StreamExt};
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace::{Frame, RpcError, RpcSession};
use rapace_tracing::{
    EventMeta, Field, SpanMeta, TracingConfigClient, TracingSink, TracingSinkServer,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::watch;
use tokio_stream::wrappers::TcpListenerStream;

use mod_http_proto::{ContentServiceServer, TcpTunnelClient};

use crate::content_service::HostContentService;
use crate::serve::SiteServer;

/// Type alias for our transport (now SHM-based for zero-copy)
type HostTransport = ShmTransport;

/// SHM configuration for plugin communication
/// Using larger slots (64KB) and more of them (128) for content serving
/// Total: 8MB shared memory segment
const SHM_CONFIG: ShmSessionConfig = ShmSessionConfig {
    ring_capacity: 256, // 256 descriptors in flight
    slot_size: 65536,   // 64KB per slot (fits most HTML pages)
    slot_count: 128,    // 128 slots = 8MB total
};

/// Buffer size for TCP reads
const CHUNK_SIZE: usize = 4096;

// ============================================================================
// Forwarding TracingSink - re-emits plugin tracing events to host's tracing
// ============================================================================

use std::sync::atomic::{AtomicU64, Ordering};

/// A TracingSink implementation that forwards events to the host's tracing subscriber.
///
/// Events from the plugin are re-emitted as regular tracing events on the host,
/// making plugin logs appear in the host's output with their original target.
#[derive(Clone)]
pub struct ForwardingTracingSink {
    next_span_id: Arc<AtomicU64>,
}

impl ForwardingTracingSink {
    pub fn new() -> Self {
        Self {
            next_span_id: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl Default for ForwardingTracingSink {
    fn default() -> Self {
        Self::new()
    }
}

/// Format fields for display
fn format_fields(fields: &[Field]) -> String {
    fields
        .iter()
        .filter(|f| f.name != "message")
        .map(|f| format!("{}={}", f.name, f.value))
        .collect::<Vec<_>>()
        .join(" ")
}

impl TracingSink for ForwardingTracingSink {
    async fn new_span(&self, _span: SpanMeta) -> u64 {
        // Just assign an ID - we don't reconstruct spans on host
        self.next_span_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn record(&self, _span_id: u64, _fields: Vec<Field>) {
        // No-op: we don't track spans on host side
    }

    async fn event(&self, event: EventMeta) {
        // Re-emit the event using host's tracing
        // Note: tracing macros require static targets, so we include the plugin target in the message
        let fields = format_fields(&event.fields);
        let msg = if fields.is_empty() {
            event.message.clone()
        } else {
            format!("{} {}", event.message, fields)
        };

        // Include the plugin's target in the log message
        // Use a static target for the host side
        match event.level.as_str() {
            "ERROR" => tracing::error!(target: "plugin", "[{}] {}", event.target, msg),
            "WARN" => tracing::warn!(target: "plugin", "[{}] {}", event.target, msg),
            "INFO" => tracing::info!(target: "plugin", "[{}] {}", event.target, msg),
            "DEBUG" => tracing::debug!(target: "plugin", "[{}] {}", event.target, msg),
            "TRACE" => tracing::trace!(target: "plugin", "[{}] {}", event.target, msg),
            _ => tracing::info!(target: "plugin", "[{}] {}", event.target, msg),
        }
    }

    async fn enter(&self, _span_id: u64) {
        // No-op: we don't track span enter/exit on host
    }

    async fn exit(&self, _span_id: u64) {
        // No-op
    }

    async fn drop_span(&self, _span_id: u64) {
        // No-op
    }
}

/// Create a combined dispatcher that handles both ContentService and TracingSink.
///
/// Method IDs are now globally unique hashes, so we try each dispatcher in turn.
/// The correct one will succeed, the others will return "unknown method_id".
#[allow(clippy::type_complexity)]
pub fn create_combined_dispatcher(
    content_service: Arc<HostContentService>,
    tracing_sink: ForwardingTracingSink,
) -> impl Fn(
    u32,
    u32,
    Vec<u8>,
) -> Pin<Box<dyn std::future::Future<Output = Result<Frame, RpcError>> + Send>>
+ Send
+ Sync
+ 'static {
    move |_channel_id, method_id, payload| {
        let content_service = content_service.clone();
        let tracing_sink = tracing_sink.clone();
        Box::pin(async move {
            // Try TracingSink first
            let tracing_server = TracingSinkServer::new(tracing_sink);
            let result = tracing_server.dispatch(method_id, &payload).await;

            // If not "unknown method_id", return the result
            if !matches!(
                &result,
                Err(RpcError::Status {
                    code: rapace::ErrorCode::Unimplemented,
                    ..
                })
            ) {
                return result;
            }

            // Otherwise try ContentService
            let content_server = ContentServiceServer::new((*content_service).clone());
            content_server.dispatch(method_id, &payload).await
        })
    }
}

/// Find the plugin binary path (next to the main executable)
pub fn find_plugin_path() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    let plugin_path = exe_path
        .parent()
        .ok_or_else(|| color_eyre::eyre::eyre!("Cannot find parent directory of executable"))?
        .join("dodeca-mod-http");

    if !plugin_path.exists() {
        return Err(color_eyre::eyre::eyre!(
            "Plugin binary not found at {}. Build it with: cargo build -p dodeca-mod-http",
            plugin_path.display()
        ));
    }

    Ok(plugin_path)
}

/// Start the plugin server with optional shutdown signal
///
/// This:
/// 1. Creates a shared memory segment
/// 2. Spawns the plugin process with --shm-path arg
/// 3. Serves ContentService RPCs via SHM transport (zero-copy)
/// 4. Listens for browser TCP connections and tunnels them to the plugin
///
/// If `shutdown_rx` is provided, the server will stop when the signal is received.
///
/// The `bind_ips` parameter specifies which IP addresses to bind to. This allows
/// binding to specific LAN interfaces without exposing the server on WAN interfaces.
pub async fn start_plugin_server_with_shutdown(
    server: Arc<SiteServer>,
    plugin_path: PathBuf,
    bind_ips: Vec<std::net::Ipv4Addr>,
    port: u16,
    mut shutdown_rx: Option<watch::Receiver<bool>>,
    port_tx: Option<tokio::sync::oneshot::Sender<u16>>,
    pre_bound_listener: Option<TcpListener>,
) -> Result<()> {
    // Create SHM file path
    let shm_path = format!("/tmp/dodeca-{}.shm", std::process::id());

    // Clean up any stale SHM file
    let _ = std::fs::remove_file(&shm_path);

    // Create the SHM session (host side)
    let session = ShmSession::create_file(&shm_path, SHM_CONFIG)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to create SHM: {:?}", e))?;
    tracing::info!(
        "SHM segment: {} ({}KB)",
        shm_path,
        SHM_CONFIG.slot_size * SHM_CONFIG.slot_count / 1024
    );

    // Spawn the plugin process
    let mut child = Command::new(&plugin_path)
        .arg(format!("--shm-path={}", shm_path))
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    tracing::info!("Spawned plugin: {}", plugin_path.display());

    // Give the plugin time to map the SHM
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Create the SHM transport and wrap in RpcSession
    let transport: Arc<HostTransport> = Arc::new(ShmTransport::new(session));

    // Host uses odd channel IDs (1, 3, 5, ...)
    // Plugin uses even channel IDs (2, 4, 6, ...)
    let rpc_session = Arc::new(RpcSession::with_channel_start(transport, 1));
    tracing::info!("Plugin connected via SHM");

    // Create the ContentService and TracingSink implementations
    let content_service = Arc::new(HostContentService::new(server));
    let tracing_sink = ForwardingTracingSink::new();

    // Set up combined dispatcher for both services
    rpc_session.set_dispatcher(create_combined_dispatcher(content_service, tracing_sink));

    // Spawn the RPC session demux loop
    let session_runner = rpc_session.clone();
    tokio::spawn(async move {
        if let Err(e) = session_runner.run().await {
            tracing::error!("RPC session error: {:?}", e);
        }
    });

    // Push the current RUST_LOG filter to the plugin
    // The host is the single source of truth for log filtering
    let filter_str = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let tracing_config_client = TracingConfigClient::new(rpc_session.clone());
    if let Err(e) = tracing_config_client.set_filter(filter_str.clone()).await {
        tracing::warn!("Failed to push filter to plugin: {:?}", e);
    } else {
        tracing::debug!("Pushed filter to plugin: {}", filter_str);
    }

    // Start TCP listeners for browser connections
    let (listeners, bound_port) = if let Some(listener) = pre_bound_listener {
        // Use the pre-bound listener from FD passing (for testing)
        let bound_port = listener
            .local_addr()
            .map_err(|e| {
                color_eyre::eyre::eyre!("Failed to get pre-bound listener address: {}", e)
            })?
            .port();
        tracing::info!("Using pre-bound listener on port {}", bound_port);

        // Print READY signal for test harness
        println!("READY");
        use std::io::Write;
        let _ = std::io::stdout().flush();

        (vec![listener], bound_port)
    } else {
        // Bind to requested IPs normally - one listener per IP
        // This ensures we only bind to the specific interfaces requested
        let mut listeners = Vec::new();
        let mut actual_port: Option<u16> = None;
        for ip in &bind_ips {
            let addr = std::net::SocketAddr::new(std::net::IpAddr::V4(*ip), port);
            match TcpListener::bind(addr).await {
                Ok(listener) => {
                    // Get the actual bound port (important when port=0)
                    if let Ok(bound_addr) = listener.local_addr() {
                        let bound_port = bound_addr.port();
                        if actual_port.is_none() {
                            actual_port = Some(bound_port);
                        }
                        tracing::info!("Listening on {}:{}", ip, bound_port);
                    }
                    listeners.push(listener);
                }
                Err(e) => {
                    tracing::warn!("Failed to bind to {}: {}", addr, e);
                }
            }
        }

        if listeners.is_empty() {
            return Err(color_eyre::eyre::eyre!("Failed to bind to any addresses"));
        }

        let bound_port =
            actual_port.ok_or_else(|| color_eyre::eyre::eyre!("Could not determine bound port"))?;

        // Print the actual bound port for the test harness to discover
        eprintln!("DEBUG: About to print LISTENING_PORT={}", bound_port);
        println!("LISTENING_PORT={}", bound_port);
        // Flush stdout to ensure test harness sees it immediately
        use std::io::Write;
        let _ = std::io::stdout().flush();
        eprintln!("DEBUG: Flushed LISTENING_PORT");

        (listeners, bound_port)
    };

    // Send the bound port back to the caller (if channel provided)
    if let Some(tx) = port_tx {
        let _ = tx.send(bound_port);
    }

    // Merge all listeners into a single stream - when we drop it, ports are released
    let mut accept_stream = stream::select_all(listeners.into_iter().map(TcpListenerStream::new));

    // Accept browser connections and tunnel them to the plugin
    loop {
        tokio::select! {
            accept_result = accept_stream.next() => {
                match accept_result {
                    Some(Ok(stream)) => {
                        let addr = stream.peer_addr().ok();
                        tracing::debug!("Accepted browser connection from {:?}", addr);
                        let session = rpc_session.clone();
                        tokio::spawn(async move {
                            // Create a new TcpTunnelClient for this connection
                            let tunnel_client = TcpTunnelClient::new(session.clone());
                            if let Err(e) = handle_browser_connection(stream, tunnel_client, session).await {
                                tracing::error!("Failed to handle browser connection: {:?}", e);
                            }
                        });
                    }
                    Some(Err(e)) => {
                        tracing::error!("Accept error: {:?}", e);
                    }
                    None => {
                        // All listeners closed
                        tracing::info!("All listeners closed");
                        break;
                    }
                }
            }
            status = child.wait() => {
                match status {
                    Ok(s) => tracing::info!("Plugin exited with status: {}", s),
                    Err(e) => tracing::error!("Plugin wait error: {:?}", e),
                }
                break;
            }
            _ = async {
                if let Some(ref mut rx) = shutdown_rx {
                    rx.changed().await.ok();
                    if *rx.borrow() {
                        return;
                    }
                }
                // Never complete if no shutdown receiver
                std::future::pending::<()>().await
            } => {
                tracing::info!("Shutdown signal received, stopping plugin server");
                // Kill the plugin process
                let _ = child.kill().await;
                break;
            }
        }
    }

    // Cleanup: drop the stream to release listeners, then remove SHM file
    drop(accept_stream);
    let _ = std::fs::remove_file(&shm_path);

    Ok(())
}

/// Start the plugin server (convenience wrapper without shutdown signal)
///
/// Note: This function never returns as it runs the server loop indefinitely.
/// The actual bound port is printed via println!("LISTENING_PORT={}").
pub async fn start_plugin_server(
    server: Arc<SiteServer>,
    plugin_path: PathBuf,
    bind_ips: Vec<std::net::Ipv4Addr>,
    port: u16,
) -> Result<()> {
    start_plugin_server_with_shutdown(server, plugin_path, bind_ips, port, None, None, None).await
}

/// Handle a browser TCP connection by tunneling it through the plugin
async fn handle_browser_connection(
    browser_stream: TcpStream,
    tunnel_client: TcpTunnelClient<HostTransport>,
    session: Arc<RpcSession<HostTransport>>,
) -> Result<()> {
    // Open a tunnel to the plugin
    let handle = tunnel_client
        .open()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to open tunnel: {:?}", e))?;

    let channel_id = handle.channel_id;
    tracing::debug!(channel_id, "Tunnel opened for browser connection");

    // Register the tunnel to receive incoming chunks from plugin
    let mut tunnel_rx = session.register_tunnel(channel_id);

    let (mut browser_read, mut browser_write) = browser_stream.into_split();

    // Task A: Browser → rapace (read from browser, send to tunnel)
    let session_a = session.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; CHUNK_SIZE];
        loop {
            match browser_read.read(&mut buf).await {
                Ok(0) => {
                    // Browser closed connection
                    tracing::debug!(channel_id, "Browser closed connection");
                    let _ = session_a.close_tunnel(channel_id).await;
                    break;
                }
                Ok(n) => {
                    if let Err(e) = session_a.send_chunk(channel_id, buf[..n].to_vec()).await {
                        tracing::debug!(channel_id, error = %e, "Tunnel send error");
                        break;
                    }
                }
                Err(e) => {
                    tracing::debug!(channel_id, error = %e, "Browser read error");
                    let _ = session_a.close_tunnel(channel_id).await;
                    break;
                }
            }
        }
        tracing::debug!(channel_id, "Browser→rapace task finished");
    });

    // Task B: rapace → Browser (read from tunnel, write to browser)
    tokio::spawn(async move {
        while let Some(chunk) = tunnel_rx.recv().await {
            if !chunk.payload.is_empty()
                && let Err(e) = browser_write.write_all(&chunk.payload).await
            {
                tracing::debug!(channel_id, error = %e, "Browser write error");
                break;
            }
            if chunk.is_eos {
                tracing::debug!(channel_id, "Received EOS from plugin");
                // Half-close the browser write side
                let _ = browser_write.shutdown().await;
                break;
            }
        }
        tracing::debug!(channel_id, "rapace→browser task finished");
    });

    Ok(())
}
