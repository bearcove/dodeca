//! HTTP cell server for rapace RPC communication
//!
//! This module handles:
//! - Setting up ContentService on the http cell's session (via hub)
//! - Handling TCP connections from browsers via TcpTunnel
//!
//! The http cell is loaded through the hub like all other cells.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use eyre::Result;
use rapace::{Frame, RpcError, RpcSession};
use rapace_tracing::{EventMeta, Field, SpanMeta, TracingSink};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::watch;

use cell_http_proto::{ContentServiceServer, TcpTunnelClient};
use rapace_cell::CellLifecycleServer;
use rapace_tracing::TracingSinkServer;

use crate::boot_state::{BootPhase, BootState, BootStateManager, ErrorKind};
use crate::cells::{HostCellLifecycle, all, cell_ready_registry, get_cell_session};
use crate::content_service::HostContentService;
use crate::serve::SiteServer;

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);
const REQUIRED_CELLS: [&str; 2] = ["ddc-cell-http", "ddc-cell-markdown"];
const REQUIRED_CELL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const SESSION_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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

use std::sync::atomic::AtomicU64;

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
///
/// # Boot State Contract
/// - The accept loop is NEVER aborted due to cell loading failures
/// - Connections are accepted immediately and held open
/// - If boot fails fatally, connections receive HTTP 500 responses
/// - If boot succeeds, connections are tunneled to the HTTP cell
#[allow(clippy::too_many_arguments)]
pub async fn start_cell_server_with_shutdown(
    server: Arc<SiteServer>,
    _cell_path: std::path::PathBuf, // No longer used - cells loaded via hub
    bind_ips: Vec<std::net::Ipv4Addr>,
    port: u16,
    shutdown_rx: Option<watch::Receiver<bool>>,
    port_tx: Option<tokio::sync::oneshot::Sender<u16>>,
    pre_bound_listener: Option<std::net::TcpListener>,
) -> Result<()> {
    // Create boot state manager - tracks lifecycle for connection handlers
    let boot_state = Arc::new(BootStateManager::new());
    let boot_state_rx = boot_state.subscribe();

    // Start TCP listeners for browser connections
    let (listeners, bound_port) = if let Some(listener) = pre_bound_listener {
        // Use the pre-bound listener from FD passing (for testing)
        let bound_port = listener
            .local_addr()
            .map_err(|e| eyre::eyre!("Failed to get pre-bound listener address: {}", e))?
            .port();
        if let Err(e) = listener.set_nonblocking(true) {
            tracing::warn!("Failed to set pre-bound listener non-blocking: {}", e);
        }
        tracing::info!("Using pre-bound listener on port {}", bound_port);

        (vec![listener], bound_port)
    } else {
        // Bind to requested IPs normally - one listener per IP
        let mut listeners = Vec::new();
        let mut actual_port: Option<u16> = None;
        for ip in &bind_ips {
            let requested_port = actual_port.unwrap_or(port);
            let addr = std::net::SocketAddr::new(std::net::IpAddr::V4(*ip), requested_port);
            match std::net::TcpListener::bind(addr) {
                Ok(listener) => {
                    if let Ok(bound_addr) = listener.local_addr() {
                        let bound_port = bound_addr.port();
                        if actual_port.is_none() {
                            actual_port = Some(bound_port);
                        }
                        tracing::info!("Listening on {}:{}", ip, bound_port);
                    }
                    if let Err(e) = listener.set_nonblocking(true) {
                        tracing::warn!("Failed to set non-blocking on {}: {}", ip, e);
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

    let shutdown_flag = Arc::new(AtomicBool::new(false));
    if let Some(mut shutdown_rx) = shutdown_rx.clone() {
        let shutdown_flag = shutdown_flag.clone();
        tokio::spawn(async move {
            let _ = shutdown_rx.changed().await;
            if *shutdown_rx.borrow() {
                shutdown_flag.store(true, Ordering::Relaxed);
            }
        });
    }

    // Part 4.1: Convert std listeners to tokio listeners for direct async accept.
    // This eliminates the thread+poll design that introduced cross-thread races.
    let tokio_listeners: Vec<tokio::net::TcpListener> = listeners
        .into_iter()
        .filter_map(|l| match tokio::net::TcpListener::from_std(l) {
            Ok(listener) => Some(listener),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to convert listener to tokio");
                None
            }
        })
        .collect();

    if tokio_listeners.is_empty() {
        return Err(eyre::eyre!(
            "No listeners available after conversion to tokio"
        ));
    }

    // Start accepting connections immediately; readiness is gated per-connection.
    // CRITICAL: This task is NEVER aborted - it runs until shutdown.
    let accept_server = server.clone();
    let accept_boot_state_rx = boot_state_rx.clone();
    let accept_task = tokio::spawn(async move {
        run_async_accept_loop(
            tokio_listeners,
            session_rx,
            accept_boot_state_rx,
            accept_server,
            shutdown_rx,
            shutdown_flag,
        )
        .await
    });

    // Load cells with boot state tracking
    boot_state.set_phase(BootPhase::LoadingCells);
    let registry = all().await;

    // Check that the http cell is loaded - but don't abort accept loop!
    if registry.http.is_none() {
        boot_state.set_fatal(
            ErrorKind::MissingCell,
            "HTTP cell not loaded. Build it with: cargo build -p cell-http --bin ddc-cell-http",
        );
        // Don't abort - let accept loop run and serve HTTP 500s
        return accept_task
            .await
            .map_err(|e| eyre::eyre!("Accept loop task failed: {}", e))?;
    }

    // Get the raw session to set up ContentService dispatcher
    let session = match get_cell_session("ddc-cell-http") {
        Some(session) => session,
        None => {
            boot_state.set_fatal(
                ErrorKind::CellStartupFailed,
                "HTTP cell session not found after loading",
            );
            // Don't abort - let accept loop run and serve HTTP 500s
            return accept_task
                .await
                .map_err(|e| eyre::eyre!("Accept loop task failed: {}", e))?;
        }
    };

    tracing::info!("HTTP cell connected via hub");

    // Create the ContentService implementation
    let content_service = Arc::new(HostContentService::new(server.clone()));

    // Set up multi-service dispatcher on the http cell's session
    // This replaces the basic dispatcher from cells.rs with one that includes ContentService
    session.set_dispatcher(create_http_cell_dispatcher(content_service));

    // Signal that the session is ready
    let _ = session_tx.send(Some(session.clone()));
    tracing::info!("HTTP cell session ready");

    // Wait for required cells to be ready
    boot_state.set_phase(BootPhase::WaitingCellsReady);
    if let Err(e) = crate::cells::wait_for_cells_ready(&REQUIRED_CELLS, REQUIRED_CELL_TIMEOUT).await
    {
        boot_state.set_fatal(
            ErrorKind::CellStartupFailed,
            format!("Required cells not ready: {}", e),
        );
        return accept_task
            .await
            .map_err(|e| eyre::eyre!("Accept loop task failed: {}", e))?;
    }

    // Mark boot as complete - connections can now be fully handled
    boot_state.set_ready(1);

    match accept_task.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(eyre::eyre!("Accept loop task failed: {}", e)),
    }
}

/// Part 4.1: Async accept loop using tokio::net::TcpListener directly.
/// This eliminates the thread+poll design that introduced cross-thread races
/// and OS scheduling differences between macOS and Linux.
async fn run_async_accept_loop(
    listeners: Vec<tokio::net::TcpListener>,
    session_rx: watch::Receiver<Option<Arc<RpcSession>>>,
    boot_state_rx: watch::Receiver<BootState>,
    server: Arc<SiteServer>,
    mut shutdown_rx: Option<watch::Receiver<bool>>,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
    tracing::info!(
        session_ready = session_rx.borrow().is_some(),
        boot_state = ?*boot_state_rx.borrow(),
        num_listeners = listeners.len(),
        "Async accept loop starting (Part 4.1)"
    );

    tracing::info!(
        "Accepting connections immediately; readiness gated per connection (cells={:?})",
        REQUIRED_CELLS
    );

    let accept_start = Instant::now();
    let mut accept_seq: u64 = 0;

    // For simplicity with multiple listeners, we'll use the first one.
    // In FD-passing mode there's only one listener anyway.
    // For multi-listener cases, we could use tokio::select! with all of them.
    let listener = listeners
        .into_iter()
        .next()
        .ok_or_else(|| eyre::eyre!("No listeners available"))?;

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                accept_seq = accept_seq.wrapping_add(1);
                let (stream, addr) = match accept_result {
                    Ok((s, a)) => (s, a),
                    Err(e) => {
                        tracing::warn!(error = %e, "Accept error");
                        continue;
                    }
                };

                let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
                let local_addr = stream.local_addr().ok();

                // Inspect SO_LINGER without taking ownership
                let linger_info = {
                    use socket2::SockRef;
                    let sock_ref = SockRef::from(&stream);
                    sock_ref.linger().ok()
                };

                tracing::trace!(
                    conn_id,
                    peer_addr = ?addr,
                    ?local_addr,
                    accept_seq,
                    elapsed_ms = accept_start.elapsed().as_millis(),
                    ?linger_info,
                    "Accepted browser connection"
                );

                let session_rx = session_rx.clone();
                let boot_state_rx = boot_state_rx.clone();
                let server = server.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        handle_browser_connection(
                            conn_id,
                            stream,
                            session_rx,
                            boot_state_rx,
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

    shutdown_flag.store(true, Ordering::Relaxed);
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

/// HTTP 500 response for fatal boot errors
const FATAL_ERROR_RESPONSE: &[u8] = b"HTTP/1.1 500 Internal Server Error\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Connection: close\r\n\
Content-Length: 52\r\n\
\r\n\
Server failed to start. Check server logs for details";

/// Handle a browser TCP connection by tunneling it through the cell
///
/// # Boot State Contract
/// - Connection is held open while waiting for boot to complete
/// - If boot succeeds (Ready), tunnel to HTTP cell
/// - If boot fails (Fatal), send HTTP 500 response
/// - Never reset or refuse the connection
async fn handle_browser_connection(
    conn_id: u64,
    mut browser_stream: TcpStream,
    mut session_rx: watch::Receiver<Option<Arc<RpcSession>>>,
    mut boot_state_rx: watch::Receiver<BootState>,
    server: Arc<SiteServer>,
) -> Result<()> {
    let started_at = Instant::now();
    let peer_addr = browser_stream.peer_addr().ok();
    let local_addr = browser_stream.local_addr().ok();
    tracing::trace!(
        conn_id,
        ?peer_addr,
        ?local_addr,
        "handle_browser_connection: start"
    );

    if let Ok(Some(err)) = browser_stream.take_error() {
        tracing::warn!(conn_id, error = %err, "socket error present on accept");
    }

    // Wait for boot state to resolve (Ready or Fatal)
    // This replaces the old "read 1 byte first" hack - we hold the connection
    // open while waiting, the client's request sits in the kernel receive buffer.
    let boot_wait_start = Instant::now();
    loop {
        let state = boot_state_rx.borrow().clone();
        match state {
            BootState::Ready { generation, .. } => {
                tracing::info!(
                    conn_id,
                    generation,
                    elapsed_ms = boot_wait_start.elapsed().as_millis(),
                    "Boot ready, proceeding with connection"
                );
                break;
            }
            BootState::Fatal {
                ref message,
                ref error_kind,
                ..
            } => {
                tracing::warn!(
                    conn_id,
                    error_kind = ?error_kind,
                    message = %message,
                    elapsed_ms = boot_wait_start.elapsed().as_millis(),
                    "Boot failed fatally, sending HTTP 500"
                );
                // Send HTTP 500 response instead of closing/resetting
                if let Err(e) = browser_stream.write_all(FATAL_ERROR_RESPONSE).await {
                    tracing::warn!(conn_id, error = %e, "Failed to write 500 response");
                }
                return Ok(());
            }
            BootState::Booting { ref phase, .. } => {
                tracing::debug!(
                    conn_id,
                    phase = ?phase,
                    elapsed_ms = boot_wait_start.elapsed().as_millis(),
                    "Waiting for boot to complete"
                );
            }
        }
        // Wait for state change
        if boot_state_rx.changed().await.is_err() {
            tracing::warn!(conn_id, "Boot state channel closed unexpectedly");
            return Ok(());
        }
    }

    // Get HTTP cell session
    let session = if let Some(session) = session_rx.borrow().clone() {
        session
    } else {
        tracing::info!(conn_id, "Waiting for HTTP cell session (socket held open)");
        let wait_start = Instant::now();
        let session = tokio::time::timeout(SESSION_WAIT_TIMEOUT, async {
            loop {
                session_rx.changed().await.ok();
                if let Some(session) = session_rx.borrow().clone() {
                    break session;
                }
            }
        })
        .await;
        match session {
            Ok(session) => {
                tracing::info!(
                    conn_id,
                    elapsed_ms = wait_start.elapsed().as_millis(),
                    "HTTP cell session ready (per-connection)"
                );
                session
            }
            Err(_) => {
                tracing::warn!(
                    conn_id,
                    elapsed_ms = wait_start.elapsed().as_millis(),
                    "HTTP cell session wait timed out; sending 500"
                );
                if let Err(e) = browser_stream.write_all(FATAL_ERROR_RESPONSE).await {
                    tracing::warn!(conn_id, error = %e, "Failed to write 500 response");
                }
                return Ok(());
            }
        }
    };

    let tunnel_client = TcpTunnelClient::new(session.clone());

    // Wait for revision readiness (site content built)
    tracing::trace!(conn_id, "Waiting for revision readiness (per-connection)");
    let revision_start = Instant::now();
    server.wait_revision_ready().await;
    tracing::trace!(
        conn_id,
        elapsed_ms = revision_start.elapsed().as_millis(),
        "Revision ready (per-connection)"
    );

    // Open a tunnel to the cell
    let open_started = Instant::now();
    let handle = tunnel_client
        .open()
        .await
        .map_err(|e| eyre::eyre!("Failed to open tunnel: {:?}", e))?;

    let channel_id = handle.channel_id;
    tracing::trace!(
        conn_id,
        channel_id,
        open_elapsed_ms = open_started.elapsed().as_millis(),
        "Tunnel opened for browser connection"
    );

    // Bridge browser <-> tunnel with backpressure.
    // No need to pre-read bytes - the request is in the kernel buffer.
    let mut tunnel_stream = session.tunnel_stream(channel_id);
    tracing::trace!(
        conn_id,
        channel_id,
        "Starting browser <-> tunnel bridge task"
    );
    tokio::spawn(async move {
        let bridge_started = Instant::now();
        tracing::trace!(conn_id, channel_id, "browser <-> tunnel bridge: start");
        match tokio::io::copy_bidirectional(&mut browser_stream, &mut tunnel_stream).await {
            Ok((to_tunnel, to_browser)) => {
                tracing::trace!(
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
        tracing::trace!(
            conn_id,
            channel_id,
            elapsed_ms = bridge_started.elapsed().as_millis(),
            "browser <-> tunnel bridge: done"
        );
    });

    tracing::trace!(
        conn_id,
        channel_id,
        elapsed_ms = started_at.elapsed().as_millis(),
        "handle_browser_connection: end"
    );
    Ok(())
}
