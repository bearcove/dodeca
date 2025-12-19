//! HTTP cell server for rapace RPC communication
//!
//! This module handles:
//! - Setting up ContentService on the http cell's session (via hub)
//! - Handling TCP connections from browsers via TcpTunnel
//!
//! The http cell is loaded through the hub like all other cells.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use eyre::Result;
use futures::stream::{self, StreamExt};
use rapace::{Frame, RpcError, RpcSession};
use rapace_tracing::{EventMeta, Field, SpanMeta, TracingSink};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_stream::wrappers::TcpListenerStream;

use cell_http_proto::{ContentServiceServer, TcpTunnelClient};
use rapace_cell::CellLifecycleServer;
use rapace_tracing::TracingSinkServer;

use crate::cells::{HostCellLifecycle, all, cell_ready_registry, get_cell_session};
use crate::content_service::HostContentService;
use crate::serve::SiteServer;

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);
const REQUIRED_CELLS: [&str; 2] = ["ddc-cell-http", "ddc-cell-markdown"];
const REQUIRED_CELL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Find the cell binary path (for backwards compatibility).
///
/// Note: The http cell is now loaded via the hub, so this just returns a dummy path.
/// The actual cell location is determined by cells.rs.
pub fn find_cell_path() -> Result<std::path::PathBuf> {
    // Return a dummy path - cells are loaded via hub now
    Ok(std::path::PathBuf::from("ddc-cell-http"))
}

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
    // Start TCP listeners for browser connections
    let (listeners, bound_port) = if let Some(listener) = pre_bound_listener {
        // Use the pre-bound listener from FD passing (for testing)
        let bound_port = listener
            .local_addr()
            .map_err(|e| eyre::eyre!("Failed to get pre-bound listener address: {}", e))?
            .port();
        tracing::info!("Using pre-bound listener on port {}", bound_port);

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

        (listeners, bound_port)
    };

    // Send the bound port back to the caller (if channel provided)
    if let Some(tx) = port_tx {
        let _ = tx.send(bound_port);
    }

    tracing::debug!(port = bound_port, "BOUND");

    let (session_tx, session_rx) = watch::channel::<Option<Arc<RpcSession>>>(None);

    // Start accepting connections immediately; readiness is gated per-connection.
    let accept_server = server.clone();
    let accept_task = tokio::spawn(async move {
        run_accept_loop(listeners, session_rx, accept_server, shutdown_rx).await
    });

    // Ensure all cells are loaded (including http)
    let registry = all().await;

    // Check that the http cell is loaded
    if registry.http.is_none() {
        accept_task.abort();
        return Err(eyre::eyre!(
            "HTTP cell not loaded. Build it with: cargo build -p cell-http --bin ddc-cell-http"
        ));
    }

    // Get the raw session to set up ContentService dispatcher
    let session = match get_cell_session("ddc-cell-http") {
        Some(session) => session,
        None => {
            accept_task.abort();
            return Err(eyre::eyre!("HTTP cell session not found"));
        }
    };

    tracing::info!("HTTP cell connected via hub");

    // Create the ContentService implementation
    let content_service = Arc::new(HostContentService::new(server.clone()));

    // Set up multi-service dispatcher on the http cell's session
    // This replaces the basic dispatcher from cells.rs with one that includes ContentService
    session.set_dispatcher(create_http_cell_dispatcher(content_service));

    let _ = session_tx.send(Some(session.clone()));
    tracing::info!("HTTP cell session ready (accept loop can proceed)");

    match accept_task.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(eyre::eyre!("Accept loop task failed: {}", e)),
    }
}

async fn run_accept_loop(
    listeners: Vec<TcpListener>,
    session_rx: watch::Receiver<Option<Arc<RpcSession>>>,
    server: Arc<SiteServer>,
    mut shutdown_rx: Option<watch::Receiver<bool>>,
) -> Result<()> {
    tracing::info!(
        session_ready = session_rx.borrow().is_some(),
        "Accept loop starting"
    );

    tracing::info!(
        "Accepting connections immediately; readiness gated per connection (cells={:?})",
        REQUIRED_CELLS
    );

    // Merge all listeners into a single stream
    let mut accept_stream = stream::select_all(listeners.into_iter().map(TcpListenerStream::new));

    // Accept browser connections and tunnel them to the cell
    tracing::info!("Accepting connections");
    let accept_start = std::time::Instant::now();
    let mut accept_seq: u64 = 0;
    loop {
        tracing::debug!(
            accept_seq,
            elapsed_ms = accept_start.elapsed().as_millis(),
            "Accept loop: waiting for connection..."
        );
        tokio::select! {
            accept_result = accept_stream.next() => {
                tracing::debug!(
                    accept_seq,
                    elapsed_ms = accept_start.elapsed().as_millis(),
                    accept_result = ?accept_result.as_ref().map(|r| r.as_ref().map(|_| "stream").map_err(|e| e.to_string())),
                    "Accept loop: got accept_result"
                );
                accept_seq = accept_seq.wrapping_add(1);
                match accept_result {
                    Some(Ok(stream)) => {
                        let addr = stream.peer_addr().ok();
                        let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
                        let local_addr = stream.local_addr().ok();
                        tracing::info!(
                            conn_id,
                            ?addr,
                            ?local_addr,
                            accept_seq,
                            elapsed_ms = accept_start.elapsed().as_millis(),
                            "Accepted browser connection"
                        );
                        let session_rx = session_rx.clone();
                        let server = server.clone();
                        tokio::spawn(async move {
                            if let Err(e) =
                                handle_browser_connection(
                                    conn_id,
                                    stream,
                                    session_rx,
                                    server,
                                )
                                .await
                            {
                                tracing::warn!(
                                    conn_id,
                                    error = ?e,
                                    "Failed to handle browser connection"
                                );
                            }
                        });
                    }
                    Some(Err(e)) => {
                        tracing::error!(
                            accept_seq,
                            elapsed_ms = accept_start.elapsed().as_millis(),
                            error = ?e,
                            kind = ?e.kind(),
                            raw_os_error = e.raw_os_error(),
                            "Accept error"
                        );
                    }
                    None => {
                        tracing::info!(
                            accept_seq,
                            elapsed_ms = accept_start.elapsed().as_millis(),
                            "All listeners closed"
                        );
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
    conn_id: u64,
    browser_stream: TcpStream,
    mut session_rx: watch::Receiver<Option<Arc<RpcSession>>>,
    server: Arc<SiteServer>,
) -> Result<()> {
    let started_at = Instant::now();
    let peer_addr = browser_stream.peer_addr().ok();
    let local_addr = browser_stream.local_addr().ok();
    let mut browser_stream = browser_stream;
    tracing::info!(
        conn_id,
        ?peer_addr,
        ?local_addr,
        "handle_browser_connection: start"
    );

    if let Ok(Some(err)) = browser_stream.take_error() {
        tracing::warn!(conn_id, error = %err, "socket error present on accept");
    }

    let mut pre_session_buf: Vec<u8> = Vec::new();
    let mut pre_session_reads: u64 = 0;

    let session = if let Some(session) = session_rx.borrow().clone() {
        session
    } else {
        tracing::info!(conn_id, "Waiting for HTTP cell session");
        let wait_start = Instant::now();
        loop {
            tokio::select! {
                _ = session_rx.changed() => {
                    if let Some(session) = session_rx.borrow().clone() {
                        tracing::info!(
                            conn_id,
                            elapsed_ms = wait_start.elapsed().as_millis(),
                            pre_session_reads,
                            pre_session_bytes = pre_session_buf.len(),
                            "HTTP cell session ready (per-connection)"
                        );
                        break session;
                    }
                }
                ready = browser_stream.readable() => {
                    if let Err(e) = ready {
                        tracing::warn!(conn_id, error = %e, "browser stream readable error while waiting for session");
                        continue;
                    }
                    let mut buf = [0u8; 4096];
                    match browser_stream.try_read(&mut buf) {
                        Ok(0) => {
                            tracing::warn!(
                                conn_id,
                                elapsed_ms = wait_start.elapsed().as_millis(),
                                pre_session_reads,
                                pre_session_bytes = pre_session_buf.len(),
                                "browser stream closed before session ready (read=0)"
                            );
                            return Ok(());
                        }
                        Ok(n) => {
                            pre_session_reads = pre_session_reads.wrapping_add(1);
                            pre_session_buf.extend_from_slice(&buf[..n]);
                            tracing::info!(
                                conn_id,
                                n,
                                pre_session_reads,
                                pre_session_bytes = pre_session_buf.len(),
                                "read bytes while waiting for session"
                            );
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) => {
                            return Err(e.into());
                        }
                    }
                }
            }
        }
    };

    let tunnel_client = TcpTunnelClient::new(session.clone());

    tracing::info!(conn_id, "Waiting for required cells (per-connection)");
    let required_start = Instant::now();
    if let Err(e) = crate::cells::wait_for_cells_ready(&REQUIRED_CELLS, REQUIRED_CELL_TIMEOUT).await
    {
        tracing::error!(
            conn_id,
            error = %e,
            elapsed_ms = required_start.elapsed().as_millis(),
            "Required cells not ready"
        );
        return Err(e);
    }
    tracing::info!(
        conn_id,
        elapsed_ms = required_start.elapsed().as_millis(),
        "Required cells ready (per-connection)"
    );

    tracing::info!(conn_id, "Waiting for revision readiness (per-connection)");
    let revision_start = Instant::now();
    server.wait_revision_ready().await;
    tracing::info!(
        conn_id,
        elapsed_ms = revision_start.elapsed().as_millis(),
        "Revision ready (per-connection)"
    );

    if pre_session_buf.is_empty() {
        // Read first byte before opening a tunnel.
        let mut first_byte = [0u8; 1];
        let first_read = browser_stream.read(&mut first_byte).await?;
        if first_read == 0 {
            tracing::warn!(
                conn_id,
                "browser stream closed before request (first_read=0)"
            );
            return Ok(());
        }
        pre_session_buf.extend_from_slice(&first_byte[..first_read]);
    }

    // Open a tunnel to the cell
    let open_started = Instant::now();
    let handle = tunnel_client
        .open()
        .await
        .map_err(|e| eyre::eyre!("Failed to open tunnel: {:?}", e))?;

    let channel_id = handle.channel_id;
    tracing::info!(
        conn_id,
        channel_id,
        open_elapsed_ms = open_started.elapsed().as_millis(),
        initial_bytes = pre_session_buf.len(),
        "Tunnel opened for browser connection"
    );

    // Bridge browser <-> tunnel with backpressure.
    let mut tunnel_stream = session.tunnel_stream(channel_id);
    if !pre_session_buf.is_empty() {
        tunnel_stream.write_all(&pre_session_buf).await?;
    }
    tracing::info!(
        conn_id,
        channel_id,
        "Starting browser <-> tunnel bridge task"
    );
    tokio::spawn(async move {
        let bridge_started = Instant::now();
        tracing::info!(conn_id, channel_id, "browser <-> tunnel bridge: start");
        match tokio::io::copy_bidirectional(&mut browser_stream, &mut tunnel_stream).await {
            Ok((to_tunnel, to_browser)) => {
                tracing::info!(
                    conn_id,
                    channel_id,
                    to_tunnel,
                    to_browser,
                    elapsed_ms = bridge_started.elapsed().as_millis(),
                    "browser <-> tunnel finished"
                );
            }
            Err(e) => {
                tracing::warn!(
                    conn_id,
                    channel_id,
                    error = %e,
                    elapsed_ms = bridge_started.elapsed().as_millis(),
                    "browser <-> tunnel error"
                );
            }
        }
        tracing::info!(
            conn_id,
            channel_id,
            elapsed_ms = bridge_started.elapsed().as_millis(),
            "browser <-> tunnel bridge: done"
        );
    });

    tracing::info!(
        conn_id,
        channel_id,
        elapsed_ms = started_at.elapsed().as_millis(),
        "handle_browser_connection: end"
    );
    Ok(())
}
