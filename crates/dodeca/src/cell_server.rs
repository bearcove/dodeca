//! HTTP server for dodeca browser traffic.
//!
//! This module handles:
//! - Serving browser TCP connections through the former `cell-http` axum router
//! - Handling browser DevTools Vox sessions over WebSocket
//! - Binding listeners and coordinating graceful shutdown

use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use eyre::Result;
use tokio::net::TcpStream;
use tokio::sync::watch;
use vox::FromVoxSession;

use cell_http_proto::{ContentService, ServeContent};
use futures_util::future::BoxFuture;

use crate::boot_state::BootStateManager;
use crate::serve::SiteServer;

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct DodecaHttpContext {
    server: Arc<SiteServer>,
}

impl DodecaHttpContext {
    fn new(server: Arc<SiteServer>) -> Self {
        Self { server }
    }
}

impl ddc_cell_http::RouterContext for DodecaHttpContext {
    fn find_content(
        &self,
        path: String,
        identity: Option<cell_http_proto::Identity>,
    ) -> BoxFuture<'_, ServeContent> {
        Box::pin(async move {
            let content_service =
                crate::content_service::HostContentService::new(self.server.clone());
            content_service.find_content(path, identity).await
        })
    }

    fn get_vite_port(&self) -> BoxFuture<'_, Option<u16>> {
        Box::pin(async move { crate::host::Host::get().get_vite_port() })
    }

    fn accept_devtools_connection(
        &self,
        service: &str,
        connection: vox::PendingConnection,
    ) -> std::result::Result<(), vox::Metadata<'static>> {
        match service {
            s if s == vox::NoopClient::SERVICE_NAME => {
                tracing::debug!("devtools browser root connection accepted");
                connection.handle_with(());
                Ok(())
            }
            s if s == dodeca_protocol::DevtoolsServiceClient::SERVICE_NAME => {
                let browser_id = next_devtools_browser_id();
                let svc = HostDevtoolsService::new(self.server.clone(), browser_id);
                tracing::debug!(
                    browser_id,
                    "devtools browser service connection accepted directly"
                );
                let browser: dodeca_protocol::BrowserServiceClient = connection
                    .handle_with_client(dodeca_protocol::DevtoolsServiceDispatcher::new(svc));
                self.server.register_browser(browser_id, browser.clone());
                crate::spawn::spawn({
                    let server = self.server.clone();
                    async move {
                        browser.caller.closed().await;
                        server.unregister_browser(browser_id);
                    }
                });
                Ok(())
            }
            other => Err(vec![vox::MetadataEntry::str(
                "error",
                format!("unsupported browser devtools service {other}"),
            )]),
        }
    }
}

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
// HostDevtoolsService - vox RPC implementation of DevtoolsService
// ============================================================================

use dodeca_protocol::{
    DeadLinkTarget, DevtoolsEvent, DevtoolsService, EvalResult, OpenSourceResult, ScopeEntry,
};

/// Host-side implementation of DevtoolsService for direct vox RPC.
///
/// This implements the `DevtoolsService` trait from `dodeca-protocol`,
/// allowing browser devtools to call methods over the WebSocket-backed
/// vox connection proxied through cell-http.
#[derive(Clone)]
pub struct HostDevtoolsService {
    server: Arc<SiteServer>,
    browser_id: u64,
    route: Arc<RwLock<String>>,
}

impl HostDevtoolsService {
    pub fn new(server: Arc<SiteServer>, browser_id: u64) -> Self {
        Self {
            server,
            browser_id,
            route: Arc::new(RwLock::new("/".to_string())),
        }
    }
}

/// Returns a short summary of a DevtoolsEvent for logging
pub fn event_summary(event: &DevtoolsEvent) -> String {
    match event {
        DevtoolsEvent::Reload => "Reload".to_string(),
        DevtoolsEvent::CssChanged { path } => format!("CssChanged({})", path),
        DevtoolsEvent::Patches { route, patches } => {
            format!("Patches(route={}, count={})", route, patches.len())
        }
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

pub fn next_devtools_browser_id() -> u64 {
    NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed)
}

impl DevtoolsService for HostDevtoolsService {
    /// Subscribe to devtools events for a route.
    ///
    /// This registers the browser's interest in a route. Events will be pushed
    /// via BrowserService::on_event() on the browser's virtual connection.
    ///
    /// The browser was already registered when its virtual connection was accepted.
    /// This method associates the subscription with the host-allocated browser id.
    async fn subscribe(&self, route: String) {
        tracing::info!(browser_id = self.browser_id, route = %route, "devtools: client subscribing to route");
        if let Ok(mut current) = self.route.write() {
            *current = route.clone();
        }
        self.server.set_browser_route(self.browser_id, route);
    }

    /// Get scope entries for the current route.
    async fn get_scope(&self, path: Option<Vec<String>>) -> Vec<ScopeEntry> {
        let route = self
            .route
            .read()
            .map(|route| route.clone())
            .unwrap_or_else(|_| "/".to_string());
        let path = path.unwrap_or_default();
        self.server.get_scope_for_route(&route, &path).await
    }

    /// Evaluate an expression in a snapshot's context.
    async fn eval(&self, snapshot_id: String, expression: String) -> EvalResult {
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
    async fn dismiss_error(&self, route: String) {
        tracing::debug!(route = %route, "Client dismissed error via RPC");
        // The existing implementation just logs this - errors are resolved
        // when the template successfully re-renders
    }

    async fn open_source(&self, source_file: String, line: u32) -> OpenSourceResult {
        tracing::debug!(
            browser_id = self.browser_id,
            source_file = %source_file,
            line,
            "devtools open_source RPC received"
        );

        let result = match self.server.open_source_in_editor(&source_file, line).await {
            Ok(()) => OpenSourceResult::Ok,
            Err(err) => OpenSourceResult::Err(err.to_string()),
        };

        match &result {
            OpenSourceResult::Ok => tracing::debug!(
                browser_id = self.browser_id,
                source_file = %source_file,
                line,
                "devtools open_source RPC succeeded"
            ),
            OpenSourceResult::Err(err) => tracing::debug!(
                browser_id = self.browser_id,
                source_file = %source_file,
                line,
                error = %err,
                "devtools open_source RPC failed"
            ),
        }

        result
    }

    async fn open_source_id(&self, route: String, sid: String) -> OpenSourceResult {
        tracing::debug!(
            browser_id = self.browser_id,
            route = %route,
            sid = %sid,
            "devtools open_source_id RPC received"
        );

        let result = match self.server.open_source_id_in_editor(&route, &sid).await {
            Ok(()) => OpenSourceResult::Ok,
            Err(err) => OpenSourceResult::Err(err.to_string()),
        };

        match &result {
            OpenSourceResult::Ok => tracing::debug!(
                browser_id = self.browser_id,
                route = %route,
                sid = %sid,
                "devtools open_source_id RPC succeeded"
            ),
            OpenSourceResult::Err(err) => tracing::debug!(
                browser_id = self.browser_id,
                route = %route,
                sid = %sid,
                error = %err,
                "devtools open_source_id RPC failed"
            ),
        }

        result
    }

    async fn open_dead_link(&self, route: String, target: DeadLinkTarget) -> OpenSourceResult {
        tracing::debug!(
            browser_id = self.browser_id,
            route = %route,
            target = ?target,
            "devtools open_dead_link RPC received"
        );

        let result = match self
            .server
            .open_dead_link_in_editor(&route, target.clone())
            .await
        {
            Ok(()) => OpenSourceResult::Ok,
            Err(err) => OpenSourceResult::Err(err.to_string()),
        };

        match &result {
            OpenSourceResult::Ok => tracing::debug!(
                browser_id = self.browser_id,
                route = %route,
                target = ?target,
                "devtools open_dead_link RPC succeeded"
            ),
            OpenSourceResult::Err(err) => tracing::debug!(
                browser_id = self.browser_id,
                route = %route,
                target = ?target,
                error = %err,
                "devtools open_dead_link RPC failed"
            ),
        }

        result
    }

    async fn edit_load(&self, token: String, route: String) -> dodeca_protocol::EditLoad {
        tracing::debug!(browser_id = self.browser_id, route = %route, "devtools edit_load");
        self.server.edit_load(&token, &route).await
    }

    async fn edit_preview(
        &self,
        token: String,
        source_key: String,
        buffer: String,
    ) -> dodeca_protocol::EditPreview {
        tracing::debug!(browser_id = self.browser_id, source_key = %source_key, "devtools edit_preview");
        self.server.edit_preview(&token, &source_key, &buffer).await
    }

    async fn edit_save(
        &self,
        token: String,
        req: dodeca_protocol::EditSaveReq,
    ) -> dodeca_protocol::EditSave {
        tracing::debug!(browser_id = self.browser_id, source_key = %req.source_key, "devtools edit_save");
        self.server
            .edit_save(
                &token,
                &req.source_key,
                &req.buffer,
                &req.base,
                &req.message,
            )
            .await
    }

    async fn edit_upload(
        &self,
        token: String,
        req: dodeca_protocol::EditUploadReq,
    ) -> dodeca_protocol::EditUpload {
        tracing::debug!(browser_id = self.browser_id, source_key = %req.source_key, bytes = req.bytes.len(), "devtools edit_upload");
        self.server
            .edit_upload(&token, &req.source_key, &req.filename, &req.bytes)
            .await
    }

    async fn lsp(
        &self,
        token: String,
        client_to_server: vox::Rx<String>,
        server_to_client: vox::Tx<String>,
    ) {
        tracing::debug!(browser_id = self.browser_id, "devtools lsp session opening");
        // Return immediately: the vox request must succeed now, or its default
        // 30s timeout fires and tears the channels down with it. The session
        // runs on the channels, which outlive the request.
        let server = self.server.clone();
        tokio::spawn(async move {
            server
                .run_lsp_session(&token, client_to_server, server_to_client)
                .await;
        });
    }

    async fn edit_read(&self, token: String, uri: String) -> dodeca_protocol::EditRead {
        self.server.edit_read(&token, &uri).await
    }

    async fn edit_list(&self, token: String) -> dodeca_protocol::EditList {
        self.server.edit_list(&token).await
    }
}

/// Start the HTTP server with optional shutdown signal
///
/// This:
/// 1. Builds the HTTP router with direct access to the site server
/// 2. Listens for browser TCP connections
/// 3. Serves each connection through hyper/axum
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
    // Preserve the old host service path while HTTP/TUI compatibility remains.
    crate::cells::provide_site_server(server.clone());
    let http_app = ddc_cell_http::build_router(Arc::new(DodecaHttpContext::new(server.clone())));

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
        crate::spawn::spawn(async move {
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
    let accept_task = crate::spawn::spawn(async move {
        run_async_accept_loop(
            tokio_listeners,
            accept_server,
            http_app,
            shutdown_rx,
            shutdown_flag,
        )
        .await
    });

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
    http_app: axum::Router,
    shutdown_rx: Option<watch::Receiver<bool>>,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
    tracing::debug!(num_listeners = listeners.len(), "Accept loop starting");

    // Spawn accept tasks for each listener
    let mut accept_handles = Vec::new();
    for listener in listeners {
        let server = server.clone();
        let http_app = http_app.clone();
        let shutdown_flag = shutdown_flag.clone();

        let task_handle = crate::spawn::spawn(async move {
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
                let http_app = http_app.clone();
                crate::spawn::spawn(async move {
                    if let Err(e) =
                        handle_browser_connection(conn_id, stream, server, http_app).await
                    {
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

/// Handle a browser TCP connection directly through the HTTP router.
async fn handle_browser_connection(
    conn_id: u64,
    browser_stream: TcpStream,
    server: Arc<SiteServer>,
    http_app: axum::Router,
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

    // Wait for revision readiness (site content built)
    tracing::trace!(conn_id, "Waiting for revision readiness (per-connection)");
    let revision_start = Instant::now();
    server.wait_revision_ready().await;
    tracing::trace!(
        conn_id,
        elapsed_ms = revision_start.elapsed().as_millis(),
        "Revision ready (per-connection)"
    );

    hyper::server::conn::http1::Builder::new()
        .serve_connection(
            hyper_util::rt::TokioIo::new(browser_stream),
            hyper_util::service::TowerToHyperService::new(http_app),
        )
        .with_upgrades()
        .await
        .map_err(|e| eyre::eyre!("HTTP connection error: {}", e))?;

    tracing::trace!(
        conn_id,
        elapsed_ms = started_at.elapsed().as_millis(),
        "handle_browser_connection: end"
    );
    Ok(())
}

// Browser virtual connections are accepted by the HTTP router's WebSocket
// handler and served directly by `HostDevtoolsService`.
