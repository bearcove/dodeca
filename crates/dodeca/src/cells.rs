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
use cell_data_proto::DataLoaderClient;
use cell_dialoguer_proto::DialoguerClient;
use cell_fonts_proto::{FontProcessorClient, FontResult, SubsetFontInput};
use cell_gingembre_proto::{ContextId, RenderResult, TemplateRendererClient};
use cell_host_proto::{
    CallFunctionResult, CommandResult, HostService, KeysAtResult, LoadTemplateResult, ReadyAck,
    ReadyMsg, ResolveDataResult, ServeContent, ServerCommand, Value,
};
use cell_html_diff_proto::HtmlDifferClient;
use cell_html_proto::HtmlProcessorClient;
use cell_http_proto::{ScopeEntry, TcpTunnelClient};
use cell_image_proto::{ImageProcessorClient, ImageResult, ResizeInput, ThumbhashInput};
use cell_js_proto::{JsProcessorClient, JsRewriteInput};
use cell_jxl_proto::{JXLEncodeInput, JXLProcessorClient, JXLResult};
use cell_lifecycle_proto::CellLifecycle;
use cell_linkcheck_proto::{LinkCheckInput, LinkCheckResult, LinkCheckerClient, LinkStatus};
use cell_markdown_proto::MarkdownProcessorClient;
use cell_minify_proto::{MinifierClient, MinifyResult};
use cell_sass_proto::{SassCompilerClient, SassInput, SassResult};
use cell_svgo_proto::{SvgoOptimizerClient, SvgoResult};
use cell_term_proto::{RecordConfig, TermRecorderClient, TermResult};
use cell_tui_proto::TuiDisplayClient;
use cell_vite_proto::ViteManagerClient;
use cell_webp_proto::{WebPEncodeInput, WebPProcessorClient, WebPResult};
use dashmap::DashMap;
use facet::Facet;

use roam_shm::driver::MultiPeerHostDriver;
use roam_shm::{SegmentConfig, ShmHost};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;
use tracing::{debug, error, warn};

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

// ============================================================================
// Cell Readiness Registry
// ============================================================================

/// Registry for tracking cell readiness (RPC-ready state).
/// Tracks by cell name (logical name like "gingembre", "sass").
#[derive(Clone)]
pub struct CellReadyRegistry {
    ready: Arc<DashMap<String, ReadyMsg>>,
    failed: Arc<DashMap<String, String>>,
}

impl CellReadyRegistry {
    fn new() -> Self {
        Self {
            ready: Arc::new(DashMap::new()),
            failed: Arc::new(DashMap::new()),
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

    /// Mark a cell as failed (called from child monitor task when process crashes).
    pub fn mark_failed(&self, cell_name: &str, reason: String) {
        self.failed.insert(cell_name.to_string(), reason);
    }

    /// Check if a cell has been marked as failed, returning the failure reason.
    pub fn failure_reason(&self, cell_name: &str) -> Option<String> {
        self.failed.get(cell_name).map(|r| r.value().clone())
    }
}

static CELL_READY_REGISTRY: OnceLock<CellReadyRegistry> = OnceLock::new();

pub fn cell_ready_registry() -> &'static CellReadyRegistry {
    CELL_READY_REGISTRY.get_or_init(CellReadyRegistry::new)
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
    async fn ready(&self, _cx: &roam::Context, msg: ReadyMsg) -> ReadyAck {
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
    async fn ready(&self, _cx: &roam::Context, msg: ReadyMsg) -> ReadyAck {
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
    async fn load_template(
        &self,
        cx: &roam::Context,
        context_id: ContextId,
        name: String,
    ) -> LoadTemplateResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host.load_template(cx, context_id, name).await
    }

    async fn resolve_data(
        &self,
        cx: &roam::Context,
        context_id: ContextId,
        path: Vec<String>,
    ) -> ResolveDataResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host.resolve_data(cx, context_id, path).await
    }

    async fn keys_at(
        &self,
        cx: &roam::Context,
        context_id: ContextId,
        path: Vec<String>,
    ) -> KeysAtResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host.keys_at(cx, context_id, path).await
    }

    async fn call_function(
        &self,
        cx: &roam::Context,
        context_id: ContextId,
        name: String,
        args: Vec<Value>,
        kwargs: Vec<(String, Value)>,
    ) -> CallFunctionResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host
            .call_function(cx, context_id, name, args, kwargs)
            .await
    }

    // Content Service
    async fn find_content(&self, cx: &roam::Context, path: String) -> ServeContent {
        if let Some(server) = &self.site_server {
            use cell_http_proto::ContentService;
            let content_service = crate::content_service::HostContentService::new(server.clone());
            content_service.find_content(cx, path).await
        } else {
            ServeContent::NotFound {
                html: "Not in serve mode".to_string(),
                generation: 0,
            }
        }
    }

    async fn get_scope(
        &self,
        cx: &roam::Context,
        route: String,
        path: Vec<String>,
    ) -> Vec<ScopeEntry> {
        if let Some(server) = &self.site_server {
            use cell_http_proto::ContentService;
            let content_service = crate::content_service::HostContentService::new(server.clone());
            content_service.get_scope(cx, route, path).await
        } else {
            vec![]
        }
    }

    async fn eval_expression(
        &self,
        cx: &roam::Context,
        route: String,
        expression: String,
    ) -> cell_host_proto::EvalResult {
        if let Some(server) = &self.site_server {
            use cell_http_proto::ContentService;
            let content_service = crate::content_service::HostContentService::new(server.clone());
            content_service.eval_expression(cx, route, expression).await
        } else {
            cell_host_proto::EvalResult::Err("Not in serve mode".to_string())
        }
    }

    // TUI Commands (TUI → Host)
    async fn send_command(&self, _cx: &roam::Context, command: ServerCommand) -> CommandResult {
        // Forward to Host singleton
        crate::host::Host::get().handle_tui_command(command)
    }

    async fn quit(&self, _cx: &roam::Context) {
        crate::host::Host::get().signal_exit();
    }

    // Vite Integration
    async fn get_vite_port(&self, _cx: &roam::Context) -> Option<u16> {
        crate::host::Host::get().get_vite_port()
    }

    // HTML Host callbacks
    async fn minify_css(
        &self,
        _cx: &roam::Context,
        css: String,
    ) -> cell_host_proto::MinifyCssResult {
        // Delegate to CSS cell for minification (empty path_map = minify only)
        match css_cell().await {
            Some(client) => match client.rewrite_and_minify(css, HashMap::new()).await {
                Ok(cell_css_proto::CssResult::Success { css }) => {
                    cell_host_proto::MinifyCssResult::Success { css }
                }
                Ok(cell_css_proto::CssResult::Error { message }) => {
                    cell_host_proto::MinifyCssResult::Error { message }
                }
                Err(e) => cell_host_proto::MinifyCssResult::Error {
                    message: format!("RPC error: {:?}", e),
                },
            },
            None => cell_host_proto::MinifyCssResult::Error {
                message: "CSS cell not available".to_string(),
            },
        }
    }

    async fn minify_js(&self, _cx: &roam::Context, js: String) -> cell_host_proto::MinifyJsResult {
        // Delegate to JS cell for minification (using empty path_map for minify-only)
        match js_cell().await {
            Some(client) => {
                let input = cell_js_proto::JsRewriteInput {
                    js,
                    path_map: HashMap::new(),
                };
                match client.rewrite_string_literals(input).await {
                    Ok(js) => cell_host_proto::MinifyJsResult::Success { js },
                    Err(e) => cell_host_proto::MinifyJsResult::Error {
                        message: format!("RPC error: {:?}", e),
                    },
                }
            }
            None => cell_host_proto::MinifyJsResult::Error {
                message: "JS cell not available".to_string(),
            },
        }
    }

    async fn process_inline_css(
        &self,
        _cx: &roam::Context,
        css: String,
        path_map: HashMap<String, String>,
    ) -> cell_host_proto::ProcessCssResult {
        // Delegate to CSS cell for URL rewriting
        match css_cell().await {
            Some(client) => match client.rewrite_and_minify(css, path_map).await {
                Ok(cell_css_proto::CssResult::Success { css }) => {
                    cell_host_proto::ProcessCssResult::Success { css }
                }
                Ok(cell_css_proto::CssResult::Error { message }) => {
                    cell_host_proto::ProcessCssResult::Error { message }
                }
                Err(e) => cell_host_proto::ProcessCssResult::Error {
                    message: format!("RPC error: {:?}", e),
                },
            },
            None => cell_host_proto::ProcessCssResult::Error {
                message: "CSS cell not available".to_string(),
            },
        }
    }

    async fn process_inline_js(
        &self,
        _cx: &roam::Context,
        js: String,
        path_map: HashMap<String, String>,
    ) -> cell_host_proto::ProcessJsResult {
        // Delegate to JS cell for string literal rewriting
        match js_cell().await {
            Some(client) => {
                let input = cell_js_proto::JsRewriteInput { js, path_map };
                match client.rewrite_string_literals(input).await {
                    Ok(js) => cell_host_proto::ProcessJsResult::Success { js },
                    Err(e) => cell_host_proto::ProcessJsResult::Error {
                        message: format!("RPC error: {:?}", e),
                    },
                }
            }
            None => cell_host_proto::ProcessJsResult::Error {
                message: "JS cell not available".to_string(),
            },
        }
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
    CellDef::new("mermaid"),
    CellDef::new("html"),
    CellDef::new("minify"),
    CellDef::new("css"),
    CellDef::new("sass"),
    CellDef::new("js"),
    CellDef::new("svgo"),
    CellDef::new("fonts"),
    CellDef::new("linkcheck"),
    CellDef::new("html-diff"),
    CellDef::new("dialoguer").inherit_stdio(),
    CellDef::new("code-execution"),
    CellDef::new("http"),
    CellDef::new("gingembre"),
    CellDef::new("data"),
    CellDef::new("vite"),
    // Term needs terminal access for PTY recording
    CellDef::new("term").inherit_stdio(),
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
            debug!("Cell infrastructure initialized");
        }
        Err(e) => {
            let _ = INIT_ERROR.set(e.to_string());
        }
    }
    CellRegistry::new()
}

async fn init_cells_inner() -> eyre::Result<()> {
    use crate::host::PendingCell;

    // Clean up stale SHM files from previous runs
    #[cfg(unix)]
    roam_shm::cleanup::cleanup_stale_shm_files("dodeca-shm")?;

    // Create SHM path in /tmp/roam-shm/ directory
    // The watchdog cleans up on exit, and we clean up stale files on startup
    #[cfg(unix)]
    let shm_path = roam_shm::cleanup::get_shm_path("dodeca-shm", std::process::id())?;

    // On Windows, use regular temp dir (Auto cleanup via FILE_FLAG_DELETE_ON_CLOSE)
    #[cfg(not(unix))]
    let shm_path = std::env::temp_dir().join(format!("dodeca-shm-{}", std::process::id()));

    // Configure segment for multi-cell architecture
    // We have ~19 cells, so allocate for 24 guests with headroom.
    // Keep 16MB max_payload for large images (decoded pixel data can be 30-50MB for high-res).
    //
    // Environment variable overrides (for debugging):
    //   DODECA_SHM_MAX_GUESTS - max guests (default: 24)
    //   DODECA_SHM_MAX_PAYLOAD_MB - max payload in MB (default: 16)
    fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    let max_payload_mb: u32 = env_or("DODECA_SHM_MAX_PAYLOAD_MB", 16);
    let max_payload = (max_payload_mb as usize) * 1024 * 1024;
    let max_guests: u32 = env_or("DODECA_SHM_MAX_GUESTS", 24);

    debug!(
        max_guests,
        max_payload_mb, "SHM config (override with DODECA_SHM_* env vars)"
    );

    use roam_shm::SizeClass;
    let config = SegmentConfig {
        max_guests,
        max_payload_size: max_payload as u32,
        var_slot_classes: vec![
            SizeClass::new(1024, 1024),            // 1 KB × 1024 (small RPC args)
            SizeClass::new(16 * 1024, 256),        // 16 KB × 256 (typical payloads)
            SizeClass::new(256 * 1024, 32),        // 256 KB × 32 (images, CSS)
            SizeClass::new(max_payload as u32, 8), // max_payload × 8 (large blobs)
        ],
        // On Windows: FILE_FLAG_DELETE_ON_CLOSE deletes when all handles close (guests can still open)
        // On Unix: Manual cleanup via Drop guard (unlink immediately breaks lazy spawning)
        file_cleanup: if cfg!(windows) {
            roam_shm::FileCleanup::Auto
        } else {
            roam_shm::FileCleanup::Manual
        },
        ..SegmentConfig::default()
    };

    // Create SHM host
    let host = ShmHost::create(&shm_path, config)?;
    debug!("init_cells_inner: SHM host created");

    // Extract diagnostic view before host is moved into driver
    let diagnostic_view = roam_shm::ShmDiagnosticView::from_host(&host);
    dodeca_debug::register_diagnostic(move || {
        let output = diagnostic_view.diagnostics().format();
        if !output.is_empty() {
            eprint!("{}", output);
        }
    });
    debug!("init_cells_inner: SHM diagnostic view registered for SIGUSR1");

    // On Unix: spawn watchdog and create .meta file
    #[cfg(unix)]
    {
        roam_shm::cleanup::write_meta_file(&shm_path)?;
        roam_shm::cleanup::spawn_watchdog(shm_path.clone())?;
    }

    // Find cell binary directory
    let cell_dir = find_cell_directory()?;
    debug!(cell_dir = %cell_dir.display(), "init_cells_inner: found cell directory");

    // Collect cell metadata for lazy spawning (no peer_ids or tickets yet)
    let mut cell_info: Vec<(&'static str, PathBuf, bool)> = Vec::new();
    let mut missing_binaries: Vec<(&'static str, PathBuf)> = Vec::new();

    // Check if TUI mode is enabled
    let tui_enabled = crate::host::Host::get().is_tui_mode();

    for cell_def in CELL_DEFS {
        #[cfg(windows)]
        let binary_name = format!("ddc-cell-{}.exe", cell_def.suffix);
        #[cfg(not(windows))]
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
    let (driver, _handles, _incoming_connections, driver_handle) = builder.build();
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
    let driver_handle = crate::spawn::spawn(async move {
        debug!("MultiPeerHostDriver: starting (lazy spawning mode)");
        debug!("[driver task] before driver.run()");

        let result = driver.run().await;

        debug!("[driver task] after driver.run(), result={:?}", result);

        match result {
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

    // Also spawn a watchdog that checks if driver task panics
    crate::spawn::spawn(async move {
        match driver_handle.await {
            Ok(()) => {
                error!("MultiPeerHostDriver task completed normally (this shouldn't happen early)");
            }
            Err(e) if e.is_panic() => {
                error!("MultiPeerHostDriver task PANICKED: {:?}", e);
                std::process::exit(1);
            }
            Err(e) if e.is_cancelled() => {
                error!("MultiPeerHostDriver task was cancelled");
            }
            Err(e) => {
                error!("MultiPeerHostDriver task failed: {:?}", e);
            }
        }
    });

    // Spawn tracing consumer task (receives records from cells)
    if let Some(mut tracing_rx) = crate::host::Host::get().take_tracing_receiver() {
        crate::spawn::spawn(async move {
            debug!("Tracing consumer: starting");
            while let Some(tagged) = tracing_rx.recv().await {
                // Dispatch through the host's tracing subscriber
                roam_tracing::dispatch_record(&tagged);
            }
            debug!("Tracing consumer: channel closed");
        });
    }

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
// Cell Client Accessor Functions
// ============================================================================

/// Create a client for the given cell if available.
///
/// Uses Host for handle lookup. With lazy spawning, will spawn cell on first access.
macro_rules! cell_client_accessor {
    ($name:ident, $suffix:expr, $client:ty) => {
        #[allow(unused)]
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

// Data processing
cell_client_accessor!(data_cell, "data", DataLoaderClient);

// Vite management
cell_client_accessor!(vite_cell, "vite", ViteManagerClient);

// Other cells
cell_client_accessor!(font_cell, "fonts", FontProcessorClient);
cell_client_accessor!(linkcheck_cell, "linkcheck", LinkCheckerClient);
cell_client_accessor!(html_diff_cell, "html_diff", HtmlDifferClient);
cell_client_accessor!(dialoguer_cell, "dialoguer", DialoguerClient);
cell_client_accessor!(code_execution_cell, "code_execution", CodeExecutorClient);
cell_client_accessor!(http_cell, "http", TcpTunnelClient);
cell_client_accessor!(term_cell, "term", TermRecorderClient);

/// Record a terminal session interactively
pub async fn record_term_interactive(config: RecordConfig) -> Result<TermResult, eyre::Error> {
    let client = term_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Term cell not available"))?;
    client
        .record_interactive(config)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

/// Record a terminal session with an auto-executed command
pub async fn record_term_command(
    command: String,
    config: RecordConfig,
) -> Result<TermResult, eyre::Error> {
    let client = term_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Term cell not available"))?;
    client
        .record_command(command, config)
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
    pub statuses: std::collections::HashMap<String, LinkStatus>,
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
            statuses: output.results,
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
    pub rate_limit_ms: u64,
}

pub async fn highlight_code_cell(lang: &str, code: &str) -> Result<String, eyre::Error> {
    let client = markdown_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Markdown cell not available"))?;
    match client
        .highlight_code(lang.to_string(), code.to_string())
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))?
    {
        cell_markdown_proto::HighlightResult::Success { html } => Ok(html),
        cell_markdown_proto::HighlightResult::Error { message } => {
            Err(eyre::eyre!("Highlight error: {}", message))
        }
    }
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

pub async fn eval_expression_cell(
    context_id: ContextId,
    expression: &str,
    context: Value,
) -> eyre::Result<cell_gingembre_proto::EvalResult> {
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

pub async fn minify_html_cell(input: String) -> Result<MinifyResult, eyre::Error> {
    minify_html(input).await
}

pub async fn optimize_svg_cell(input: String) -> Result<SvgoResult, eyre::Error> {
    optimize_svg(input).await
}

/// Extract links and element IDs from HTML using the HTML cell's parser.
/// This uses a proper HTML parser instead of regex.
pub async fn extract_links_from_html(
    html: String,
) -> Result<cell_html_proto::ExtractedLinks, eyre::Error> {
    let client = html_cell()
        .await
        .ok_or_else(|| eyre::eyre!("HTML cell not available"))?;
    client
        .extract_links(html)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
}

pub async fn rewrite_string_literals_in_js_cell(
    js: String,
    path_map: HashMap<String, String>,
) -> Result<String, eyre::Error> {
    let client = js_cell()
        .await
        .ok_or_else(|| eyre::eyre!("JS cell not available"))?;
    let input = JsRewriteInput { js, path_map };
    client
        .rewrite_string_literals(input)
        .await
        .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
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
pub async fn compile_sass_cell(
    input: &HashMap<String, String>,
    load_paths: &[String],
) -> Result<SassResult, eyre::Error> {
    let client = sass_cell()
        .await
        .ok_or_else(|| eyre::eyre!("SASS cell not available"))?;
    let sass_input = SassInput {
        files: input.clone(),
        load_paths: load_paths.to_vec(),
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
