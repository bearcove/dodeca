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
use cell_gingembre_proto::{
    CallFunctionResult, ContextId, EvalResult, KeysAtResult, LoadTemplateResult, RenderResult,
    ResolveDataResult, TemplateHost, TemplateHostDispatcher, TemplateRendererClient,
};
use cell_html_diff_proto::{DiffInput, DiffResult, HtmlDiffResult, HtmlDifferClient};
use cell_html_proto::HtmlProcessorClient;
use cell_http_proto::TcpTunnelClient;
use cell_image_proto::{ImageProcessorClient, ImageResult, ResizeInput, ThumbhashInput};
use cell_js_proto::{JsProcessorClient, JsResult, JsRewriteInput};
use cell_jxl_proto::{JXLEncodeInput, JXLProcessorClient, JXLResult};
use cell_linkcheck_proto::{LinkCheckInput, LinkCheckResult, LinkCheckerClient, LinkStatus};
use cell_markdown_proto::{
    FrontmatterResult, MarkdownProcessorClient, MarkdownResult, ParseResult,
};
use cell_minify_proto::{MinifierClient, MinifyResult};
use cell_pagefind_proto::{
    SearchFile, SearchIndexInput, SearchIndexResult, SearchIndexerClient, SearchPage,
};
use cell_sass_proto::{SassCompilerClient, SassInput, SassResult};
use cell_svgo_proto::{SvgoOptimizerClient, SvgoResult};
use cell_webp_proto::{WebPEncodeInput, WebPProcessorClient, WebPResult};
use dashmap::DashMap;
use facet::Facet;
use facet_value::Value;
use roam::session::{ChannelRegistry, ConnectionHandle, Never, RoamError, ServiceDispatcher};
use roam_shm::driver::{MultiPeerHostDriver, establish_multi_peer_host};
use roam_shm::{AddPeerOptions, PeerId, SegmentConfig, ShmHost, SpawnTicket};
use roam_tracing::{CellTracingClient, TracingHost};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tracing::{debug, info, warn};

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

/// Simple readiness message (replacing rapace_cell::ReadyMsg)
#[derive(Debug, Clone, Facet)]
pub struct ReadyMsg {
    pub peer_id: u16,
    pub cell_name: String,
}

/// Simple readiness acknowledgement
#[derive(Debug, Clone, Facet)]
pub struct ReadyAck {
    pub ok: bool,
    pub host_time_unix_ms: Option<u64>,
}

/// CellLifecycle service - cells call this to signal readiness
#[roam::service]
pub trait CellLifecycle {
    async fn ready(&self, msg: ReadyMsg) -> ReadyAck;
}

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
    async fn ready(&self, msg: ReadyMsg) -> Result<ReadyAck, RoamError<Never>> {
        let peer_id = msg.peer_id;
        let cell_name = msg.cell_name.clone();
        debug!("Cell {} (peer_id={}) is ready", cell_name, peer_id);
        self.registry.mark_ready(msg);

        let host_time_unix_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64);

        Ok(ReadyAck {
            ok: true,
            host_time_unix_ms,
        })
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
                Ok(Ok(roam_tracing::ConfigResult::Ok)) => {
                    debug!("Pushed tracing config to {} cell", cell_label);
                }
                Ok(Ok(roam_tracing::ConfigResult::InvalidFilter(msg))) => {
                    warn!("Invalid tracing filter for {} cell: {}", cell_label, msg);
                }
                Ok(Err(e)) => {
                    warn!(
                        "RPC error pushing tracing config to {} cell: {:?}",
                        cell_label, e
                    );
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
// Host Dispatcher (combines all host-side services)
// ============================================================================

/// No-op dispatcher for cells that don't need host callbacks
pub struct NoOpDispatcher;

impl ServiceDispatcher for NoOpDispatcher {
    fn dispatch(
        &self,
        _method_id: u64,
        _payload: Vec<u8>,
        request_id: u64,
        registry: &mut ChannelRegistry,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        roam::session::dispatch_unknown_method(request_id, registry)
    }
}

// ============================================================================
// Template Host Implementation (for gingembre cell)
// ============================================================================

// Re-export TemplateHostImpl from template_host module
pub use crate::template_host::{RenderContextRegistry, TemplateHostImpl};

// ============================================================================
// Gingembre Cell
// ============================================================================

static GINGEMBRE_CELL: tokio::sync::OnceCell<Arc<TemplateRendererClient>> =
    tokio::sync::OnceCell::const_new();

pub async fn init_gingembre_cell() -> Option<()> {
    // TODO: Implement gingembre cell initialization with roam
    // This requires setting up TemplateHostDispatcher and connecting to the cell
    tracing::debug!("init_gingembre_cell: not yet implemented for roam");
    None
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
        .map_err(|e| eyre::eyre!("RPC call error: {:?}", e))?
        .map_err(|e| eyre::eyre!("RPC service error: {:?}", e))?;
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
        .map_err(|e| eyre::eyre!("RPC call error: {:?}", e))?
        .map_err(|e| eyre::eyre!("RPC service error: {:?}", e))?;
    Ok(result)
}

// ============================================================================
// Render Context Registry
// ============================================================================

pub use crate::template_host::{RenderContext, render_context_registry};

// ============================================================================
// Cell Registry Implementation
// ============================================================================

pub struct CellRegistry {
    /// HTTP cell connection handle
    pub http: Option<ConnectionHandle>,
    /// Gingembre template cell connection handle
    pub gingembre: Option<ConnectionHandle>,
    /// Markdown cell connection handle
    pub markdown: Option<ConnectionHandle>,
    /// Image cell connection handle
    pub image: Option<ConnectionHandle>,
    /// HTML diff cell connection handle
    pub html_diff: Option<ConnectionHandle>,
    /// Search indexer cell connection handle
    pub search_indexer: Option<ConnectionHandle>,
    /// SASS cell connection handle
    pub sass: Option<ConnectionHandle>,
    /// CSS cell connection handle
    pub css: Option<ConnectionHandle>,
    /// JS cell connection handle
    pub js: Option<ConnectionHandle>,
}

impl CellRegistry {
    fn new() -> Self {
        Self {
            http: None,
            gingembre: None,
            markdown: None,
            image: None,
            html_diff: None,
            search_indexer: None,
            sass: None,
            css: None,
            js: None,
        }
    }
}

async fn init_cells() -> CellRegistry {
    // TODO: Initialize all cells using MultiPeerHostDriver
    // For now, return empty registry
    tracing::info!("Cell initialization not yet implemented for roam");
    CellRegistry::new()
}

pub async fn all() -> &'static CellRegistry {
    CELLS.get_or_init(init_cells).await
}

// ============================================================================
// Hub Access (for cell_server.rs compatibility)
// ============================================================================

pub async fn get_hub() -> Option<(Arc<ShmHost>, PathBuf)> {
    // TODO: Return proper SHM host reference
    None
}

// ============================================================================
// Placeholder Cell Client Functions
// ============================================================================

// These functions will be implemented once the cell infrastructure is complete.
// For now they return errors indicating cells aren't available.

macro_rules! cell_client_stub {
    ($name:ident, $ret:ty) => {
        pub async fn $name() -> Option<$ret> {
            tracing::warn!(concat!(stringify!($name), " not yet implemented for roam"));
            None
        }
    };
}

// Image processing
cell_client_stub!(image_cell, Arc<ImageProcessorClient>);
cell_client_stub!(webp_cell, Arc<WebPProcessorClient>);
cell_client_stub!(jxl_cell, Arc<JXLProcessorClient>);

// Text processing
cell_client_stub!(markdown_cell, Arc<MarkdownProcessorClient>);
cell_client_stub!(html_cell, Arc<HtmlProcessorClient>);
cell_client_stub!(minify_cell, Arc<MinifierClient>);
cell_client_stub!(css_cell, Arc<CssProcessorClient>);
cell_client_stub!(sass_cell, Arc<SassCompilerClient>);
cell_client_stub!(js_cell, Arc<JsProcessorClient>);
cell_client_stub!(svgo_cell, Arc<SvgoOptimizerClient>);

// Other cells
cell_client_stub!(font_cell, Arc<FontProcessorClient>);
cell_client_stub!(linkcheck_cell, Arc<LinkCheckerClient>);
cell_client_stub!(html_diff_cell, Arc<HtmlDifferClient>);
cell_client_stub!(dialoguer_cell, Arc<DialoguerClient>);
cell_client_stub!(pagefind_cell, Arc<SearchIndexerClient>);
cell_client_stub!(code_execution_cell, Arc<CodeExecutorClient>);

// HTTP cell - special case, handled by cell_server.rs
pub async fn http_cell() -> Option<Arc<TcpTunnelClient>> {
    tracing::warn!("http_cell not yet implemented for roam");
    None
}

// ============================================================================
// Convenience Functions (wrappers around cell clients)
// ============================================================================

// These will be implemented once cell infrastructure is complete.
// For now they return errors.

pub async fn resize_image(_input: ResizeInput) -> Result<ImageResult, eyre::Error> {
    Err(eyre::eyre!("Image cell not available"))
}

pub async fn encode_webp(_input: WebPEncodeInput) -> Result<WebPResult, eyre::Error> {
    Err(eyre::eyre!("WebP cell not available"))
}

pub async fn encode_jxl(_input: JXLEncodeInput) -> Result<JXLResult, eyre::Error> {
    Err(eyre::eyre!("JXL cell not available"))
}

pub async fn compute_thumbhash(_input: ThumbhashInput) -> Result<ImageResult, eyre::Error> {
    Err(eyre::eyre!("Image cell not available"))
}

pub async fn parse_markdown(_input: String) -> Result<ParseResult, eyre::Error> {
    Err(eyre::eyre!("Markdown cell not available"))
}

pub async fn render_markdown(_input: String) -> Result<MarkdownResult, eyre::Error> {
    Err(eyre::eyre!("Markdown cell not available"))
}

pub async fn extract_frontmatter(_input: String) -> Result<FrontmatterResult, eyre::Error> {
    Err(eyre::eyre!("Markdown cell not available"))
}

pub async fn minify_html(_input: String) -> Result<MinifyResult, eyre::Error> {
    Err(eyre::eyre!("Minify cell not available"))
}

pub async fn minify_css(_input: String) -> Result<MinifyResult, eyre::Error> {
    Err(eyre::eyre!("Minify cell not available"))
}

pub async fn minify_js(_input: String) -> Result<MinifyResult, eyre::Error> {
    Err(eyre::eyre!("Minify cell not available"))
}

pub async fn process_css(_input: String) -> Result<CssResult, eyre::Error> {
    Err(eyre::eyre!("CSS cell not available"))
}

pub async fn compile_sass(_input: SassInput) -> Result<SassResult, eyre::Error> {
    Err(eyre::eyre!("SASS cell not available"))
}

pub async fn rewrite_js(_input: JsRewriteInput) -> Result<JsResult, eyre::Error> {
    Err(eyre::eyre!("JS cell not available"))
}

pub async fn optimize_svg(_input: String) -> Result<SvgoResult, eyre::Error> {
    Err(eyre::eyre!("SVGO cell not available"))
}

pub async fn analyze_font(_input: Vec<u8>) -> Result<FontAnalysis, eyre::Error> {
    Err(eyre::eyre!("Font cell not available"))
}

pub async fn subset_font(_input: SubsetFontInput) -> Result<FontResult, eyre::Error> {
    Err(eyre::eyre!("Font cell not available"))
}

pub async fn check_links(_input: LinkCheckInput) -> Result<LinkCheckResult, eyre::Error> {
    Err(eyre::eyre!("Link check cell not available"))
}

pub async fn diff_html(_input: DiffInput) -> Result<HtmlDiffResult, eyre::Error> {
    Err(eyre::eyre!("HTML diff cell not available"))
}

pub async fn build_search_index(
    _input: SearchIndexInput,
) -> Result<SearchIndexResult, eyre::Error> {
    Err(eyre::eyre!("Pagefind cell not available"))
}

pub async fn execute_code_samples(
    _input: ExecuteSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    Err(eyre::eyre!("Code execution cell not available"))
}

pub async fn extract_code_samples(
    _input: ExtractSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    Err(eyre::eyre!("Code execution cell not available"))
}

// ============================================================================
// Additional Function Aliases (for compatibility with other modules)
// ============================================================================

// These are aliases for the cell accessor and wrapper functions
// that other modules expect.

pub use dialoguer_cell as dialoguer_client;

pub fn has_linkcheck_cell() -> bool {
    false // Cell not available yet
}

/// Result of link checking - wrapper for internal use
#[derive(Debug, Clone)]
pub struct UrlCheckResult {
    pub statuses: Vec<LinkStatus>,
}

pub async fn check_urls_cell(_urls: Vec<String>, _options: CheckOptions) -> Option<UrlCheckResult> {
    tracing::warn!("Link check cell not available");
    None
}

#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    pub timeout_secs: u64,
    pub skip_domains: Vec<String>,
    pub rate_limit_ms: u64,
}

pub async fn parse_and_render_markdown_cell(
    _source_path: &str,
    _content: &str,
) -> Result<cell_markdown_proto::ParseResult, MarkdownParseError> {
    Err(MarkdownParseError {
        message: "Markdown cell not available".to_string(),
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
    _html: String,
    _code_metadata: HashMap<String, cell_html_proto::CodeExecutionMetadata>,
) -> Result<(String, bool), eyre::Error> {
    Err(eyre::eyre!("HTML cell not available"))
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
    _html: String,
    _dead_links: Vec<String>,
) -> Result<String, eyre::Error> {
    Err(eyre::eyre!("HTML cell not available"))
}

pub async fn rewrite_urls_in_html_cell(
    _html: String,
    _rewrites: Vec<(String, String)>,
) -> Result<String, eyre::Error> {
    Err(eyre::eyre!("HTML cell not available"))
}

pub async fn rewrite_string_literals_in_js_cell(
    _js: String,
    _rewrites: Vec<(String, String)>,
) -> Result<String, eyre::Error> {
    Err(eyre::eyre!("JS cell not available"))
}

pub async fn rewrite_urls_in_css_cell(
    _css: String,
    _rewrites: Vec<(String, String)>,
) -> Result<String, eyre::Error> {
    Err(eyre::eyre!("CSS cell not available"))
}

pub async fn decompress_font_cell(_data: Vec<u8>) -> Result<Vec<u8>, eyre::Error> {
    Err(eyre::eyre!("Font cell not available"))
}

pub async fn compress_to_woff2_cell(_data: Vec<u8>) -> Result<Vec<u8>, eyre::Error> {
    Err(eyre::eyre!("Font cell not available"))
}

pub async fn subset_font_cell(input: SubsetFontInput) -> Result<FontResult, eyre::Error> {
    subset_font(input).await
}

// Image decoding/encoding cell wrappers
// These return Option to match what image.rs expects
pub async fn decode_png_cell(_data: &[u8]) -> Option<DecodedImage> {
    tracing::warn!("Image cell not available");
    None
}

pub async fn decode_jpeg_cell(_data: &[u8]) -> Option<DecodedImage> {
    tracing::warn!("Image cell not available");
    None
}

pub async fn decode_gif_cell(_data: &[u8]) -> Option<DecodedImage> {
    tracing::warn!("Image cell not available");
    None
}

pub async fn decode_webp_cell(_data: &[u8]) -> Option<DecodedImage> {
    tracing::warn!("Image cell not available");
    None
}

pub async fn decode_jxl_cell(_data: &[u8]) -> Option<DecodedImage> {
    tracing::warn!("Image cell not available");
    None
}

pub async fn resize_image_cell(
    _pixels: &[u8],
    _width: u32,
    _height: u32,
    _channels: u8,
    _target_width: u32,
) -> Option<DecodedImage> {
    tracing::warn!("Image cell not available");
    None
}

pub async fn generate_thumbhash_cell(_pixels: &[u8], _width: u32, _height: u32) -> Option<String> {
    tracing::warn!("Image cell not available");
    None
}

pub async fn encode_webp_cell(
    _pixels: &[u8],
    _width: u32,
    _height: u32,
    _quality: u8,
) -> Option<Vec<u8>> {
    tracing::warn!("WebP cell not available");
    None
}

pub async fn encode_jxl_cell(
    _pixels: &[u8],
    _width: u32,
    _height: u32,
    _quality: u8,
) -> Option<Vec<u8>> {
    tracing::warn!("JXL cell not available");
    None
}

// SASS/CSS cell wrappers
pub async fn compile_sass_cell(
    _input: &HashMap<String, String>,
) -> Result<SassResult, eyre::Error> {
    Err(eyre::eyre!("SASS cell not available"))
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
pub async fn extract_css_from_html_cell(_html: &str) -> Result<String, eyre::Error> {
    Err(eyre::eyre!("HTML cell not available"))
}

// Font analysis
pub async fn analyze_fonts_cell(_html: &str, _css: &str) -> Result<FontAnalysis, eyre::Error> {
    Err(eyre::eyre!("Font cell not available"))
}

// TUI cell spawning (placeholder)
pub fn spawn_cell_with_dispatcher<D>(
    _binary_path: &Path,
    _binary_name: &str,
    _dispatcher: D,
    _config: &CellSpawnConfig,
) -> Option<SpawnedCellResult>
where
    D: ServiceDispatcher + 'static,
{
    tracing::warn!("spawn_cell_with_dispatcher not yet implemented for roam");
    None
}
