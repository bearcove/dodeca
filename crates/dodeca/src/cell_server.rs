//! HTTP cell server for roam RPC communication
//!
//! This module handles:
//! - Setting up ContentService on the http cell's session (via hub)
//! - Handling TCP connections from browsers via TcpTunnel
//! - Accepting virtual connections from browsers through cell-http
//!
//! The http cell is loaded through the hub like all other cells.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use eyre::Result;
use roam::{Tunnel, tunnel_pair};
use roam_shm::driver::IncomingConnections;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::watch;

use cell_http_proto::{TcpTunnelClient, WebSocketTunnel};
use dodeca_protocol::BrowserServiceClient;

use crate::boot_state::BootStateManager;
use crate::host::Host;
use crate::serve::SiteServer;

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

/// Find the cell binary path (for backwards compatibility).
///
/// Note: The http cell is now loaded via the hub, so this just returns a dummy path.
/// The actual cell location is determined by cells.rs.
pub fn find_cell_path() -> Result<std::path::PathBuf> {
    // Return a dummy path - cells are loaded via hub now
    Ok(std::path::PathBuf::from("ddc-cell-http"))
}

// Note: Cell tracing is handled by roam_tracing's TracingHost which subscribes
// to each cell's CellTracing service. See cells.rs for the host-side setup.

// ============================================================================
// HostWebSocketTunnel - handles devtools WebSocket tunnel from cell
// ============================================================================

/// Host-side implementation of WebSocketTunnel for devtools connections.
///
/// When the cell receives a WebSocket connection at /_/ws, it calls this service
/// to open a tunnel back to the host. The host then handles the devtools protocol
/// (ClientMessage/ServerMessage) and forwards LiveReload broadcasts.
#[derive(Clone)]
pub struct HostWebSocketTunnel {
    server: Arc<SiteServer>,
}

impl HostWebSocketTunnel {
    pub fn new(server: Arc<SiteServer>) -> Self {
        Self { server }
    }
}

impl WebSocketTunnel for HostWebSocketTunnel {
    async fn open(&self, _cx: &roam::Context, tunnel: Tunnel) {
        let channel_id = tunnel.tx.channel_id();
        tracing::debug!(channel_id, "DevTools WebSocket tunnel opened on host");

        // Spawn a task to handle the devtools protocol on this tunnel
        let server = self.server.clone();
        tokio::spawn(async move {
            use futures_util::FutureExt;
            tracing::debug!(channel_id, "DevTools tunnel handler task spawned");
            let result =
                std::panic::AssertUnwindSafe(handle_devtools_tunnel(channel_id, tunnel, server))
                    .catch_unwind()
                    .await;
            match result {
                Ok(()) => {
                    tracing::debug!(channel_id, "DevTools tunnel handler task ended normally")
                }
                Err(e) => {
                    tracing::error!(channel_id, "DevTools tunnel handler task PANICKED: {:?}", e)
                }
            }
        });
    }
}

/// Handle a devtools tunnel connection.
///
/// This reads ClientMessage from the tunnel and sends ServerMessage back.
/// It also subscribes to LiveReload broadcasts and forwards them.
async fn handle_devtools_tunnel(channel_id: u64, tunnel: Tunnel, server: Arc<SiteServer>) {
    use dodeca_protocol::{ClientMessage, ServerMessage, facet_postcard};
    use roam::{DEFAULT_TUNNEL_CHUNK_SIZE, tunnel_stream};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    tracing::debug!(channel_id, "DevTools tunnel handler starting");

    // Track what route this client is currently viewing
    let mut current_route: Option<String> = None;

    // Subscribe to livereload broadcasts
    let mut livereload_rx = server.livereload_tx.subscribe();
    tracing::debug!(channel_id, "Subscribed to livereload broadcasts");

    // Create a duplex for the tunnel
    let (client, server_stream) = tokio::io::duplex(64 * 1024);
    let (read_handle, write_handle) = tunnel_stream(client, tunnel, DEFAULT_TUNNEL_CHUNK_SIZE);

    // Monitor the pump tasks for errors
    let read_handle_channel_id = channel_id;
    let write_handle_channel_id = channel_id;
    tokio::spawn(async move {
        match read_handle.await {
            Ok(Ok(())) => tracing::debug!(read_handle_channel_id, "tunnel read pump completed ok"),
            Ok(Err(e)) => {
                tracing::warn!(read_handle_channel_id, error = %e, "tunnel read pump error")
            }
            Err(e) => {
                tracing::warn!(read_handle_channel_id, error = %e, "tunnel read pump task panicked")
            }
        }
    });
    tokio::spawn(async move {
        match write_handle.await {
            Ok(Ok(())) => {
                tracing::debug!(write_handle_channel_id, "tunnel write pump completed ok")
            }
            Ok(Err(e)) => {
                tracing::warn!(write_handle_channel_id, error = %e, "tunnel write pump error")
            }
            Err(e) => {
                tracing::warn!(write_handle_channel_id, error = %e, "tunnel write pump task panicked")
            }
        }
    });

    // Split for concurrent read/write
    let (mut read_half, mut write_half) = tokio::io::split(server_stream);
    tracing::debug!(channel_id, "Split tunnel stream into read/write halves");

    // Buffer for reading messages
    let mut read_buf = vec![0u8; 64 * 1024];

    // Send any existing errors to the newly connected client
    let current_errors = server.get_current_errors();
    tracing::debug!(
        channel_id,
        error_count = current_errors.len(),
        "Sending existing errors to client"
    );
    for error in current_errors {
        let msg = ServerMessage::Error(error);
        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
            tracing::debug!(
                channel_id,
                bytes_len = bytes.len(),
                "Writing error message to tunnel"
            );
            if write_half.write_all(&bytes).await.is_err() {
                tracing::warn!(
                    channel_id,
                    "Failed to send initial error message, tunnel closed"
                );
                return;
            }
        }
    }

    tracing::debug!(channel_id, "Entering main loop, waiting for messages...");

    loop {
        tokio::select! {
            // Handle incoming messages from the cell (browser -> host)
            result = read_half.read(&mut read_buf) => {
                tracing::debug!(channel_id, "read_half.read returned: {:?}", result.as_ref().map(|n| *n));
                match result {
                    Ok(0) => {
                        tracing::debug!(channel_id, "DevTools tunnel closed (EOF)");
                        break;
                    }
                    Ok(n) => {
                        let bytes = &read_buf[..n];
                        match facet_postcard::from_slice::<ClientMessage>(bytes) {
                            Ok(msg) => {
                                tracing::debug!(channel_id, "Received client message: {:?}", msg);
                                match msg {
                                    ClientMessage::Route { path } => {
                                        tracing::debug!(channel_id, route = %path, "Client viewing route");
                                        current_route = Some(path);
                                    }
                                    ClientMessage::GetScope { request_id, snapshot_id, path } => {
                                        let route = snapshot_id.unwrap_or_else(|| "/".to_string());
                                        let path = path.unwrap_or_default();
                                        let scope = server.get_scope_for_route(&route, &path).await;
                                        let response = ServerMessage::ScopeResponse { request_id, scope };
                                        if let Ok(bytes) = facet_postcard::to_vec(&response) {
                                            if write_half.write_all(&bytes).await.is_err() {
                                                tracing::warn!(channel_id, "Failed to send scope response, tunnel closed");
                                                break;
                                            }
                                        }
                                    }
                                    ClientMessage::Eval { request_id, snapshot_id, expression } => {
                                        let result = match server.eval_expression_for_route(&snapshot_id, &expression).await {
                                            Ok(value) => dodeca_protocol::EvalResult::Ok(value),
                                            Err(e) => dodeca_protocol::EvalResult::Err(e),
                                        };
                                        let response = ServerMessage::EvalResponse { request_id, result };
                                        if let Ok(bytes) = facet_postcard::to_vec(&response) {
                                            if write_half.write_all(&bytes).await.is_err() {
                                                tracing::warn!(channel_id, "Failed to send eval response, tunnel closed");
                                                break;
                                            }
                                        }
                                    }
                                    ClientMessage::DismissError { route } => {
                                        tracing::debug!(channel_id, route = %route, "Client dismissed error");
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(channel_id, "Failed to parse client message: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(channel_id, error = %e, "DevTools tunnel read error");
                        break;
                    }
                }
            }

            // Handle LiveReload broadcasts (host -> browser)
            result = livereload_rx.recv() => {
                match result {
                    Ok(msg) => {
                        let server_msg = match msg {
                            crate::serve::LiveReloadMsg::Reload => Some(ServerMessage::Reload),
                            crate::serve::LiveReloadMsg::CssUpdate { path } => Some(ServerMessage::CssChanged { path }),
                            crate::serve::LiveReloadMsg::Patches { route, patches } => {
                                let dominated = current_route.as_ref().is_some_and(|r| r == &route);
                                if dominated {
                                    Some(ServerMessage::Patches(patches))
                                } else {
                                    tracing::trace!(
                                        channel_id,
                                        patch_route = %route,
                                        client_route = ?current_route,
                                        "Skipping patches for different route"
                                    );
                                    None
                                }
                            }
                            crate::serve::LiveReloadMsg::Error { route, message, template, line, snapshot_id } => {
                                Some(ServerMessage::Error(dodeca_protocol::ErrorInfo {
                                    route,
                                    message,
                                    template,
                                    line,
                                    column: None,
                                    source_snippet: None,
                                    snapshot_id,
                                    available_variables: vec![],
                                }))
                            }
                            crate::serve::LiveReloadMsg::ErrorResolved { route } => {
                                Some(ServerMessage::ErrorResolved { route })
                            }
                        };
                        if let Some(server_msg) = server_msg {
                            if let Ok(bytes) = facet_postcard::to_vec(&server_msg) {
                                if write_half.write_all(&bytes).await.is_err() {
                                    tracing::warn!(channel_id, "Failed to send LiveReload message");
                                    break;
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(channel_id, lagged = n, "LiveReload receiver lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!(channel_id, "LiveReload channel closed");
                        break;
                    }
                }
            }
        }
    }

    tracing::debug!(channel_id, "DevTools tunnel handler finished");
}

// ============================================================================
// HostDevtoolsService - roam RPC implementation of DevtoolsService
// ============================================================================

use dodeca_protocol::{DevtoolsEvent, DevtoolsService, EvalResult, ScopeEntry};

/// Host-side implementation of DevtoolsService for direct roam RPC.
///
/// This implements the `DevtoolsService` trait from `dodeca-protocol`,
/// allowing browser devtools to call methods directly via roam RPC
/// over WebSocket (proxied through cell-http via ForwardingDispatcher).
#[derive(Clone)]
pub struct HostDevtoolsService {
    server: Arc<SiteServer>,
}

impl HostDevtoolsService {
    pub fn new(server: Arc<SiteServer>) -> Self {
        Self { server }
    }
}

/// Returns a short summary of a DevtoolsEvent for logging
pub fn event_summary(event: &DevtoolsEvent) -> String {
    match event {
        DevtoolsEvent::Reload => "Reload".to_string(),
        DevtoolsEvent::CssChanged { path } => format!("CssChanged({})", path),
        DevtoolsEvent::Patches(patches) => format!("Patches(count={})", patches.len()),
        DevtoolsEvent::Error(info) => {
            let msg_preview: String = info.message.chars().take(50).collect();
            let ellipsis = if info.message.len() > 50 { "…" } else { "" };
            format!(
                "Error(route={}, msg={}{})",
                info.route, msg_preview, ellipsis
            )
        }
        DevtoolsEvent::ErrorResolved { route } => format!("ErrorResolved(route={})", route),
    }
}

impl DevtoolsService for HostDevtoolsService {
    /// Subscribe to devtools events for a route.
    ///
    /// This registers the browser's interest in a route. Events will be pushed
    /// via BrowserService::on_event() on the browser's virtual connection.
    ///
    /// Note: The actual browser registration happens when the virtual connection
    /// is accepted in `accept_browser_connections`. This method just sets the route.
    async fn subscribe(&self, _cx: &roam::Context, route: String) {
        tracing::info!(route = %route, "devtools: client subscribing to route");
        // TODO: We need to associate this subscription with the browser's connection.
        // For now, this is a placeholder - the browser is already registered when
        // its virtual connection was accepted. We'd need the connection context here
        // to call server.set_browser_route(browser_id, route).
        //
        // The current architecture doesn't pass the browser_id through.
        // We may need to refactor to pass context or use a different pattern.
    }

    /// Get scope entries for the current route.
    async fn get_scope(&self, _cx: &roam::Context, path: Option<Vec<String>>) -> Vec<ScopeEntry> {
        // Use "/" as default route - the client should call subscribe() first
        // to establish which route they're viewing
        let path = path.unwrap_or_default();
        self.server.get_scope_for_route("/", &path).await
    }

    /// Evaluate an expression in a snapshot's context.
    async fn eval(
        &self,
        _cx: &roam::Context,
        snapshot_id: String,
        expression: String,
    ) -> EvalResult {
        match self
            .server
            .eval_expression_for_route(&snapshot_id, &expression)
            .await
        {
            Ok(value) => EvalResult::Ok(value),
            Err(e) => EvalResult::Err(e),
        }
    }

    /// Dismiss an error notification.
    async fn dismiss_error(&self, _cx: &roam::Context, route: String) {
        tracing::debug!(route = %route, "Client dismissed error via RPC");
        // The existing implementation just logs this - errors are resolved
        // when the template successfully re-renders
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
    _cell_path: std::path::PathBuf,
    bind_ips: Vec<std::net::Ipv4Addr>,
    port: u16,
    shutdown_rx: Option<watch::Receiver<bool>>,
    port_tx: Option<tokio::sync::oneshot::Sender<u16>>,
    pre_bound_listener: Option<std::net::TcpListener>,
) -> Result<()> {
    // Provide SiteServer for HTTP cell initialization (must be before all())
    crate::cells::provide_site_server(server.clone());

    // Create boot state manager
    let boot_state = Arc::new(BootStateManager::new());
    let _boot_state_rx = boot_state.subscribe();

    // Start TCP listeners for browser connections
    let (listeners, bound_port) = if let Some(listener) = pre_bound_listener {
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

    // Send the bound port back to the caller
    if let Some(tx) = port_tx {
        let _ = tx.send(bound_port);
    }

    tracing::debug!(port = bound_port, "BOUND");

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

    // Convert std listeners to tokio listeners
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

    // Start accepting connections immediately
    let accept_server = server.clone();
    let accept_task = tokio::spawn(async move {
        run_async_accept_loop(tokio_listeners, accept_server, shutdown_rx, shutdown_flag).await
    });

    // Accept loop will spawn cells lazily on first connection

    match accept_task.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(eyre::eyre!("Accept loop task failed: {}", e)),
    }
}

/// Async accept loop
async fn run_async_accept_loop(
    listeners: Vec<tokio::net::TcpListener>,
    server: Arc<SiteServer>,
    shutdown_rx: Option<watch::Receiver<bool>>,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
    tracing::debug!(
        num_listeners = listeners.len(),
        "Accept loop starting - cells will spawn on demand"
    );

    // Spawn accept tasks for each listener
    let mut accept_handles = Vec::new();
    for listener in listeners {
        let server = server.clone();
        let shutdown_flag = shutdown_flag.clone();

        let task_handle = tokio::spawn(async move {
            loop {
                if shutdown_flag.load(Ordering::Relaxed) {
                    break;
                }

                let accept_result = listener.accept().await;
                let (stream, addr) = match accept_result {
                    Ok((s, a)) => (s, a),
                    Err(e) => {
                        if shutdown_flag.load(Ordering::Relaxed) {
                            break;
                        }
                        tracing::warn!(error = %e, "Accept error");
                        continue;
                    }
                };

                let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
                let local_addr = stream.local_addr().ok();

                tracing::trace!(
                    conn_id,
                    peer_addr = ?addr,
                    ?local_addr,
                    "Accepted browser connection"
                );

                let server = server.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_browser_connection(conn_id, stream, server).await {
                        tracing::warn!(
                            conn_id,
                            error = ?e,
                            "Failed to handle browser connection"
                        );
                    }
                });
            }
        });
        accept_handles.push(task_handle);
    }

    // Wait for shutdown signal
    if let Some(mut rx) = shutdown_rx {
        loop {
            rx.changed().await.ok();
            if *rx.borrow() {
                tracing::info!("Shutdown signal received, stopping HTTP server");
                break;
            }
        }
    } else {
        std::future::pending::<()>().await;
    }

    shutdown_flag.store(true, Ordering::Relaxed);

    for handle in accept_handles {
        handle.abort();
    }

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

/// Start a cell server with a static content service (for `ddc serve --static` mode)
///
/// This is used when serving pre-built static files without the full dodeca system.
pub async fn start_static_cell_server<C>(
    _content_service: Arc<C>,
    _cell_path: std::path::PathBuf,
    _bind_ips: Vec<std::net::Ipv4Addr>,
    _port: u16,
    _port_tx: Option<tokio::sync::oneshot::Sender<u16>>,
) -> Result<()>
where
    C: cell_http_proto::ContentService + Send + Sync + 'static,
{
    // TODO: Implement static content serving with roam
    // For now, return an error indicating this is not yet implemented
    Err(eyre::eyre!(
        "Static content serving not yet implemented for roam migration"
    ))
}

/// HTTP 500 response for fatal boot errors
const FATAL_ERROR_RESPONSE: &[u8] = b"HTTP/1.1 500 Internal Server Error\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Connection: close\r\n\
Content-Length: 52\r\n\
\r\n\
Server failed to start. Check server logs for details";

/// Handle a browser TCP connection by tunneling it through the cell
async fn handle_browser_connection(
    conn_id: u64,
    mut browser_stream: TcpStream,
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

    // Get HTTP cell client (spawns lazily on first access)
    let tunnel_client = match Host::get().client_async::<TcpTunnelClient>().await {
        Some(client) => client,
        None => {
            tracing::error!(conn_id, "Failed to get HTTP cell client");
            if let Err(e) = browser_stream.write_all(FATAL_ERROR_RESPONSE).await {
                tracing::warn!(conn_id, error = %e, "Failed to write 500 response");
            }
            return Ok(());
        }
    };

    // Wait for revision readiness (site content built)
    tracing::trace!(conn_id, "Waiting for revision readiness (per-connection)");
    let revision_start = Instant::now();
    server.wait_revision_ready().await;
    tracing::trace!(
        conn_id,
        elapsed_ms = revision_start.elapsed().as_millis(),
        "Revision ready (per-connection)"
    );

    // Create a tunnel pair - local stays here, remote goes to cell
    let (local, remote) = tunnel_pair();
    let channel_id = local.tx.channel_id();

    // Open a tunnel to the cell by passing the remote end
    let open_started = Instant::now();
    tunnel_client
        .open(remote)
        .await
        .map_err(|e| eyre::eyre!("Failed to open tunnel: {:?}", e))?;

    tracing::trace!(
        conn_id,
        channel_id,
        open_elapsed_ms = open_started.elapsed().as_millis(),
        "Tunnel opened for browser connection"
    );

    // Bridge browser <-> tunnel
    // Create a duplex to bridge the tunnel
    let (client, server_stream) = tokio::io::duplex(64 * 1024);
    let (_read_handle, _write_handle) =
        roam::tunnel_stream(client, local, roam::DEFAULT_TUNNEL_CHUNK_SIZE);

    tracing::trace!(
        conn_id,
        channel_id,
        "Starting browser <-> tunnel bridge task"
    );

    // Use the server side of the duplex for the browser connection
    let mut tunnel_stream = server_stream;
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

// ============================================================================
// Browser Virtual Connection Handling
// ============================================================================

/// Accept incoming virtual connections from browsers through cell-http.
///
/// Each browser that connects via WebSocket to /_/ws opens a virtual connection
/// through cell-http to the host. This function accepts those connections and
/// registers them with the SiteServer for receiving devtools events.
pub async fn accept_browser_connections(
    mut incoming: IncomingConnections,
    server: Arc<SiteServer>,
) {
    tracing::info!("Starting browser virtual connection acceptor");

    while let Some(conn) = incoming.recv().await {
        tracing::debug!("Received incoming virtual connection from browser");

        // Accept the connection
        let handle = match conn.accept(roam::wire::Metadata::default()).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = ?e, "Failed to accept browser virtual connection");
                continue;
            }
        };

        tracing::info!("Accepted browser virtual connection");

        // Create a BrowserServiceClient to call the browser
        let browser_client = BrowserServiceClient::new(handle);

        // Register this browser with the server
        server.register_browser(browser_client);
    }

    tracing::info!("Browser virtual connection acceptor finished");
}
