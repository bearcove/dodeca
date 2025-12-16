//! HTTP cell server for rapace RPC communication
//!
//! This module handles:
//! - Setting up ContentService on the http cell's session (via hub)
//! - Handling TCP connections from browsers via TcpTunnel
//!
//! The http cell is loaded through the hub like all other cells.

use std::pin::Pin;
use std::sync::Arc;

use eyre::Result;
use futures::stream::{self, StreamExt};
use rapace::{Frame, RpcError, RpcSession};
use rapace_tracing::{EventMeta, Field, SpanMeta, TracingSink};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_stream::wrappers::TcpListenerStream;

use cell_http_proto::{ContentServiceServer, TcpTunnelClient};
use cell_lifecycle_proto::CellLifecycleServer;
use rapace_tracing::TracingSinkServer;

use crate::cells::{HostCellLifecycle, all, cell_ready_registry, get_cell_session};
use crate::content_service::HostContentService;
use crate::serve::SiteServer;

/// Find the cell binary path (for backwards compatibility).
///
/// Note: The http cell is now loaded via the hub, so this just returns a dummy path.
/// The actual cell location is determined by cells.rs.
pub fn find_cell_path() -> Result<std::path::PathBuf> {
    // Return a dummy path - cells are loaded via hub now
    Ok(std::path::PathBuf::from("ddc-cell-http"))
}

/// Buffer size for TCP reads
const CHUNK_SIZE: usize = 4096;

// ============================================================================
// Forwarding TracingSink - re-emits cell tracing events to host's tracing
// ============================================================================

use std::sync::atomic::{AtomicU64, Ordering};

/// A TracingSink implementation that forwards events to the host's tracing subscriber.
///
/// Events from the cell are re-emitted as regular tracing events on the host,
/// making cell logs appear in the host's output with their original target.
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
        // Note: tracing macros require static targets, so we include the cell target in the message
        let fields = format_fields(&event.fields);
        let msg = if fields.is_empty() {
            event.message.clone()
        } else {
            format!("{} {}", event.message, fields)
        };

        // Include the cell's target in the log message
        // Use a static target for the host side
        match event.level.as_str() {
            "ERROR" => tracing::error!(target: "cell", "[{}] {}", event.target, msg),
            "WARN" => tracing::warn!(target: "cell", "[{}] {}", event.target, msg),
            "INFO" => tracing::info!(target: "cell", "[{}] {}", event.target, msg),
            "DEBUG" => tracing::debug!(target: "cell", "[{}] {}", event.target, msg),
            "TRACE" => tracing::trace!(target: "cell", "[{}] {}", event.target, msg),
            _ => tracing::info!(target: "cell", "[{}] {}", event.target, msg),
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

/// Create a multi-service dispatcher that handles TracingSink, CellLifecycle, and ContentService.
///
/// Method IDs are globally unique hashes, so we try each service in turn.
/// The correct one will succeed, the others will return "unknown method_id".
#[allow(clippy::type_complexity)]
fn create_http_cell_dispatcher(
    content_service: Arc<HostContentService>,
) -> impl Fn(Frame) -> Pin<Box<dyn std::future::Future<Output = Result<Frame, RpcError>> + Send>>
+ Send
+ Sync
+ 'static {
    let tracing_sink = ForwardingTracingSink::new();
    let lifecycle_registry = cell_ready_registry().clone();

    move |frame: Frame| {
        let content_service = content_service.clone();
        let tracing_sink = tracing_sink.clone();
        let lifecycle_registry = lifecycle_registry.clone();
        let method_id = frame.desc.method_id;
        let payload = frame.payload_bytes().to_vec();

        Box::pin(async move {
            // Try TracingSink service first
            let tracing_server = TracingSinkServer::new(tracing_sink);
            if let Ok(frame) = tracing_server.dispatch(method_id, &payload).await {
                return Ok(frame);
            }

            // Try CellLifecycle service
            let lifecycle_impl = HostCellLifecycle::new(lifecycle_registry);
            let lifecycle_server = CellLifecycleServer::new(lifecycle_impl);
            if let Ok(frame) = lifecycle_server.dispatch(method_id, &payload).await {
                return Ok(frame);
            }

            // Try ContentService
            let content_server = ContentServiceServer::new((*content_service).clone());
            content_server.dispatch(method_id, &payload).await
        })
    }
}

/// Start the HTTP cell server with optional shutdown signal
///
/// This:
/// 1. Ensures the http cell is loaded (via all())
/// 2. Sets up ContentService on the http cell's session
/// 3. Listens for browser TCP connections and tunnels them to the cell
///
/// If `shutdown_rx` is provided, the server will stop when the signal is received.
///
/// The `bind_ips` parameter specifies which IP addresses to bind to.
pub async fn start_cell_server_with_shutdown(
    server: Arc<SiteServer>,
    _cell_path: std::path::PathBuf, // No longer used - cells loaded via hub
    bind_ips: Vec<std::net::Ipv4Addr>,
    port: u16,
    mut shutdown_rx: Option<watch::Receiver<bool>>,
    port_tx: Option<tokio::sync::oneshot::Sender<u16>>,
    pre_bound_listener: Option<TcpListener>,
) -> Result<()> {
    // Ensure all cells are loaded (including http)
    let registry = all();

    // Check that the http cell is loaded
    if registry.http.is_none() {
        return Err(eyre::eyre!(
            "HTTP cell not loaded. Build it with: cargo build -p cell-http --bin ddc-cell-http"
        ));
    }

    // Get the raw session to set up ContentService dispatcher
    let session = get_cell_session("ddc-cell-http")
        .ok_or_else(|| eyre::eyre!("HTTP cell session not found"))?;

    tracing::info!("HTTP cell connected via hub");

    // Create the ContentService implementation
    let content_service = Arc::new(HostContentService::new(server));

    // Set up multi-service dispatcher on the http cell's session
    // This replaces the basic dispatcher from cells.rs with one that includes ContentService
    session.set_dispatcher(create_http_cell_dispatcher(content_service));

    // Start TCP listeners for browser connections
    let (listeners, bound_port) = if let Some(listener) = pre_bound_listener {
        // Use the pre-bound listener from FD passing (for testing)
        let bound_port = listener
            .local_addr()
            .map_err(|e| eyre::eyre!("Failed to get pre-bound listener address: {}", e))?
            .port();
        tracing::info!("Using pre-bound listener on port {}", bound_port);

        // Print READY signal for test harness
        println!("READY");
        use std::io::Write;
        let _ = std::io::stdout().flush();

        (vec![listener], bound_port)
    } else {
        // Bind to requested IPs normally - one listener per IP
        let mut listeners = Vec::new();
        let mut actual_port: Option<u16> = None;
        for ip in &bind_ips {
            let addr = std::net::SocketAddr::new(std::net::IpAddr::V4(*ip), port);
            match TcpListener::bind(addr).await {
                Ok(listener) => {
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
            return Err(eyre::eyre!("Failed to bind to any addresses"));
        }

        let bound_port =
            actual_port.ok_or_else(|| eyre::eyre!("Could not determine bound port"))?;

        // Wait for required cells to be ready before declaring the server ready
        // This prevents the race condition where tests/clients connect before cells can handle RPCs
        let required_cells = ["ddc-cell-http", "ddc-cell-markdown"];
        let timeout = std::time::Duration::from_secs(5);

        tracing::info!(
            "Waiting for required cells to be ready: {:?}",
            required_cells
        );
        if let Err(e) = crate::cells::wait_for_cells_ready(&required_cells, timeout).await {
            tracing::warn!(
                "Timeout waiting for cells to be ready: {}. Proceeding anyway.",
                e
            );
        }

        eprintln!("DEBUG: About to print LISTENING_PORT={}", bound_port);
        println!("LISTENING_PORT={}", bound_port);
        use std::io::Write;
        let _ = std::io::stdout().flush();
        eprintln!("DEBUG: Flushed LISTENING_PORT");

        (listeners, bound_port)
    };

    // Send the bound port back to the caller (if channel provided)
    if let Some(tx) = port_tx {
        let _ = tx.send(bound_port);
    }

    // Merge all listeners into a single stream
    let mut accept_stream = stream::select_all(listeners.into_iter().map(TcpListenerStream::new));

    // Accept browser connections and tunnel them to the cell
    loop {
        tokio::select! {
            accept_result = accept_stream.next() => {
                match accept_result {
                    Some(Ok(stream)) => {
                        let addr = stream.peer_addr().ok();
                        tracing::debug!("Accepted browser connection from {:?}", addr);
                        let session = session.clone();
                        tokio::spawn(async move {
                            // Create TcpTunnelClient per connection
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
                        tracing::info!("All listeners closed");
                        break;
                    }
                }
            }
            _ = async {
                if let Some(ref mut rx) = shutdown_rx {
                    rx.changed().await.ok();
                    if *rx.borrow() {
                        return;
                    }
                }
                std::future::pending::<()>().await
            } => {
                tracing::info!("Shutdown signal received, stopping HTTP server");
                break;
            }
        }
    }

    // Cleanup: drop the stream to release listeners
    drop(accept_stream);

    Ok(())
}

/// Start the cell server (convenience wrapper without shutdown signal)
#[allow(dead_code)]
pub async fn start_cell_server(
    server: Arc<SiteServer>,
    cell_path: std::path::PathBuf,
    bind_ips: Vec<std::net::Ipv4Addr>,
    port: u16,
) -> Result<()> {
    start_cell_server_with_shutdown(server, cell_path, bind_ips, port, None, None, None).await
}

/// Handle a browser TCP connection by tunneling it through the cell
async fn handle_browser_connection(
    browser_stream: TcpStream,
    tunnel_client: TcpTunnelClient,
    session: Arc<RpcSession>,
) -> Result<()> {
    // Open a tunnel to the cell
    let handle = tunnel_client
        .open()
        .await
        .map_err(|e| eyre::eyre!("Failed to open tunnel: {:?}", e))?;

    let channel_id = handle.channel_id;
    tracing::debug!(channel_id, "Tunnel opened for browser connection");

    // Register the tunnel to receive incoming chunks from cell
    let mut tunnel_rx = session.register_tunnel(channel_id);

    let (mut browser_read, mut browser_write) = browser_stream.into_split();

    // Task A: Browser → rapace (read from browser, send to tunnel)
    let session_a = session.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; CHUNK_SIZE];
        loop {
            match browser_read.read(&mut buf).await {
                Ok(0) => {
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
            let payload = chunk.payload_bytes();
            if !payload.is_empty()
                && let Err(e) = browser_write.write_all(payload).await
            {
                tracing::debug!(channel_id, error = %e, "Browser write error");
                break;
            }
            if chunk.is_eos() {
                tracing::debug!(channel_id, "Received EOS from cell");
                let _ = browser_write.shutdown().await;
                break;
            }
        }
        tracing::debug!(channel_id, "rapace→browser task finished");
    });

    Ok(())
}
