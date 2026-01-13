//! Cell loading and management for dodeca.
//!
//! Cells are separate processes that handle specialized tasks (image processing,
//! markdown rendering, etc.). They communicate with the host via roam RPC over
//! shared memory.
//!
//! # Hub Architecture
//!
//! All cells share a single SHM segment. Each cell gets its own ring pair
//! within the segment and communicates via socketpair doorbells.
//!
//! The host uses `MultiPeerHostDriver` to manage all cell connections.

use cell_code_execution_proto::{
    CodeExecutionResult, CodeExecutorClient, ExecuteSamplesInput, ExtractSamplesInput,
};
use cell_css_proto::{CssProcessorClient, CssResult};
use cell_dialoguer_proto::DialoguerClient;
use cell_fonts_proto::{FontAnalysis, FontProcessorClient, FontResult, SubsetFontInput};
use cell_gingembre_proto::{ContextId, EvalResult, RenderResult, TemplateRendererClient};
use cell_host_proto::{
    CallFunctionResult, CommandResult, HostService, HostServiceDispatcher, KeysAtResult,
    LoadTemplateResult, ReadyAck, ReadyMsg, ResolveDataResult, ServeContent, ServerCommand, Value,
};
use cell_html_diff_proto::{DiffInput, HtmlDiffResult, HtmlDifferClient};
use cell_html_proto::HtmlProcessorClient;
use cell_http_proto::{ScopeEntry, TcpTunnelClient};
use cell_image_proto::{ImageProcessorClient, ImageResult, ResizeInput, ThumbhashInput};
use cell_js_proto::{JsProcessorClient, JsResult, JsRewriteInput};
use cell_jxl_proto::{JXLEncodeInput, JXLProcessorClient, JXLResult};
use cell_lifecycle_proto::CellLifecycle;
use cell_linkcheck_proto::{LinkCheckInput, LinkCheckResult, LinkCheckerClient, LinkStatus};
use cell_markdown_proto::{
    FrontmatterResult, MarkdownProcessorClient, MarkdownResult, ParseResult,
};
use cell_minify_proto::{MinifierClient, MinifyResult};
use cell_pagefind_proto::{SearchIndexInput, SearchIndexResult, SearchIndexerClient};
use cell_sass_proto::{SassCompilerClient, SassInput, SassResult};
use cell_svgo_proto::{SvgoOptimizerClient, SvgoResult};
use cell_tui_proto::TuiDisplayClient;
use cell_webp_proto::{WebPEncodeInput, WebPProcessorClient, WebPResult};
use dashmap::DashMap;
use facet::Facet;
use roam::Tunnel;
use roam::session::{ConnectionHandle, ServiceDispatcher};
use roam_shm::driver::MultiPeerHostDriver;
use roam_shm::{AddPeerOptions, PeerId, SegmentConfig, ShmHost};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::serve::SiteServer;
use crate::template_host::TemplateHostImpl;

// ============================================================================
// Global State
// ============================================================================

/// Global SHM host (shared by all cells).
static SHM_HOST: OnceLock<ShmHost> = OnceLock::new();

/// SHM segment path.
static SHM_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Global connection handles for all cells (peer_id -> handle).
static CELL_HANDLES: OnceLock<Arc<std::sync::RwLock<HashMap<PeerId, ConnectionHandle>>>> =
    OnceLock::new();

/// Mapping from cell name to peer ID.
static CELL_NAME_TO_PEER_ID: OnceLock<Arc<std::sync::RwLock<HashMap<String, PeerId>>>> =
    OnceLock::new();

/// Whether cells should suppress startup messages (set when TUI is active).
static QUIET_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// SiteServer for HTTP cell initialization.
/// Must be set via `provide_site_server()` before calling `all()` if HTTP serving is needed.
static SITE_SERVER_FOR_INIT: OnceLock<Arc<SiteServer>> = OnceLock::new();

/// Provide the SiteServer for HTTP cell initialization.
/// This must be called before `all()` when the HTTP cell needs to serve content.
/// For build-only commands, this can be skipped.
pub fn provide_site_server(server: Arc<SiteServer>) {
    let _ = SITE_SERVER_FOR_INIT.set(server);
}

// Note: TUI command forwarding now goes through Host::get().handle_tui_command()
// The old TUI_HOST_FOR_INIT global has been removed.
// Exit signaling now goes through Host::get().signal_exit() / wait_for_exit().

/// Enable quiet mode for spawned cells (call this when TUI is active).
pub fn set_quiet_mode(quiet: bool) {
    QUIET_MODE.store(quiet, std::sync::atomic::Ordering::SeqCst);
}

/// Check if quiet mode is enabled.
fn is_quiet_mode() -> bool {
    QUIET_MODE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Get a cell's connection handle by peer ID.
pub fn get_cell_handle(peer_id: PeerId) -> Option<ConnectionHandle> {
    CELL_HANDLES.get()?.read().ok()?.get(&peer_id).cloned()
}

/// Get a cell's connection handle by name.
pub fn get_cell_handle_by_name(name: &str) -> Option<ConnectionHandle> {
    let peer_id = CELL_NAME_TO_PEER_ID
        .get()?
        .read()
        .ok()?
        .get(name)
        .copied()?;
    get_cell_handle(peer_id)
}

// ============================================================================
// Stdio Capture
// ============================================================================

fn spawn_stdio_pump<R>(label: String, stream: &'static str, reader: R)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    debug!(cell = %label, %stream, "cell stdio EOF");
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
                    info!(cell = %label, %stream, "{trimmed}");
                }
                Err(e) => {
                    warn!(cell = %label, %stream, error = ?e, "cell stdio read failed");
                    break;
                }
            }
        }
    });
}

fn capture_cell_stdio(label: &str, child: &mut tokio::process::Child) {
    if let Some(stdout) = child.stdout.take() {
        spawn_stdio_pump(label.to_string(), "stdout", stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_stdio_pump(label.to_string(), "stderr", stderr);
    }
}

// ============================================================================
// Cell Readiness Registry
// ============================================================================

/// Registry for tracking cell readiness (RPC-ready state).
#[derive(Clone)]
pub struct CellReadyRegistry {
    ready: Arc<DashMap<u16, ReadyMsg>>,
}

impl CellReadyRegistry {
    fn new() -> Self {
        Self {
            ready: Arc::new(DashMap::new()),
        }
    }

    fn mark_ready(&self, msg: ReadyMsg) {
        let peer_id = msg.peer_id;
        self.ready.insert(peer_id, msg);
    }

    pub fn is_ready(&self, peer_id: u16) -> bool {
        self.ready.contains_key(&peer_id)
    }

    pub async fn wait_for_all_ready(
        &self,
        peer_ids: &[u16],
        timeout: Duration,
    ) -> eyre::Result<()> {
        let start = std::time::Instant::now();
        for &peer_id in peer_ids {
            loop {
                if self.is_ready(peer_id) {
                    break;
                }
                if start.elapsed() >= timeout {
                    let cell_name = PEER_DIAG_INFO
                        .read()
                        .ok()
                        .and_then(|info| {
                            info.iter()
                                .find(|p| p.peer_id == peer_id)
                                .map(|p| p.name.clone())
                        })
                        .unwrap_or_else(|| format!("peer_{}", peer_id));
                    return Err(eyre::eyre!(
                        "Timeout waiting for {} (peer {}) to be ready",
                        cell_name,
                        peer_id
                    ));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        Ok(())
    }
}

static CELL_READY_REGISTRY: OnceLock<CellReadyRegistry> = OnceLock::new();

pub fn cell_ready_registry() -> &'static CellReadyRegistry {
    CELL_READY_REGISTRY.get_or_init(CellReadyRegistry::new)
}

pub async fn wait_for_cells_ready(cell_names: &[&str], timeout: Duration) -> eyre::Result<()> {
    let mut peer_ids = Vec::new();
    if let Ok(info) = PEER_DIAG_INFO.read() {
        for &cell_name in cell_names {
            let peer_id = info
                .iter()
                .find(|i| i.name == cell_name)
                .map(|i| i.peer_id)
                .ok_or_else(|| eyre::eyre!("Cell {} not found", cell_name))?;
            peer_ids.push(peer_id);
        }
    } else {
        return Err(eyre::eyre!("Failed to acquire peer info lock"));
    }
    cell_ready_registry()
        .wait_for_all_ready(&peer_ids, timeout)
        .await
}

/// Host implementation of CellLifecycle service
#[derive(Clone)]
pub struct HostCellLifecycle {
    registry: CellReadyRegistry,
}

impl HostCellLifecycle {
    pub fn new(registry: CellReadyRegistry) -> Self {
        Self { registry }
    }
}

impl CellLifecycle for HostCellLifecycle {
    async fn ready(&self, msg: ReadyMsg) -> ReadyAck {
        let peer_id = msg.peer_id;
        let cell_name = msg.cell_name.clone();
        debug!("Cell {} (peer_id={}) is ready", cell_name, peer_id);
        self.registry.mark_ready(msg);

        let host_time_unix_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64);

        ReadyAck {
            ok: true,
            host_time_unix_ms,
        }
    }
}

// ============================================================================
// Unified Host Service Implementation
// ============================================================================

/// Unified host service that all cells connect to.
///
/// This implements `HostService` by delegating to the specialized implementations.
/// TUI command forwarding goes through `Host::get()`.
#[derive(Clone)]
pub struct HostServiceImpl {
    lifecycle: HostCellLifecycle,
    template_host: crate::template_host::TemplateHostImpl,
    site_server: Option<Arc<SiteServer>>,
}

impl HostServiceImpl {
    pub fn new(
        lifecycle: HostCellLifecycle,
        template_host: crate::template_host::TemplateHostImpl,
        site_server: Option<Arc<SiteServer>>,
    ) -> Self {
        Self {
            lifecycle,
            template_host,
            site_server,
        }
    }
}

impl HostService for HostServiceImpl {
    // Cell Lifecycle
    async fn ready(&self, msg: ReadyMsg) -> ReadyAck {
        let peer_id = msg.peer_id;
        let cell_name = msg.cell_name.clone();
        debug!("Cell {} (peer_id={}) is ready", cell_name, peer_id);
        self.lifecycle.registry.mark_ready(msg);

        let host_time_unix_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64);

        ReadyAck {
            ok: true,
            host_time_unix_ms,
        }
    }

    // Template Host
    async fn load_template(&self, context_id: ContextId, name: String) -> LoadTemplateResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host.load_template(context_id, name).await
    }

    async fn resolve_data(&self, context_id: ContextId, path: Vec<String>) -> ResolveDataResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host.resolve_data(context_id, path).await
    }

    async fn keys_at(&self, context_id: ContextId, path: Vec<String>) -> KeysAtResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host.keys_at(context_id, path).await
    }

    async fn call_function(
        &self,
        context_id: ContextId,
        name: String,
        args: Vec<Value>,
        kwargs: Vec<(String, Value)>,
    ) -> CallFunctionResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host
            .call_function(context_id, name, args, kwargs)
            .await
    }

    // Content Service
    async fn find_content(&self, path: String) -> ServeContent {
        if let Some(server) = &self.site_server {
            use cell_http_proto::ContentService;
            let content_service = crate::content_service::HostContentService::new(server.clone());
            content_service.find_content(path).await
        } else {
            ServeContent::NotFound {
                html: "Not in serve mode".to_string(),
                generation: 0,
            }
        }
    }

    async fn get_scope(&self, route: String, path: Vec<String>) -> Vec<ScopeEntry> {
        if let Some(server) = &self.site_server {
            use cell_http_proto::ContentService;
            let content_service = crate::content_service::HostContentService::new(server.clone());
            content_service.get_scope(route, path).await
        } else {
            vec![]
        }
    }

    async fn eval_expression(
        &self,
        route: String,
        expression: String,
    ) -> cell_host_proto::EvalResult {
        if let Some(server) = &self.site_server {
            use cell_http_proto::ContentService;
            let content_service = crate::content_service::HostContentService::new(server.clone());
            content_service.eval_expression(route, expression).await
        } else {
            cell_host_proto::EvalResult::Err("Not in serve mode".to_string())
        }
    }

    // WebSocket Tunnel
    async fn open_websocket(&self, tunnel: Tunnel) {
        if let Some(server) = &self.site_server {
            use cell_http_proto::WebSocketTunnel;
            let ws_tunnel = crate::cell_server::HostWebSocketTunnel::new(server.clone());
            ws_tunnel.open(tunnel).await
        }
        // Not in serve mode - drop the tunnel
    }

    // TUI Commands (TUI → Host)
    async fn send_command(&self, command: ServerCommand) -> CommandResult {
        // Forward to Host singleton
        crate::host::Host::get().handle_tui_command(command)
    }
}

// ============================================================================
// Peer Diagnostics
// ============================================================================

struct PeerDiagInfo {
    peer_id: u16,
    name: String,
    handle: ConnectionHandle,
}

static PEER_DIAG_INFO: RwLock<Vec<PeerDiagInfo>> = RwLock::new(Vec::new());

fn register_peer_diag(peer_id: u16, name: &str, handle: ConnectionHandle) {
    if let Ok(mut info) = PEER_DIAG_INFO.write() {
        info.push(PeerDiagInfo {
            peer_id,
            name: name.to_string(),
            handle,
        });
    }
}

/// Get a cell's connection handle by binary name (e.g., "ddc-cell-http").
pub fn get_cell_session(name: &str) -> Option<ConnectionHandle> {
    PEER_DIAG_INFO
        .read()
        .ok()?
        .iter()
        .find(|info| info.name == name)
        .map(|info| info.handle.clone())
}

/// Get the TUI display client for pushing updates to the TUI cell.
pub fn get_tui_display_client() -> Option<TuiDisplayClient> {
    crate::host::Host::get().client::<TuiDisplayClient>()
}

// ============================================================================
// Decoded Image Type (re-export)
// ============================================================================

pub type DecodedImage = cell_image_proto::DecodedImage;

// ============================================================================
// Cell Registry
// ============================================================================

static CELLS: tokio::sync::OnceCell<CellRegistry> = tokio::sync::OnceCell::const_new();

pub async fn init_and_wait_for_cells() -> eyre::Result<()> {
    // Trigger cell loading
    let _ = all().await;

    // Initialize gingembre cell (special case - has TemplateHost service)
    init_gingembre_cell().await;

    // Get all spawned peer IDs
    let peer_ids: Vec<u16> = PEER_DIAG_INFO
        .read()
        .map_err(|_| eyre::eyre!("Failed to acquire peer info lock"))?
        .iter()
        .map(|info| info.peer_id)
        .collect();

    if peer_ids.is_empty() {
        debug!("No cells loaded, skipping readiness wait");
        return Ok(());
    }

    // Wait for all cells to complete readiness handshake
    let timeout = Duration::from_secs(10);
    cell_ready_registry()
        .wait_for_all_ready(&peer_ids, timeout)
        .await?;

    // Push tracing config to cells
    push_tracing_config_to_cells().await;

    debug!("All {} cells ready", peer_ids.len());
    Ok(())
}

async fn push_tracing_config_to_cells() {
    let filter_str = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| crate::logging::DEFAULT_TRACING_FILTER.to_string());

    let Ok(peers) = PEER_DIAG_INFO.read() else {
        warn!("Failed to acquire peer info lock; skipping tracing config push");
        return;
    };

    for peer in peers.iter() {
        let handle = peer.handle.clone();
        let cell_label = peer.name.clone();
        let filter_str = filter_str.clone();
        tokio::spawn(async move {
            use roam_tracing::{CellTracingClient, Level, TracingConfig};
            let client = CellTracingClient::new(handle);
            let config = TracingConfig {
                min_level: Level::Debug,
                filters: vec![filter_str.clone()],
                include_span_events: false,
            };
            match client.configure(config).await {
                Ok(roam_tracing::ConfigResult::Ok) => {
                    debug!("Pushed tracing config to {} cell", cell_label);
                }
                Ok(roam_tracing::ConfigResult::InvalidFilter(msg)) => {
                    warn!("Invalid tracing filter for {} cell: {}", cell_label, msg);
                }
                Err(e) => {
                    warn!(
                        "Failed to push tracing config to {} cell: {:?}",
                        cell_label, e
                    );
                }
            }
        });
    }
}

// ============================================================================
// Cell Spawning
// ============================================================================

pub struct CellSpawnConfig {
    pub inherit_stdio: bool,
    pub manage_child: bool,
}

impl Default for CellSpawnConfig {
    fn default() -> Self {
        Self {
            inherit_stdio: false,
            manage_child: true,
        }
    }
}

pub struct SpawnedCellResult {
    pub peer_id: u16,
    pub handle: ConnectionHandle,
    pub child: Option<tokio::process::Child>,
}

/// Find a cell binary by name
pub fn find_cell_binary(name: &str) -> Option<PathBuf> {
    // Look next to the current executable
    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;
    let binary_path = exe_dir.join(name);
    if binary_path.exists() {
        return Some(binary_path);
    }

    // Try target/debug or target/release
    let target_dir = exe_dir.parent()?;
    for profile in ["debug", "release"] {
        let path = target_dir.join(profile).join(name);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

// ============================================================================
// Template Host Implementation (for gingembre cell)
// ============================================================================

// ============================================================================
// Gingembre Cell
// ============================================================================

static GINGEMBRE_CELL: tokio::sync::OnceCell<Arc<TemplateRendererClient>> =
    tokio::sync::OnceCell::const_new();

pub async fn init_gingembre_cell() -> Option<()> {
    let handle = get_cell_handle_by_name("gingembre")?;
    let client = Arc::new(TemplateRendererClient::new(handle));
    GINGEMBRE_CELL.set(client).ok()?;
    debug!("Gingembre cell initialized");
    Some(())
}

pub async fn gingembre_cell() -> Option<Arc<TemplateRendererClient>> {
    GINGEMBRE_CELL.get().cloned()
}

pub async fn render_template(
    context_id: ContextId,
    template_name: &str,
    initial_context: Value,
) -> eyre::Result<RenderResult> {
    let cell = gingembre_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Gingembre cell not initialized"))?;
    let result = cell
        .render(context_id, template_name.to_string(), initial_context)
        .await
        .map_err(|e| eyre::eyre!("RPC call error: {:?}", e))?;
    Ok(result)
}

pub async fn eval_template_expression(
    context_id: ContextId,
    expression: &str,
    context: Value,
) -> eyre::Result<EvalResult> {
    let cell = gingembre_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Gingembre cell not initialized"))?;
    let result = cell
        .eval_expression(context_id, expression.to_string(), context)
        .await
        .map_err(|e| eyre::eyre!("RPC call error: {:?}", e))?;
    Ok(result)
}

// ============================================================================
// Cell Registry Implementation
// ============================================================================

/// Configuration for a cell's spawn behavior.
struct CellDef {
    /// Binary suffix (e.g., "image" -> "ddc-cell-image")
    suffix: &'static str,
    /// If true, cell inherits stdio for direct terminal access
    inherit_stdio: bool,
}

impl CellDef {
    const fn new(suffix: &'static str) -> Self {
        Self {
            suffix,
            inherit_stdio: false,
        }
    }

    const fn inherit_stdio(mut self) -> Self {
        self.inherit_stdio = true;
        self
    }
}

/// Cell definitions with their spawn configuration.
const CELL_DEFS: &[CellDef] = &[
    CellDef::new("image"),
    CellDef::new("webp"),
    CellDef::new("jxl"),
    CellDef::new("markdown"),
    CellDef::new("html"),
    CellDef::new("minify"),
    CellDef::new("css"),
    CellDef::new("sass"),
    CellDef::new("js"),
    CellDef::new("svgo"),
    CellDef::new("fonts"),
    CellDef::new("linkcheck"),
    CellDef::new("html-diff"),
    CellDef::new("dialoguer"),
    CellDef::new("pagefind"),
    CellDef::new("code-execution"),
    CellDef::new("http"),
    CellDef::new("gingembre"),
    // TUI needs terminal access
    CellDef::new("tui").inherit_stdio(),
];

/// Cell registry providing typed client accessors.
pub struct CellRegistry {
    _phantom: std::marker::PhantomData<()>,
}

impl CellRegistry {
    fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

/// Initialize the cell infrastructure.
///
/// This function:
/// 1. Creates the SHM host with a temp path
/// 2. Spawns all cell processes
/// 3. Sets up the MultiPeerHostDriver
/// 4. Stores connection handles for later use
async fn init_cells() -> CellRegistry {
    match init_cells_inner().await {
        Ok(()) => {
            info!("Cell infrastructure initialized successfully");
        }
        Err(e) => {
            warn!("Failed to initialize cell infrastructure: {}", e);
        }
    }
    CellRegistry::new()
}

async fn init_cells_inner() -> eyre::Result<()> {
    // Create temp SHM path
    let shm_path = std::env::temp_dir().join(format!("dodeca-shm-{}", std::process::id()));

    // Configure segment for multi-cell architecture
    let max_payload = 16 * 1024 * 1024; // 16MB for large payloads
    let config = SegmentConfig {
        max_guests: 32,
        ring_size: 256,
        slots_per_guest: 64,
        slot_size: max_payload + 8, // slot_size must be >= max_payload_size + 4, and multiple of 8
        max_payload_size: max_payload,
        ..SegmentConfig::default()
    };

    // Create SHM host
    let mut host = ShmHost::create(&shm_path, config)?;
    let hub_path = shm_path.clone();

    // Store hub path globally
    let _ = SHM_PATH.set(hub_path.clone());

    // Find cell binary directory
    let cell_dir = find_cell_directory()?;

    // Spawn all cells and collect (peer_id, binary_name, cell_name, child) mappings
    let mut peer_cells: Vec<(PeerId, String, String, tokio::process::Child)> = Vec::new();

    // Check if TUI mode is enabled
    let tui_enabled = crate::host::Host::get().is_tui_mode();

    for cell_def in CELL_DEFS {
        let binary_name = format!("ddc-cell-{}", cell_def.suffix);
        let cell_name = cell_def.suffix.replace('-', "_");
        let binary_path = cell_dir.join(&binary_name);

        // Skip TUI cell if not in TUI mode
        if cell_name == "tui" && !tui_enabled {
            debug!("Skipping TUI cell (not in TUI mode)");
            continue;
        }

        if !binary_path.exists() {
            debug!("Cell binary not found: {}", binary_path.display());
            continue;
        }

        // Add peer to host - all cell deaths are fatal
        let cell_name_for_death = cell_name.clone();
        let ticket = host.add_peer(AddPeerOptions {
            peer_name: Some(binary_name.clone()),
            on_death: Some(Arc::new(move |peer_id| {
                eprintln!(
                    "FATAL: {} cell died unexpectedly (peer_id={:?})",
                    cell_name_for_death, peer_id
                );
                std::process::exit(1);
            })),
        })?;

        let peer_id = ticket.peer_id;

        // Spawn cell process
        let mut cmd = Command::new(&binary_path);
        for arg in ticket.to_args() {
            cmd.arg(arg);
        }

        if cell_def.inherit_stdio {
            cmd.stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        } else if is_quiet_mode() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .env("DODECA_QUIET", "1");
        } else {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        }

        let mut child = ur_taking_me_with_you::spawn_dying_with_parent_async(cmd)?;

        // Capture stdio (not for cells that inherit)
        if !cell_def.inherit_stdio && !is_quiet_mode() {
            capture_cell_stdio(&binary_name, &mut child);
        }

        debug!(
            "Spawned {} cell (peer_id={:?}) from {}",
            cell_name,
            peer_id,
            binary_path.display()
        );

        peer_cells.push((peer_id, binary_name, cell_name, child));

        // Drop ticket after spawn to close our end of the doorbell
        drop(ticket);
    }

    if peer_cells.is_empty() {
        return Err(eyre::eyre!("No cells found to spawn"));
    }

    // Build the unified host service - one service for all cells
    // Note: TUI command forwarding and render contexts go through Host::get()
    let host_service = HostServiceImpl::new(
        HostCellLifecycle::new(cell_ready_registry().clone()),
        TemplateHostImpl::new(),
        SITE_SERVER_FOR_INIT.get().cloned(),
    );

    // Every cell gets the same dispatcher
    let mut builder = MultiPeerHostDriver::new(host);
    for (peer_id, _, _, _) in &peer_cells {
        let dispatcher = HostServiceDispatcher::new(host_service.clone());
        builder = builder.add_peer(*peer_id, dispatcher);
    }

    let (driver, handles) = builder.build();

    // Initialize global handle storage
    let cell_handles = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let name_to_peer = Arc::new(std::sync::RwLock::new(HashMap::new()));

    // Store handles globally and register with Host
    for (peer_id, binary_name, cell_name, _) in &peer_cells {
        if let Some(handle) = handles.get(peer_id) {
            if let Ok(mut map) = cell_handles.write() {
                map.insert(*peer_id, handle.clone());
            }
            if let Ok(mut map) = name_to_peer.write() {
                map.insert(cell_name.clone(), *peer_id);
            }

            // Register with Host for type-safe client access
            crate::host::Host::get().register_cell_handle(binary_name.clone(), handle.clone());

            // Also register for diagnostics
            register_peer_diag(peer_id.get() as u16, cell_name, handle.clone());
        }
    }
    let _ = CELL_HANDLES.set(cell_handles);
    let _ = CELL_NAME_TO_PEER_ID.set(name_to_peer);

    // Spawn driver task
    tokio::spawn(async move {
        if let Err(e) = driver.run().await {
            warn!("MultiPeerHostDriver error: {:?}", e);
        }
    });

    // Spawn child management tasks
    for (peer_id, _, cell_name, child) in peer_cells {
        let cell_label = cell_name.clone();
        tokio::spawn(async move {
            let mut child = child;
            match child.wait().await {
                Ok(status) => {
                    if !status.success() {
                        eprintln!("FATAL: {} cell crashed with status: {}", cell_label, status);
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("FATAL: {} cell wait error: {}", cell_label, e);
                    std::process::exit(1);
                }
            }
            info!("{} cell (peer {:?}) exited", cell_label, peer_id);
        });
    }

    Ok(())
}

/// Find the directory containing cell binaries.
fn find_cell_directory() -> eyre::Result<PathBuf> {
    // Try DODECA_CELL_PATH first
    if let Ok(path) = std::env::var("DODECA_CELL_PATH") {
        let dir = PathBuf::from(path);
        if dir.is_dir() {
            return Ok(dir);
        }
    }

    // Try adjacent to current exe
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(dir) = exe_path.parent() {
            if dir.join("ddc-cell-image").exists() || dir.join("ddc-cell-image.exe").exists() {
                return Ok(dir.to_path_buf());
            }
        }
    }

    // Try target/debug or target/release
    #[cfg(debug_assertions)]
    let profile = "debug";
    #[cfg(not(debug_assertions))]
    let profile = "release";

    let target_dir = PathBuf::from("target").join(profile);
    if target_dir.is_dir() {
        return Ok(target_dir);
    }

    Err(eyre::eyre!("Could not find cell binary directory"))
}

pub async fn all() -> &'static CellRegistry {
    CELLS.get_or_init(init_cells).await
}

// ============================================================================
// Hub Access (for cell_server.rs compatibility)
// ============================================================================

pub async fn get_hub() -> Option<(Arc<ShmHost>, PathBuf)> {
    // The hub is now owned by the MultiPeerHostDriver, not accessible externally
    // This function is deprecated for the roam architecture
    warn!("get_hub() is deprecated - hub is owned by MultiPeerHostDriver");
    None
}

// ============================================================================
// Cell Client Accessor Functions
// ============================================================================

/// Create a client for the given cell if available.
///
/// Uses Host for handle lookup when available, falls back to legacy lookup.
macro_rules! cell_client_accessor {
    ($name:ident, $suffix:expr, $client:ty) => {
        pub async fn $name() -> Option<Arc<$client>> {
            // Use Host for handle lookup (lazily initializes)
            crate::host::Host::get().client::<$client>().map(Arc::new)
        }
    };
}

// Image processing
cell_client_accessor!(image_cell, "image", ImageProcessorClient);
cell_client_accessor!(webp_cell, "webp", WebPProcessorClient);
cell_client_accessor!(jxl_cell, "jxl", JXLProcessorClient);

// Text processing
cell_client_accessor!(markdown_cell, "markdown", MarkdownProcessorClient);
cell_client_accessor!(html_cell, "html", HtmlProcessorClient);
cell_client_accessor!(minify_cell, "minify", MinifierClient);
cell_client_accessor!(css_cell, "css", CssProcessorClient);
cell_client_accessor!(sass_cell, "sass", SassCompilerClient);
cell_client_accessor!(js_cell, "js", JsProcessorClient);
cell_client_accessor!(svgo_cell, "svgo", SvgoOptimizerClient);

// Other cells
cell_client_accessor!(font_cell, "fonts", FontProcessorClient);
cell_client_accessor!(linkcheck_cell, "linkcheck", LinkCheckerClient);
cell_client_accessor!(html_diff_cell, "html_diff", HtmlDifferClient);
cell_client_accessor!(dialoguer_cell, "dialoguer", DialoguerClient);
cell_client_accessor!(pagefind_cell, "pagefind", SearchIndexerClient);
cell_client_accessor!(code_execution_cell, "code_execution", CodeExecutorClient);
cell_client_accessor!(http_cell, "http", TcpTunnelClient);

// ============================================================================
// Convenience Functions (wrappers around cell clients)
// ============================================================================

pub async fn resize_image(input: ResizeInput) -> Result<ImageResult, eyre::Error> {
    let client = image_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Image cell not available"))?;
    client
        .resize_image(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn encode_webp(input: WebPEncodeInput) -> Result<WebPResult, eyre::Error> {
    let client = webp_cell()
        .await
        .ok_or_else(|| eyre::eyre!("WebP cell not available"))?;
    client
        .encode_webp(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn encode_jxl(input: JXLEncodeInput) -> Result<JXLResult, eyre::Error> {
    let client = jxl_cell()
        .await
        .ok_or_else(|| eyre::eyre!("JXL cell not available"))?;
    client
        .encode_jxl(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn compute_thumbhash(input: ThumbhashInput) -> Result<ImageResult, eyre::Error> {
    let client = image_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Image cell not available"))?;
    client
        .generate_thumbhash_data_url(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn parse_markdown(
    source_path: &str,
    content: String,
) -> Result<ParseResult, eyre::Error> {
    let client = markdown_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Markdown cell not available"))?;
    client
        .parse_and_render(source_path.to_string(), content)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn render_markdown(
    source_path: &str,
    markdown: String,
) -> Result<MarkdownResult, eyre::Error> {
    let client = markdown_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Markdown cell not available"))?;
    client
        .render_markdown(source_path.to_string(), markdown)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn extract_frontmatter(content: String) -> Result<FrontmatterResult, eyre::Error> {
    let client = markdown_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Markdown cell not available"))?;
    client
        .parse_frontmatter(content)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn minify_html(html: String) -> Result<MinifyResult, eyre::Error> {
    let client = minify_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Minify cell not available"))?;
    client
        .minify_html(html)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn compile_sass(input: SassInput) -> Result<SassResult, eyre::Error> {
    let client = sass_cell()
        .await
        .ok_or_else(|| eyre::eyre!("SASS cell not available"))?;
    client
        .compile_sass(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn rewrite_js(input: JsRewriteInput) -> Result<JsResult, eyre::Error> {
    let client = js_cell()
        .await
        .ok_or_else(|| eyre::eyre!("JS cell not available"))?;
    client
        .rewrite_string_literals(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn optimize_svg(svg: String) -> Result<SvgoResult, eyre::Error> {
    let client = svgo_cell()
        .await
        .ok_or_else(|| eyre::eyre!("SVGO cell not available"))?;
    client
        .optimize_svg(svg)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn subset_font(input: SubsetFontInput) -> Result<FontResult, eyre::Error> {
    let client = font_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Font cell not available"))?;
    client
        .subset_font(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn check_links(input: LinkCheckInput) -> Result<LinkCheckResult, eyre::Error> {
    let client = linkcheck_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Link check cell not available"))?;
    client
        .check_links(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn diff_html(input: DiffInput) -> Result<HtmlDiffResult, eyre::Error> {
    let client = html_diff_cell()
        .await
        .ok_or_else(|| eyre::eyre!("HTML diff cell not available"))?;
    client
        .diff_html(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn build_search_index(input: SearchIndexInput) -> Result<SearchIndexResult, eyre::Error> {
    let client = pagefind_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Pagefind cell not available"))?;
    client
        .build_search_index(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn execute_code_samples(
    input: ExecuteSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    let client = code_execution_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Code execution cell not available"))?;
    client
        .execute_code_samples(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn extract_code_samples(
    input: ExtractSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    let client = code_execution_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Code execution cell not available"))?;
    client
        .extract_code_samples(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

// ============================================================================
// Additional Function Aliases (for compatibility with other modules)
// ============================================================================

// These are aliases for the cell accessor and wrapper functions
// that other modules expect.

pub use dialoguer_cell as dialoguer_client;

pub fn has_linkcheck_cell() -> bool {
    // Check if the cell handle is available
    get_cell_handle_by_name("linkcheck").is_some()
}

/// Result of link checking - wrapper for internal use
#[derive(Debug, Clone)]
pub struct UrlCheckResult {
    pub statuses: Vec<LinkStatus>,
}

pub async fn check_urls_cell(urls: Vec<String>, options: CheckOptions) -> Option<UrlCheckResult> {
    let client = linkcheck_cell().await?;
    let input = LinkCheckInput {
        urls,
        delay_ms: options.rate_limit_ms,
        timeout_secs: options.timeout_secs,
    };
    match client.check_links(input).await {
        Ok(LinkCheckResult::Success { output }) => Some(UrlCheckResult {
            statuses: output.results.into_values().collect(),
        }),
        Ok(LinkCheckResult::Error { message }) => {
            tracing::warn!("Link check error: {}", message);
            None
        }
        Err(e) => {
            tracing::warn!("Link check RPC error: {:?}", e);
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    pub timeout_secs: u64,
    pub skip_domains: Vec<String>,
    pub rate_limit_ms: u64,
}

pub async fn parse_and_render_markdown_cell(
    source_path: &str,
    content: &str,
) -> Result<cell_markdown_proto::ParseResult, MarkdownParseError> {
    let client = markdown_cell().await.ok_or_else(|| MarkdownParseError {
        message: "Markdown cell not available".to_string(),
    })?;
    client
        .parse_and_render(source_path.to_string(), content.to_string())
        .await
        .map_err(|e| MarkdownParseError {
            message: format!("RPC error: {:?}", e),
        })
}

pub async fn execute_code_samples_cell(
    input: ExecuteSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    execute_code_samples(input).await
}

pub async fn extract_code_samples_cell(
    input: ExtractSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    extract_code_samples(input).await
}

pub async fn inject_code_buttons_cell(
    html: String,
    code_metadata: HashMap<String, cell_html_proto::CodeExecutionMetadata>,
) -> Result<(String, bool), eyre::Error> {
    let client = html_cell()
        .await
        .ok_or_else(|| eyre::eyre!("HTML cell not available"))?;
    match client.inject_code_buttons(html, code_metadata).await {
        Ok(cell_html_proto::HtmlResult::SuccessWithFlag { html, flag }) => Ok((html, flag)),
        Ok(cell_html_proto::HtmlResult::Success { html }) => Ok((html, false)),
        Ok(cell_html_proto::HtmlResult::Error { message }) => Err(eyre::eyre!(message)),
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

pub async fn render_template_cell(
    context_id: ContextId,
    template_name: &str,
    initial_context: Value,
) -> eyre::Result<RenderResult> {
    render_template(context_id, template_name, initial_context).await
}

pub async fn build_search_index_cell(
    input: SearchIndexInput,
) -> Result<SearchIndexResult, eyre::Error> {
    build_search_index(input).await
}

pub async fn diff_html_cell(input: DiffInput) -> Result<HtmlDiffResult, eyre::Error> {
    diff_html(input).await
}

pub async fn minify_html_cell(input: String) -> Result<MinifyResult, eyre::Error> {
    minify_html(input).await
}

pub async fn optimize_svg_cell(input: String) -> Result<SvgoResult, eyre::Error> {
    optimize_svg(input).await
}

pub async fn mark_dead_links_cell(
    html: String,
    known_routes: std::collections::HashSet<String>,
) -> Result<String, eyre::Error> {
    let client = html_cell()
        .await
        .ok_or_else(|| eyre::eyre!("HTML cell not available"))?;
    match client.mark_dead_links(html, known_routes).await {
        Ok(cell_html_proto::HtmlResult::Success { html }) => Ok(html),
        Ok(cell_html_proto::HtmlResult::SuccessWithFlag { html, .. }) => Ok(html),
        Ok(cell_html_proto::HtmlResult::Error { message }) => Err(eyre::eyre!(message)),
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

pub async fn rewrite_urls_in_html_cell(
    html: String,
    path_map: HashMap<String, String>,
) -> Result<String, eyre::Error> {
    let client = html_cell()
        .await
        .ok_or_else(|| eyre::eyre!("HTML cell not available"))?;
    match client.rewrite_urls(html, path_map).await {
        Ok(cell_html_proto::HtmlResult::Success { html }) => Ok(html),
        Ok(cell_html_proto::HtmlResult::SuccessWithFlag { html, .. }) => Ok(html),
        Ok(cell_html_proto::HtmlResult::Error { message }) => Err(eyre::eyre!(message)),
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

pub async fn rewrite_string_literals_in_js_cell(
    js: String,
    path_map: HashMap<String, String>,
) -> Result<String, eyre::Error> {
    let client = js_cell()
        .await
        .ok_or_else(|| eyre::eyre!("JS cell not available"))?;
    let input = JsRewriteInput { js, path_map };
    match client.rewrite_string_literals(input).await {
        Ok(JsResult::Success { js }) => Ok(js),
        Ok(JsResult::Error { message }) => Err(eyre::eyre!(message)),
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

pub async fn rewrite_urls_in_css_cell(
    css: String,
    path_map: HashMap<String, String>,
) -> Result<String, eyre::Error> {
    let client = css_cell()
        .await
        .ok_or_else(|| eyre::eyre!("CSS cell not available"))?;
    match client.rewrite_and_minify(css, path_map).await {
        Ok(CssResult::Success { css }) => Ok(css),
        Ok(CssResult::Error { message }) => Err(eyre::eyre!(message)),
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

pub async fn decompress_font_cell(data: Vec<u8>) -> Result<Vec<u8>, eyre::Error> {
    let client = font_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Font cell not available"))?;
    match client.decompress_font(data).await {
        Ok(FontResult::DecompressSuccess { data }) => Ok(data),
        Ok(FontResult::Error { message }) => Err(eyre::eyre!(message)),
        Ok(other) => Err(eyre::eyre!("Unexpected result: {:?}", other)),
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

pub async fn compress_to_woff2_cell(data: Vec<u8>) -> Result<Vec<u8>, eyre::Error> {
    let client = font_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Font cell not available"))?;
    match client.compress_to_woff2(data).await {
        Ok(FontResult::CompressSuccess { data }) => Ok(data),
        Ok(FontResult::Error { message }) => Err(eyre::eyre!(message)),
        Ok(other) => Err(eyre::eyre!("Unexpected result: {:?}", other)),
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

pub async fn subset_font_cell(input: SubsetFontInput) -> Result<FontResult, eyre::Error> {
    subset_font(input).await
}

// Image decoding/encoding cell wrappers
// These return Option to match what image.rs expects
pub async fn decode_png_cell(data: &[u8]) -> Option<DecodedImage> {
    let client = image_cell().await?;
    match client.decode_png(data.to_vec()).await {
        Ok(ImageResult::Success { image }) => Some(image),
        Ok(ImageResult::Error { message }) => {
            tracing::warn!("PNG decode error: {}", message);
            None
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("PNG decode RPC error: {:?}", e);
            None
        }
    }
}

pub async fn decode_jpeg_cell(data: &[u8]) -> Option<DecodedImage> {
    let client = image_cell().await?;
    match client.decode_jpeg(data.to_vec()).await {
        Ok(ImageResult::Success { image }) => Some(image),
        Ok(ImageResult::Error { message }) => {
            tracing::warn!("JPEG decode error: {}", message);
            None
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("JPEG decode RPC error: {:?}", e);
            None
        }
    }
}

pub async fn decode_gif_cell(data: &[u8]) -> Option<DecodedImage> {
    let client = image_cell().await?;
    match client.decode_gif(data.to_vec()).await {
        Ok(ImageResult::Success { image }) => Some(image),
        Ok(ImageResult::Error { message }) => {
            tracing::warn!("GIF decode error: {}", message);
            None
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("GIF decode RPC error: {:?}", e);
            None
        }
    }
}

pub async fn decode_webp_cell(data: &[u8]) -> Option<DecodedImage> {
    let client = webp_cell().await?;
    match client.decode_webp(data.to_vec()).await {
        Ok(WebPResult::DecodeSuccess {
            pixels,
            width,
            height,
            channels,
        }) => Some(DecodedImage {
            pixels,
            width,
            height,
            channels,
        }),
        Ok(WebPResult::Error { message }) => {
            tracing::warn!("WebP decode error: {}", message);
            None
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("WebP decode RPC error: {:?}", e);
            None
        }
    }
}

pub async fn decode_jxl_cell(data: &[u8]) -> Option<DecodedImage> {
    let client = jxl_cell().await?;
    match client.decode_jxl(data.to_vec()).await {
        Ok(JXLResult::DecodeSuccess {
            pixels,
            width,
            height,
            channels,
        }) => Some(DecodedImage {
            pixels,
            width,
            height,
            channels,
        }),
        Ok(JXLResult::Error { message }) => {
            tracing::warn!("JXL decode error: {}", message);
            None
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("JXL decode RPC error: {:?}", e);
            None
        }
    }
}

pub async fn resize_image_cell(
    pixels: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    target_width: u32,
) -> Option<DecodedImage> {
    let client = image_cell().await?;
    let input = ResizeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        channels,
        target_width,
    };
    match client.resize_image(input).await {
        Ok(ImageResult::Success { image }) => Some(image),
        Ok(ImageResult::Error { message }) => {
            tracing::warn!("Resize error: {}", message);
            None
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("Resize RPC error: {:?}", e);
            None
        }
    }
}

pub async fn generate_thumbhash_cell(pixels: &[u8], width: u32, height: u32) -> Option<String> {
    let client = image_cell().await?;
    let input = ThumbhashInput {
        pixels: pixels.to_vec(),
        width,
        height,
    };
    match client.generate_thumbhash_data_url(input).await {
        Ok(ImageResult::ThumbhashSuccess { data_url }) => Some(data_url),
        Ok(ImageResult::Error { message }) => {
            tracing::warn!("Thumbhash error: {}", message);
            None
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("Thumbhash RPC error: {:?}", e);
            None
        }
    }
}

pub async fn encode_webp_cell(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Option<Vec<u8>> {
    let client = webp_cell().await?;
    let input = WebPEncodeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };
    match client.encode_webp(input).await {
        Ok(WebPResult::EncodeSuccess { data }) => Some(data),
        Ok(WebPResult::Error { message }) => {
            tracing::warn!("WebP encode error: {}", message);
            None
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("WebP encode RPC error: {:?}", e);
            None
        }
    }
}

pub async fn encode_jxl_cell(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Option<Vec<u8>> {
    let client = jxl_cell().await?;
    let input = JXLEncodeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };
    match client.encode_jxl(input).await {
        Ok(JXLResult::EncodeSuccess { data }) => Some(data),
        Ok(JXLResult::Error { message }) => {
            tracing::warn!("JXL encode error: {}", message);
            None
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("JXL encode RPC error: {:?}", e);
            None
        }
    }
}

// SASS/CSS cell wrappers
pub async fn compile_sass_cell(input: &HashMap<String, String>) -> Result<SassResult, eyre::Error> {
    let client = sass_cell()
        .await
        .ok_or_else(|| eyre::eyre!("SASS cell not available"))?;
    let sass_input = SassInput {
        files: input.clone(),
    };
    client
        .compile_sass(sass_input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

// Markdown error type
#[derive(Debug, Clone, Facet)]
pub struct MarkdownParseError {
    pub message: String,
}

impl std::fmt::Display for MarkdownParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MarkdownParseError {}

// HTML/CSS extraction
pub async fn extract_css_from_html_cell(html: &str) -> Result<String, eyre::Error> {
    let client = font_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Font cell not available"))?;
    match client.extract_css_from_html(html.to_string()).await {
        Ok(FontResult::CssSuccess { css }) => Ok(css),
        Ok(FontResult::Error { message }) => Err(eyre::eyre!(message)),
        Ok(other) => Err(eyre::eyre!("Unexpected result: {:?}", other)),
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

// Font analysis
pub async fn analyze_fonts_cell(html: &str, css: &str) -> Result<FontAnalysis, eyre::Error> {
    let client = font_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Font cell not available"))?;
    match client
        .analyze_fonts(html.to_string(), css.to_string())
        .await
    {
        Ok(FontResult::AnalysisSuccess { analysis }) => Ok(analysis),
        Ok(FontResult::Error { message }) => Err(eyre::eyre!(message)),
        Ok(other) => Err(eyre::eyre!("Unexpected result: {:?}", other)),
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

/// Spawn a single cell with a custom dispatcher.
///
/// This creates a separate SHM segment for the cell, which is appropriate for
/// cells like TUI that need special handling (inherit_stdio, custom dispatcher).
///
/// For the main cells, use `init_cells()` which uses `MultiPeerHostDriver`.
pub fn spawn_cell_with_dispatcher<D>(
    binary_path: &Path,
    binary_name: &str,
    dispatcher: D,
    config: &CellSpawnConfig,
) -> Option<SpawnedCellResult>
where
    D: ServiceDispatcher + Send + 'static,
{
    use roam_shm::driver::establish_host_peer;

    // Create a separate SHM segment for this cell
    let shm_path =
        std::env::temp_dir().join(format!("dodeca-{}-{}", binary_name, std::process::id()));

    let max_payload = 4 * 1024 * 1024; // 4MB
    let segment_config = SegmentConfig {
        max_guests: 2, // Just host + this cell
        ring_size: 128,
        slots_per_guest: 32,
        slot_size: max_payload + 8,
        max_payload_size: max_payload,
        ..SegmentConfig::default()
    };

    let mut host = match ShmHost::create(&shm_path, segment_config) {
        Ok(h) => h,
        Err(e) => {
            warn!("Failed to create SHM host for {}: {:?}", binary_name, e);
            return None;
        }
    };

    // Add peer
    let cell_name_for_death = binary_name.to_string();
    let ticket = match host.add_peer(AddPeerOptions {
        peer_name: Some(binary_name.to_string()),
        on_death: Some(Arc::new(move |peer_id| {
            warn!(
                "{} cell died unexpectedly (peer_id={:?})",
                cell_name_for_death, peer_id
            );
        })),
    }) {
        Ok(t) => t,
        Err(e) => {
            warn!("Failed to add peer for {}: {:?}", binary_name, e);
            return None;
        }
    };

    let peer_id = ticket.peer_id;

    // Spawn cell process
    let mut cmd = Command::new(binary_path);
    for arg in ticket.to_args() {
        cmd.arg(arg);
    }

    if config.inherit_stdio {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    } else if is_quiet_mode() {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("DODECA_QUIET", "1");
    } else {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }

    let mut child = match ur_taking_me_with_you::spawn_dying_with_parent_async(cmd) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to spawn {}: {:?}", binary_name, e);
            return None;
        }
    };

    // Capture stdio if not inheriting and not quiet
    if !config.inherit_stdio && !is_quiet_mode() {
        capture_cell_stdio(binary_name, &mut child);
    }

    // Drop ticket to close our end of the doorbell
    drop(ticket);

    // Establish host-side connection with the provided dispatcher
    let (handle, driver) = establish_host_peer(host, peer_id, dispatcher);

    // Spawn driver task
    let cell_label = binary_name.to_string();
    tokio::spawn(async move {
        if let Err(e) = driver.run().await {
            warn!("{} cell driver error: {:?}", cell_label, e);
        }
    });

    debug!(
        "Spawned {} cell (peer_id={:?}) from {}",
        binary_name,
        peer_id,
        binary_path.display()
    );

    let child = if config.manage_child {
        let cell_label = binary_name.to_string();
        let child_handle = child;
        tokio::spawn(async move {
            let mut child = child_handle;
            match child.wait().await {
                Ok(status) => {
                    if !status.success() {
                        warn!("{} cell exited with status: {}", cell_label, status);
                    }
                }
                Err(e) => {
                    warn!("{} cell wait error: {:?}", cell_label, e);
                }
            }
        });
        None
    } else {
        Some(child)
    };

    Some(SpawnedCellResult {
        peer_id: peer_id.get() as u16,
        handle,
        child,
    })
}
