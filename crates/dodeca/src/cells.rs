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
use roam_shm::{AddPeerOptions, SegmentConfig, ShmHost};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tracing::{debug, error, info, warn};

use crate::serve::SiteServer;

// ============================================================================
// Global State
// ============================================================================

// Note: Most globals have been moved to Host singleton:
// - Site server: Host::get().site_server()
// - Quiet mode: Host::get().is_quiet_mode()
// - Cell handles: Host::get().get_cell_handle(name)

/// Provide the SiteServer for HTTP cell initialization.
/// This must be called before cells are initialized when the HTTP cell needs to serve content.
/// For build-only commands, this can be skipped.
pub fn provide_site_server(server: Arc<SiteServer>) {
    crate::host::Host::get().provide_site_server(server);
}

// Note: TUI command forwarding now goes through Host::get().handle_tui_command()
// The old TUI_HOST_FOR_INIT global has been removed.
// Exit signaling now goes through Host::get().signal_exit() / wait_for_exit().
// Quiet mode now goes through Host::get().set_quiet_mode() / is_quiet_mode().

/// Enable quiet mode for spawned cells (call this when TUI is active).
pub fn set_quiet_mode(quiet: bool) {
    crate::host::Host::get().set_quiet_mode(quiet);
}

/// Check if quiet mode is enabled.
fn is_quiet_mode() -> bool {
    crate::host::Host::get().is_quiet_mode()
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
/// Tracks by cell name (logical name like "gingembre", "sass").
#[derive(Clone)]
pub struct CellReadyRegistry {
    ready: Arc<DashMap<String, ReadyMsg>>,
}

impl CellReadyRegistry {
    fn new() -> Self {
        Self {
            ready: Arc::new(DashMap::new()),
        }
    }

    fn mark_ready(&self, msg: ReadyMsg) {
        // Normalize: cells report with underscores (code_execution) but we use hyphens (code-execution)
        let cell_name = msg.cell_name.replace('_', "-");
        debug!(
            cell_name = %cell_name,
            peer_id = msg.peer_id,
            "CellReadyRegistry::mark_ready: marking cell as ready"
        );
        self.ready.insert(cell_name.clone(), msg);
        debug!(
            cell_name = %cell_name,
            "CellReadyRegistry::mark_ready: cell marked ready, registry now has {} cells",
            self.ready.len()
        );
    }

    pub fn is_ready(&self, cell_name: &str) -> bool {
        self.ready.contains_key(cell_name)
    }

    pub async fn wait_for_all_ready(
        &self,
        cell_names: &[&str],
        timeout: Duration,
    ) -> eyre::Result<()> {
        let start = std::time::Instant::now();
        for &cell_name in cell_names {
            loop {
                if self.is_ready(cell_name) {
                    break;
                }
                if start.elapsed() >= timeout {
                    return Err(eyre::eyre!("Timeout waiting for {} to be ready", cell_name));
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
    cell_ready_registry()
        .wait_for_all_ready(cell_names, timeout)
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

/// Get the TUI display client for pushing updates to the TUI cell.
/// This will spawn the TUI cell if it hasn't been spawned yet.
pub async fn get_tui_display_client() -> Option<TuiDisplayClient> {
    crate::host::Host::get()
        .client_async::<TuiDisplayClient>()
        .await
}

// ============================================================================
// Decoded Image Type (re-export)
// ============================================================================

pub type DecodedImage = cell_image_proto::DecodedImage;

// ============================================================================
// Cell Registry
// ============================================================================

static CELLS: tokio::sync::OnceCell<CellRegistry> = tokio::sync::OnceCell::const_new();
static INIT_ERROR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Ensure cell registry is initialized (registers cells for lazy spawning, does NOT spawn them).
/// This is idempotent and safe to call multiple times.
/// Called automatically by client_async(), should not be called directly.
pub(crate) async fn ensure_cell_registry_initialized() -> eyre::Result<()> {
    let _ = CELLS.get_or_init(init_cells).await;

    // Check if init failed
    if let Some(err) = INIT_ERROR.get() {
        return Err(eyre::eyre!("Cell initialization failed: {}", err));
    }

    Ok(())
}

async fn push_tracing_config_to_cells() {
    let filter_str = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| crate::logging::DEFAULT_TRACING_FILTER.to_string());

    for (cell_name, handle) in crate::host::Host::get().iter_cell_handles() {
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
                    debug!("Pushed tracing config to {} cell", cell_name);
                }
                Ok(roam_tracing::ConfigResult::InvalidFilter(msg)) => {
                    warn!("Invalid tracing filter for {} cell: {}", cell_name, msg);
                }
                Err(e) => {
                    warn!(
                        "Failed to push tracing config to {} cell: {:?}",
                        cell_name, e
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
// Template Rendering
// ============================================================================

pub async fn render_template(
    context_id: ContextId,
    template_name: &str,
    initial_context: Value,
) -> eyre::Result<RenderResult> {
    let cell = crate::host::Host::get()
        .client_async::<TemplateRendererClient>()
        .await
        .ok_or_else(|| eyre::eyre!("Gingembre cell not available"))?;
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
    let cell = crate::host::Host::get()
        .client_async::<TemplateRendererClient>()
        .await
        .ok_or_else(|| eyre::eyre!("Gingembre cell not available"))?;
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
            info!("Cell infrastructure initialized");
        }
        Err(e) => {
            let _ = INIT_ERROR.set(e.to_string());
        }
    }
    CellRegistry::new()
}

async fn init_cells_inner() -> eyre::Result<()> {
    use crate::host::PendingCell;

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
    debug!("init_cells_inner: SHM host created");

    // Find cell binary directory
    let cell_dir = find_cell_directory()?;
    debug!(cell_dir = %cell_dir.display(), "init_cells_inner: found cell directory");

    // Collect cell metadata for lazy spawning (no peer_ids or tickets yet)
    let mut cell_info: Vec<(&'static str, PathBuf, bool)> = Vec::new();
    let mut missing_binaries: Vec<(&'static str, PathBuf)> = Vec::new();

    // Check if TUI mode is enabled
    let tui_enabled = crate::host::Host::get().is_tui_mode();

    for cell_def in CELL_DEFS {
        let binary_name = format!("ddc-cell-{}", cell_def.suffix);
        let cell_name = cell_def.suffix; // Logical name (e.g., "gingembre", "html-diff")
        let binary_path = cell_dir.join(&binary_name);

        // Skip TUI cell if not in TUI mode
        if cell_name == "tui" && !tui_enabled {
            continue;
        }

        if !binary_path.exists() {
            missing_binaries.push((cell_name, binary_path));
            continue;
        }

        debug!(
            "Registered {} cell for lazy spawn from {}",
            cell_name,
            binary_path.display()
        );

        cell_info.push((cell_name, binary_path, cell_def.inherit_stdio));
    }

    // If most cells are missing, installation is incomplete
    let total_expected = cell_info.len() + missing_binaries.len();
    if missing_binaries.len() > total_expected / 2 {
        let missing_names: Vec<_> = missing_binaries.iter().map(|(n, _)| *n).collect();
        return Err(eyre::eyre!(
            "dodeca installation is incomplete.\n\n\
            Missing {} of {} cell binaries: {}\n\n\
            The cell binaries (ddc-cell-*) should be in the same directory as 'ddc'.\n\
            Please reinstall dodeca or download a complete release.",
            missing_binaries.len(),
            total_expected,
            missing_names.join(", ")
        ));
    } else if !missing_binaries.is_empty() {
        // Only a few missing - warn but continue
        let names: Vec<_> = missing_binaries.iter().map(|(n, _)| *n).collect();
        warn!("Some cell binaries not found: {:?}", names);
    }

    // Build driver with NO peers initially (lazy spawning)
    let builder = MultiPeerHostDriver::builder(host);
    let (driver, _handles, driver_handle) = builder.build();
    debug!("init_cells_inner: driver built with no peers (lazy spawning enabled)");

    // Store driver handle for dynamic peer creation
    crate::host::Host::get().set_driver_handle(driver_handle);

    // Store cell metadata as pending cells (will create peers on first access)
    for (cell_name, binary_path, inherit_stdio) in cell_info {
        debug!(
            cell = cell_name,
            "init_cells_inner: registering pending cell"
        );

        let pending = PendingCell {
            binary_path,
            inherit_stdio,
        };
        crate::host::Host::get().register_pending_cell(cell_name.to_string(), pending);
    }

    debug!("init_cells_inner: spawning driver task");

    // Spawn driver task
    tokio::spawn(async move {
        info!("MultiPeerHostDriver: starting (lazy spawning mode)");
        match driver.run().await {
            Ok(()) => {
                // Driver exited cleanly - this means control channel was disconnected
                // AND no peers were left
                error!(
                    "MultiPeerHostDriver: exited cleanly - control channel disconnected with no peers"
                );
            }
            Err(e) => {
                error!("MultiPeerHostDriver: exited with error: {:?}", e);
            }
        }
    });

    debug!("init_cells_inner: complete");
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
/// Uses Host for handle lookup. With lazy spawning, will spawn cell on first access.
macro_rules! cell_client_accessor {
    ($name:ident, $suffix:expr, $client:ty) => {
        pub async fn $name() -> Option<Arc<$client>> {
            // Use Host for handle lookup with lazy spawning support
            crate::host::Host::get()
                .client_async::<$client>()
                .await
                .map(Arc::new)
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

// Template rendering
cell_client_accessor!(gingembre_cell, "gingembre", TemplateRendererClient);

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
    use roam_shm::driver::MultiPeerHostDriver;

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

    // Establish host-side connection with the provided dispatcher using multi-peer builder
    let builder = MultiPeerHostDriver::builder(host);
    let builder = builder.add_peer(peer_id, dispatcher);
    let (driver, mut handles, _driver_handle) = builder.build();
    let handle = handles.remove(&peer_id).expect("peer handle must exist");

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
