//! Cell loading and management for dodeca.
//!
//! Cells are loaded from dynamic libraries (.so on Linux, .dylib on macOS).
//! Currently supports image encoding/decoding cells (WebP, JXL).
//!
//! # Hub Architecture
//!
//! All cells share a single SHM "hub" file with variable-size slot allocation.
//! Each cell gets its own ring pair within the hub and communicates via
//! socketpair doorbells.

use crate::cell_server::ForwardingTracingSink;

use cell_code_execution_proto::{
    CodeExecutionResult, CodeExecutorClient, ExecuteSamplesInput, ExtractSamplesInput,
};
use cell_css_proto::{CssProcessorClient, CssResult};
use cell_dialoguer_proto::DialoguerClient;
use cell_fonts_proto::{FontAnalysis, FontProcessorClient, FontResult, SubsetFontInput};
use cell_gingembre_proto::{
    ContextId, EvalResult, RenderResult, TemplateHostServer, TemplateRendererClient,
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
use rapace::transport::shm::{AddPeerOptions, HubConfig, HubHost};
use rapace::{AnyTransport, Frame, RpcError, RpcSession, Session};
use rapace_cell::{CellLifecycle, CellLifecycleServer, ReadyAck, ReadyMsg};
use rapace_cell::{DispatcherBuilder, ServiceDispatch};
use rapace_tracing::{TracingConfigClient, TracingSinkServer};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tracing::{debug, info, warn};
/// Global hub host for all cells.
static HUB: OnceLock<Arc<HubHost>> = OnceLock::new();

/// Hub SHM path.
static HUB_PATH: OnceLock<PathBuf> = OnceLock::new();

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
///
/// Cells call the CellLifecycle.ready() RPC after starting their demux loop,
/// proving they can handle RPC requests.
#[derive(Clone)]
pub struct CellReadyRegistry {
    /// Map of peer_id -> ReadyMsg
    ready: Arc<DashMap<u16, ReadyMsg>>,
}

impl CellReadyRegistry {
    fn new() -> Self {
        Self {
            ready: Arc::new(DashMap::new()),
        }
    }

    /// Mark a peer as ready
    fn mark_ready(&self, msg: ReadyMsg) {
        let peer_id = msg.peer_id;
        self.ready.insert(peer_id, msg);
    }

    /// Check if a peer is ready
    pub fn is_ready(&self, peer_id: u16) -> bool {
        self.ready.contains_key(&peer_id)
    }

    /// Wait for multiple peers to become ready with timeout
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

/// Global cell readiness registry
static CELL_READY_REGISTRY: OnceLock<CellReadyRegistry> = OnceLock::new();

/// Get or initialize the cell readiness registry
pub fn cell_ready_registry() -> &'static CellReadyRegistry {
    CELL_READY_REGISTRY.get_or_init(CellReadyRegistry::new)
}

/// Wait for multiple cells to become ready by name with timeout
pub async fn wait_for_cells_ready(cell_names: &[&str], timeout: Duration) -> eyre::Result<()> {
    let mut peer_ids = Vec::new();

    // Look up all peer IDs
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

    // Wait for all cells to become ready
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

/// Decoded image data returned by cells
pub type DecodedImage = cell_image_proto::DecodedImage;

/// Global cell registry, initialized once (async).
static CELLS: tokio::sync::OnceCell<CellRegistry> = tokio::sync::OnceCell::const_new();

/// Initialize cells and wait for ALL of them to complete their readiness handshake.
///
/// This must be called ONCE at startup before any RPC calls are made.
/// All cells will send a `CellLifecycle.ready()` RPC after starting their demux loop,
/// proving they can handle requests. This function waits for all of them.
pub async fn init_and_wait_for_cells() -> eyre::Result<()> {
    use std::time::Duration;

    // Trigger cell loading - this spawns all cells and their RPC sessions
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

    // Wait for all cells to complete their readiness handshake
    let timeout = Duration::from_secs(10);
    cell_ready_registry()
        .wait_for_all_ready(&peer_ids, timeout)
        .await?;

    // Push current host filter to cells (now that they're ready and the hub is not contended).
    push_tracing_filter_to_cells().await;

    debug!("All {} cells ready", peer_ids.len());
    Ok(())
}

async fn push_tracing_filter_to_cells() {
    let filter_str = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| crate::logging::DEFAULT_TRACING_FILTER.to_string());

    let Ok(peers) = PEER_DIAG_INFO.read() else {
        warn!("Failed to acquire peer info lock; skipping tracing filter push");
        return;
    };

    for peer in peers.iter() {
        let rpc_session = peer.rpc_session.clone();
        let cell_label = peer.name.clone();
        let filter_str = filter_str.clone();
        tokio::spawn(async move {
            let tracing_config_client = TracingConfigClient::new(rpc_session);
            if let Err(e) = tracing_config_client.set_filter(filter_str.clone()).await {
                warn!("Failed to push filter to {} cell: {:?}", cell_label, e);
            } else {
                debug!("Pushed filter to {} cell: {}", cell_label, filter_str);
            }
        });
    }
}

/// Peer info for diagnostics
struct PeerDiagInfo {
    peer_id: u16,
    name: String,
    rpc_session: Arc<Session>,
}

/// Temporary wrapper for TracingSinkServer to implement ServiceDispatch.
///
/// FIXME: This is a workaround for rapace PR #105's broken crate detection.
/// rapace-tracing doesn't get auto-generated wrappers because it depends on
/// `rapace` (umbrella crate) not `rapace-cell` directly, so the macro's
/// `crate_name("rapace-cell")` check fails.
///
/// Tracking issue: <https://github.com/bearcove/rapace/issues/107>
/// Once rapace implements blanket `impl<T: Dispatchable> ServiceDispatch for Arc<T>`,
/// this wrapper can be deleted.
struct TracingSinkService(Arc<TracingSinkServer<ForwardingTracingSink>>);

impl ServiceDispatch for TracingSinkService {
    fn method_ids(&self) -> &'static [u32] {
        use rapace_tracing::{
            TRACING_SINK_METHOD_ID_DROP_SPAN, TRACING_SINK_METHOD_ID_ENTER,
            TRACING_SINK_METHOD_ID_EVENT, TRACING_SINK_METHOD_ID_EXIT,
            TRACING_SINK_METHOD_ID_NEW_SPAN, TRACING_SINK_METHOD_ID_RECORD,
        };
        &[
            TRACING_SINK_METHOD_ID_NEW_SPAN,
            TRACING_SINK_METHOD_ID_RECORD,
            TRACING_SINK_METHOD_ID_EVENT,
            TRACING_SINK_METHOD_ID_ENTER,
            TRACING_SINK_METHOD_ID_EXIT,
            TRACING_SINK_METHOD_ID_DROP_SPAN,
        ]
    }

    fn dispatch(
        &self,
        method_id: u32,
        frame: Frame,
        buffer_pool: &rapace::BufferPool,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Frame, RpcError>> + Send + 'static>,
    > {
        let server = self.0.clone();
        let buffer_pool = buffer_pool.clone();
        Box::pin(async move { server.dispatch(method_id, &frame, &buffer_pool).await })
    }
}

/// Global peer diagnostic info.
static PEER_DIAG_INFO: RwLock<Vec<PeerDiagInfo>> = RwLock::new(Vec::new());

/// Register a peer's transport and RPC session for diagnostics.
fn register_peer_diag(peer_id: u16, name: &str, rpc_session: Arc<Session>) {
    if let Ok(mut info) = PEER_DIAG_INFO.write() {
        info.push(PeerDiagInfo {
            peer_id,
            name: name.to_string(),
            rpc_session,
        });
    }
}

/// Get a cell's RPC session by binary name (e.g., "ddc-cell-http").
/// Returns None if the cell is not loaded.
pub fn get_cell_session(name: &str) -> Option<Arc<Session>> {
    PEER_DIAG_INFO
        .read()
        .ok()?
        .iter()
        .find(|info| info.name == name)
        .map(|info| info.rpc_session.clone())
}

/// Get the global hub and hub path (initializing if needed).
/// Returns None if hub creation fails.
pub async fn get_hub() -> Option<(Arc<HubHost>, PathBuf)> {
    // Ensure all() is called to initialize the hub
    let _ = all().await;
    let hub = HUB.get()?.clone();
    let hub_path = HUB_PATH.get()?.clone();
    Some((hub, hub_path))
}

// ============================================================================
// Unified Cell Spawning
// ============================================================================

/// Configuration for spawning a cell.
pub struct CellSpawnConfig {
    /// Whether to inherit stdin/stdout/stderr (for terminal cells like TUI).
    pub inherit_stdio: bool,
    /// Whether to manage the child process internally (spawn wait task).
    /// If false, the Child is returned for the caller to manage.
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

/// Result of spawning a cell.
pub struct SpawnedCellResult {
    pub peer_id: u16,
    pub rpc_session: Arc<Session>,
    /// Only Some if `manage_child` was false in config.
    pub child: Option<tokio::process::Child>,
}

/// Core cell spawning function that handles all the common boilerplate.
///
/// This function:
/// 1. Adds a peer to the hub with death detection
/// 2. Spawns the cell process with appropriate args
/// 3. Sets up stdio capture (unless inheriting)
/// 4. Creates the RPC session and transport
/// 5. Registers for diagnostics
///
/// The caller is responsible for setting up the dispatcher on the returned session.
fn spawn_cell_core(
    binary_path: &Path,
    binary_name: &str,
    hub: &Arc<HubHost>,
    config: &CellSpawnConfig,
) -> Option<SpawnedCellResult> {
    // Add peer to hub with death detection
    let cell_name = binary_name.to_string();
    let (transport, ticket) = match hub.add_peer_transport_with_options(AddPeerOptions {
        peer_name: Some(cell_name.clone()),
        on_death: Some(Arc::new({
            let cell_name = cell_name.clone();
            move |peer_id| {
                eprintln!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                eprintln!(
                    "FATAL: {} cell died unexpectedly (peer_id={})",
                    cell_name, peer_id
                );
                eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                eprintln!();
                eprintln!("The cell's doorbell signal failed, indicating it crashed or hung.");
                eprintln!("Check the logs above for error messages from the cell.");
                eprintln!();
                std::process::exit(1);
            }
        })),
    }) {
        Ok(result) => result,
        Err(e) => {
            warn!("failed to add peer for {}: {}", binary_name, e);
            return None;
        }
    };

    let peer_id = ticket.peer_id;

    // Build command with hub args from ticket
    let mut cmd = Command::new(binary_path);
    cmd.arg(format!("--hub-path={}", ticket.hub_path.display()))
        .arg(format!("--peer-id={}", ticket.peer_id))
        .arg(format!("--doorbell-fd={}", ticket.doorbell_fd));

    if config.inherit_stdio {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    } else {
        cmd.stdin(Stdio::null());
        if is_quiet_mode() {
            cmd.stdout(Stdio::null())
                .stderr(Stdio::null())
                .env("DODECA_QUIET", "1");
        } else {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        }
    }

    let mut child = match ur_taking_me_with_you::spawn_dying_with_parent_async(cmd) {
        Ok(child) => child,
        Err(e) => {
            warn!("failed to spawn {}: {}", binary_name, e);
            return None;
        }
    };

    // Register child PID for SIGUSR1 forwarding
    if let Some(pid) = child.id() {
        dodeca_debug::register_child_pid(pid);
    }

    // Capture stdio unless inheriting
    if !config.inherit_stdio && !is_quiet_mode() {
        capture_cell_stdio(binary_name, &mut child);
    }

    // Drop ticket to close our end of the peer's doorbell (child inherited it)
    drop(ticket);

    // Create RPC session
    let rpc_session = Arc::new(RpcSession::with_channel_start(transport, 1));

    // Register for diagnostics
    register_peer_diag(peer_id, binary_name, rpc_session.clone());

    debug!(
        "spawned {} cell from {} (peer_id={})",
        binary_name,
        binary_path.display(),
        peer_id
    );

    // Optionally manage child process internally
    let child = if config.manage_child {
        let cell_label = binary_name.to_string();
        let hub_for_cleanup = hub.clone();
        tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => {
                    if !status.success() {
                        eprintln!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                        eprintln!("FATAL: {} cell crashed with status: {}", cell_label, status);
                        eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                        eprintln!();
                        eprintln!("A critical cell process has terminated unexpectedly.");
                        eprintln!("Check the logs above for error messages from the cell.");
                        eprintln!();
                        eprintln!("Cell: {}", cell_label);
                        eprintln!("Exit status: {}", status);
                        eprintln!();
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                    eprintln!("FATAL: {} cell wait error: {}", cell_label, e);
                    eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                    std::process::exit(1);
                }
            }
            hub_for_cleanup
                .allocator()
                .reclaim_peer_slots(peer_id as u32);
            info!(
                "{} cell exited, reclaimed slots for peer {}",
                cell_label, peer_id
            );
        });
        None
    } else {
        Some(child)
    };

    Some(SpawnedCellResult {
        peer_id,
        rpc_session,
        child,
    })
}

/// Find a cell binary by name, searching standard locations.
fn find_cell_binary(binary_name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let executable = format!("{binary_name}.exe");
    #[cfg(not(target_os = "windows"))]
    let executable = binary_name.to_string();

    // Try DODECA_CELL_PATH first (consistent with resolve_cell_path)
    if let Ok(env_path) = std::env::var("DODECA_CELL_PATH") {
        let path = PathBuf::from(&env_path).join(&executable);
        if path.exists() {
            return Some(path);
        }
    }

    // Try adjacent to current exe
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(dir) = exe_path.parent() {
            let path = dir.join(&executable);
            if path.exists() {
                return Some(path);
            }
        }
    }

    // Try development fallback
    #[cfg(debug_assertions)]
    let profile_dir = PathBuf::from("target/debug");
    #[cfg(not(debug_assertions))]
    let profile_dir = PathBuf::from("target/release");

    let path = profile_dir.join(&executable);
    if path.exists() {
        return Some(path);
    }

    None
}

/// Start the RPC session runner task.
fn start_session_runner(rpc_session: &Arc<Session>, cell_label: &str) {
    let session_runner = rpc_session.clone();
    let cell_label = cell_label.to_string();
    tokio::spawn(async move {
        if let Err(e) = session_runner.run().await {
            warn!("{} cell RPC session error: {}", cell_label, e);
        }
    });
}

/// Create the standard dispatcher with TracingSink and CellLifecycle services.
fn create_standard_dispatcher()
-> impl Fn(Frame) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Frame, RpcError>> + Send>>
+ Send
+ Sync
+ 'static {
    let tracing_sink = ForwardingTracingSink::new();
    let lifecycle_impl = HostCellLifecycle::new(cell_ready_registry().clone());

    let buffer_pool = rapace::BufferPool::with_capacity(128, 256 * 1024);
    DispatcherBuilder::new()
        .add_service(TracingSinkService(Arc::new(TracingSinkServer::new(
            tracing_sink,
        ))))
        .add_service(CellLifecycleServer::new(lifecycle_impl).into_dispatch())
        .build(buffer_pool)
}

/// Spawn a cell binary through the hub with a custom dispatcher.
///
/// This is used for cells like TUI that need a custom dispatcher rather than
/// just being clients that we call methods on.
///
/// Set `inherit_stdio` to `true` for cells that need direct terminal access (like TUI).
/// When true, stdin/stdout/stderr are inherited from the parent process, allowing
/// the cell to interact with the terminal directly.
pub async fn spawn_cell_with_dispatcher<D>(
    binary_name: &str,
    dispatcher_factory: impl FnOnce(Arc<Session>) -> D,
    inherit_stdio: bool,
) -> Option<(Arc<Session>, tokio::process::Child)>
where
    D: Fn(
            Frame,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Frame, rapace::RpcError>> + Send>,
        > + Send
        + Sync
        + 'static,
{
    let (hub, _hub_path) = get_hub().await?;

    let binary_path = find_cell_binary(binary_name)?;

    let result = spawn_cell_core(
        &binary_path,
        binary_name,
        &hub,
        &CellSpawnConfig {
            inherit_stdio,
            manage_child: false, // We return the child
        },
    )?;

    // Set up the custom dispatcher combined with CellLifecycle service
    let custom_dispatcher = dispatcher_factory(result.rpc_session.clone());
    let lifecycle_impl = HostCellLifecycle::new(cell_ready_registry().clone());
    let lifecycle_service: Arc<dyn ServiceDispatch> =
        Arc::new(CellLifecycleServer::new(lifecycle_impl).into_dispatch());
    let lifecycle_method_ids = lifecycle_service.method_ids();
    let buffer_pool = result.rpc_session.buffer_pool().clone();

    let combined_dispatcher = move |request: Frame| {
        let method_id = request.desc.method_id;

        if lifecycle_method_ids.contains(&method_id) {
            let service = lifecycle_service.clone();
            let pool = buffer_pool.clone();
            let channel_id = request.desc.channel_id;
            let msg_id = request.desc.msg_id;
            Box::pin(async move {
                let mut response = service.dispatch(method_id, request, &pool).await?;
                response.desc.channel_id = channel_id;
                response.desc.msg_id = msg_id;
                Ok(response)
            })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<Frame, rapace::RpcError>> + Send>,
                >
        } else {
            custom_dispatcher(request)
        }
    };

    result.rpc_session.set_dispatcher(combined_dispatcher);
    start_session_runner(&result.rpc_session, binary_name);

    info!(
        "launched {} from {} (peer_id={})",
        binary_name,
        binary_path.display(),
        result.peer_id
    );

    Some((result.rpc_session, result.child.expect("child not managed")))
}

/// Dump hub transport diagnostics to stderr (called on SIGUSR1).
fn dump_hub_diagnostics() {
    eprintln!("\n--- Hub Transport Diagnostics ---");

    // Get hub
    let Some(hub) = HUB.get() else {
        eprintln!("  Hub not initialized");
        return;
    };

    // Dump per-peer ring status and pending RPC waiters
    if let Ok(peers) = PEER_DIAG_INFO.read() {
        for peer in peers.iter() {
            let recv_ring = hub.peer_recv_ring(peer.peer_id);
            let send_ring = hub.peer_send_ring(peer.peer_id);
            let pending_ids = peer.rpc_session.pending_channel_ids();
            let tunnel_ids = peer.rpc_session.tunnel_channel_ids();

            eprintln!(
                "  peer[{}] \"{}\": recv_ring({}) send_ring({})",
                peer.peer_id,
                peer.name,
                recv_ring.ring_status(),
                send_ring.ring_status()
            );

            // Show pending RPC waiters if any
            if !pending_ids.is_empty() {
                eprintln!(
                    "    pending_rpcs={} channel_ids={:?}",
                    pending_ids.len(),
                    pending_ids
                );
            }

            // Show active tunnels if any
            if !tunnel_ids.is_empty() {
                eprintln!(
                    "    active_tunnels={} channel_ids={:?}",
                    tunnel_ids.len(),
                    tunnel_ids
                );
            }
        }
    }

    eprintln!("--- End Hub Diagnostics ---\n");
}

/// Initialize hub diagnostics (register SIGUSR1 callback).
/// Call this after the hub is created.
pub fn init_hub_diagnostics() {
    dodeca_debug::register_diagnostic(dump_hub_diagnostics);
}

/// Info about a spawned cell with its RPC session already running.
struct SpawnedCell {
    peer_id: u16,
    rpc_session: Arc<Session>,
}

macro_rules! define_cells {
    ( $( $key:ident => $Client:ident ),* $(,)? ) => {
        #[derive(Default)]
        pub struct CellRegistry {
            $(
                pub $key: Option<Arc<$Client<AnyTransport>>>,
            )*
        }

        impl CellRegistry {
            /// Load cells from a directory.
            ///
            /// Spawns all cells in parallel (with RPC sessions immediately running),
            /// waits for them to register, then creates clients.
            async fn load_from_dir(dir: &Path, hub: &Arc<HubHost>, hub_path: &Path) -> Self {
                // Phase 1: Spawn all cells with RPC sessions already running
                // We yield periodically to give the RPC session tasks a chance to run,
                // which allows them to process incoming messages and free SHM slots.
                let mut spawned: Vec<(&'static str, Option<SpawnedCell>)> = Vec::new();
                let mut spawn_count = 0u32;
                $(
                    let cell_name = match stringify!($key) {
                        "syntax_highlight" => "ddc-cell-arborium".to_string(),
                        other => format!("ddc-cell-{}", other.replace('_', "-")),
                    };
                    let spawn_result = Self::spawn_cell(dir, &cell_name, hub, hub_path);
                    spawned.push((stringify!($key), spawn_result));
                    spawn_count += 1;

                    // Yield every 4 cells to let RPC sessions process messages
                    // This prevents SHM slot exhaustion during parallel startup
                    if spawn_count % 4 == 0 {
                        tokio::task::yield_now().await;
                    }
                )*

                // Final yield to ensure all spawned sessions get a chance to run
                tokio::task::yield_now().await;

                // Phase 2: Wait for all spawned cells to register
                // (RPC sessions are already running, so messages can be processed)
                let peer_ids: Vec<u16> = spawned.iter()
                    .filter_map(|(_, s)| s.as_ref().map(|p| p.peer_id))
                    .collect();
                Self::wait_for_peers(hub, &peer_ids).await;

                // Phase 3: Create clients from already-running sessions
                let mut iter = spawned.into_iter();
                $(
                    let (_, spawn_result) = iter.next().unwrap();
                    let $key = spawn_result
                        .map(|s| Arc::new($Client::new(s.rpc_session)));
                )*

                CellRegistry {
                    $($key),*
                }
            }
        }
    };
}

define_cells! {
    webp            => WebPProcessorClient,
    jxl             => JXLProcessorClient,
    minify          => MinifierClient,
    svgo            => SvgoOptimizerClient,
    sass            => SassCompilerClient,
    css             => CssProcessorClient,
    js              => JsProcessorClient,
    pagefind        => SearchIndexerClient,
    image           => ImageProcessorClient,
    fonts           => FontProcessorClient,
    linkcheck       => LinkCheckerClient,
    code_execution  => CodeExecutorClient,
    html_diff       => HtmlDifferClient,
    html            => HtmlProcessorClient,
    markdown        => MarkdownProcessorClient,
    http            => TcpTunnelClient,
}

impl CellRegistry {
    /// Spawn a cell process and immediately start its RPC session.
    ///
    /// This ensures the host can process incoming messages (like tracing events)
    /// from the cell immediately, preventing slot exhaustion.
    fn spawn_cell(
        dir: &Path,
        binary_name: &str,
        hub: &Arc<HubHost>,
        _hub_path: &Path, // now provided by ticket
    ) -> Option<SpawnedCell> {
        #[cfg(target_os = "windows")]
        let executable = format!("{binary_name}.exe");
        #[cfg(not(target_os = "windows"))]
        let executable = binary_name.to_string();

        let path = dir.join(&executable);
        if !path.exists() {
            debug!("rapace cell not found: {}", path.display());
            return None;
        }

        let result = spawn_cell_core(&path, binary_name, hub, &CellSpawnConfig::default())?;

        // Set up standard dispatcher with tracing and lifecycle services
        result
            .rpc_session
            .set_dispatcher(create_standard_dispatcher());
        start_session_runner(&result.rpc_session, binary_name);

        Some(SpawnedCell {
            peer_id: result.peer_id,
            rpc_session: result.rpc_session,
        })
    }

    /// Wait for all spawned peers to register.
    ///
    /// Async version that properly yields to the tokio runtime, allowing
    /// RPC session tasks to process incoming messages from cells.
    async fn wait_for_peers(hub: &Arc<HubHost>, peer_ids: &[u16]) {
        if peer_ids.is_empty() {
            return;
        }

        debug!(
            "waiting for {} peers to register: {:?}",
            peer_ids.len(),
            peer_ids
        );

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(2);

        loop {
            let all_active = peer_ids.iter().all(|&id| hub.is_peer_active(id));
            if all_active {
                debug!(
                    "Initialized {} cells in {:?}",
                    peer_ids.len(),
                    start.elapsed()
                );
                return;
            }

            if start.elapsed() > timeout {
                // Log which peers failed to register
                for &peer_id in peer_ids {
                    if !hub.is_peer_active(peer_id) {
                        warn!("peer {} failed to register within timeout", peer_id);
                    }
                }
                return;
            }

            // Yield to tokio runtime so RPC session tasks can process messages
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

/// Initialize the hub SHM file.
fn init_hub() -> Option<(Arc<HubHost>, PathBuf)> {
    let hub_path = env::temp_dir().join(format!("dodeca-hub-{}.shm", std::process::id()));
    let _ = std::fs::remove_file(&hub_path);

    match HubHost::create(&hub_path, HubConfig::default()) {
        Ok(hub) => {
            debug!("created hub SHM at {}", hub_path.display());
            Some((Arc::new(hub), hub_path))
        }
        Err(e) => {
            warn!("failed to create hub SHM: {}", e);
            None
        }
    }
}

/// Cell path resolution source
#[derive(Debug, Clone, Copy)]
enum CellPathSource {
    /// DODECA_CELL_PATH environment variable (highest priority, exclusive)
    EnvVar,
    /// Directory adjacent to the executable
    AdjacentToExe,
    /// "cells/" subdirectory next to executable
    CellsSubdir,
    /// Development fallback (target/debug or target/release)
    DevFallback,
}

/// Check if we're running in development mode (executable is in a target/ directory
/// within a Cargo workspace).
///
/// Returns the workspace root if we're in dev mode, None otherwise.
#[cfg(debug_assertions)]
fn detect_dev_mode() -> Option<PathBuf> {
    let exe_path = std::env::current_exe().ok()?;
    let exe_path_str = exe_path.to_string_lossy();

    // Check if we're in a target/debug or target/release directory
    if !exe_path_str.contains("/target/debug/") && !exe_path_str.contains("/target/release/") {
        return None;
    }

    // Walk up from target/debug to find workspace root
    let mut dir = exe_path.as_path();
    loop {
        if let Some(parent) = dir.parent() {
            let cargo_toml = parent.join("Cargo.toml");
            if cargo_toml.exists() {
                // Check if it's a workspace root
                if let Ok(contents) = std::fs::read_to_string(&cargo_toml) {
                    if contents.contains("[workspace]") {
                        return Some(parent.to_path_buf());
                    }
                }
            }
            dir = parent;
        } else {
            return None;
        }
    }
}

/// Rebuild all binaries when running in development mode.
///
/// This ensures cell binaries are always in sync with the main binary,
/// avoiding version mismatches that can cause deserialization errors.
#[cfg(debug_assertions)]
fn dev_rebuild_cells(workspace_root: &Path) {
    use std::process::Command as StdCommand;

    info!(
        workspace = %workspace_root.display(),
        "Development mode: rebuilding cell binaries to ensure version consistency"
    );

    // Run cargo build --bins
    let status = StdCommand::new("cargo")
        .arg("build")
        .arg("--bins")
        .current_dir(workspace_root)
        .status();

    match status {
        Ok(s) if s.success() => {
            debug!("dev rebuild: cargo build --bins succeeded");
        }
        Ok(s) => {
            warn!("dev rebuild: cargo build --bins failed with status {}", s);
        }
        Err(e) => {
            warn!("dev rebuild: failed to run cargo: {}", e);
        }
    }
}

impl std::fmt::Display for CellPathSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EnvVar => write!(f, "env:DODECA_CELL_PATH"),
            Self::AdjacentToExe => write!(f, "adjacent"),
            Self::CellsSubdir => write!(f, "cells_subdir"),
            Self::DevFallback => write!(f, "dev_fallback"),
        }
    }
}

/// Resolve the cell directory path with clear precedence.
///
/// Precedence (DODECA_CELL_PATH is exclusive if set):
/// 1. DODECA_CELL_PATH env var - if set, this is the ONLY source
/// 2. Directory next to the executable
/// 3. "cells/" subdirectory next to executable
/// 4. Development fallback (target/debug or target/release)
fn resolve_cell_directory() -> Option<(PathBuf, CellPathSource)> {
    // In development mode (debug build running from target/ in a workspace),
    // rebuild all binaries to ensure version consistency. This prevents issues
    // where the main binary and cells have mismatched serialization formats.
    #[cfg(debug_assertions)]
    if let Some(workspace_root) = detect_dev_mode() {
        dev_rebuild_cells(&workspace_root);
    }

    // If DODECA_CELL_PATH is set, use it exclusively - don't fall back
    if let Ok(env_path) = std::env::var("DODECA_CELL_PATH") {
        let path = PathBuf::from(&env_path);
        info!(
            cell_path = %path.display(),
            source = %CellPathSource::EnvVar,
            "Cell directory resolved (exclusive, no fallback)"
        );
        return Some((path, CellPathSource::EnvVar));
    }

    // Otherwise, search through fallback paths
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    // Try adjacent to executable
    if let Some(ref dir) = exe_dir {
        // Check if ddc-cell-http exists here
        let test_binary = dir.join("ddc-cell-http");
        if test_binary.exists() {
            info!(
                cell_path = %dir.display(),
                source = %CellPathSource::AdjacentToExe,
                "Cell directory resolved"
            );
            return Some((dir.clone(), CellPathSource::AdjacentToExe));
        }
    }

    // Try cells/ subdirectory
    if let Some(ref dir) = exe_dir {
        let cells_dir = dir.join("cells");
        let test_binary = cells_dir.join("ddc-cell-http");
        if test_binary.exists() {
            info!(
                cell_path = %cells_dir.display(),
                source = %CellPathSource::CellsSubdir,
                "Cell directory resolved"
            );
            return Some((cells_dir, CellPathSource::CellsSubdir));
        }
    }

    // Development fallback
    #[cfg(debug_assertions)]
    let profile_dir = PathBuf::from("target/debug");
    #[cfg(not(debug_assertions))]
    let profile_dir = PathBuf::from("target/release");

    let test_binary = profile_dir.join("ddc-cell-http");
    if test_binary.exists() {
        info!(
            cell_path = %profile_dir.display(),
            source = %CellPathSource::DevFallback,
            "Cell directory resolved"
        );
        return Some((profile_dir, CellPathSource::DevFallback));
    }

    // No cells found anywhere
    warn!(
        exe_dir = ?exe_dir,
        "No cell directory found in any search location"
    );
    None
}

/// Get the global cell registry, initializing it if needed.
pub async fn all() -> &'static CellRegistry {
    CELLS
        .get_or_init(|| async {
            // Create hub first
            let (hub, hub_path) = match init_hub() {
                Some((h, p)) => {
                    // Store in global statics for later access/cleanup
                    let _ = HUB.set(h.clone());
                    let _ = HUB_PATH.set(p.clone());
                    // Register diagnostic callback for SIGUSR1
                    init_hub_diagnostics();
                    (h, p)
                }
                None => {
                    warn!("hub creation failed, cells will not be available");
                    return Default::default();
                }
            };

            // Resolve cell directory with explicit precedence
            let Some((cell_dir, source)) = resolve_cell_directory() else {
                warn!("No cell directory resolved, cells will not be available");
                return Default::default();
            };

            // Load from the resolved directory
            let registry = CellRegistry::load_from_dir(&cell_dir, &hub, &hub_path).await;

            // Log what was loaded
            let loaded_count = [
                registry.webp.is_some(),
                registry.jxl.is_some(),
                registry.minify.is_some(),
                registry.svgo.is_some(),
                registry.sass.is_some(),
                registry.css.is_some(),
                registry.js.is_some(),
                registry.pagefind.is_some(),
                registry.image.is_some(),
                registry.fonts.is_some(),
                registry.linkcheck.is_some(),
                registry.code_execution.is_some(),
                registry.html_diff.is_some(),
                registry.html.is_some(),
                registry.markdown.is_some(),
                registry.http.is_some(),
            ]
            .iter()
            .filter(|&&b| b)
            .count();

            if loaded_count > 0 {
                debug!(
                    cell_dir = %cell_dir.display(),
                    source = %source,
                    loaded_count,
                    http = registry.http.is_some(),
                    markdown = registry.markdown.is_some(),
                    "Cells loaded"
                );
            } else {
                warn!(
                    cell_dir = %cell_dir.display(),
                    source = %source,
                    "No cells found in resolved directory"
                );
            }

            registry
        })
        .await
}

/// Encode RGBA pixels to WebP using the cell if available, otherwise return None.
#[tracing::instrument(level = "debug", skip(pixels), fields(pixels_len = pixels.len()))]
pub async fn encode_webp_cell(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Option<Vec<u8>> {
    let cell = all().await.webp.as_ref()?;

    let input = WebPEncodeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };

    match cell.encode_webp(input).await {
        Ok(WebPResult::EncodeSuccess { data }) => Some(data),
        Ok(WebPResult::Error { message }) => {
            warn!("webp cell error: {}", message);
            None
        }
        Ok(WebPResult::DecodeSuccess { .. }) => {
            warn!("webp cell returned decode result for encode operation");
            None
        }
        Err(e) => {
            warn!("webp cell call failed: {:?}", e);
            None
        }
    }
}

/// Encode RGBA pixels to JXL using the cell if available, otherwise return None.
#[tracing::instrument(level = "debug", skip(pixels), fields(pixels_len = pixels.len()))]
pub async fn encode_jxl_cell(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Option<Vec<u8>> {
    let cell = all().await.jxl.as_ref()?;

    let input = JXLEncodeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };

    match cell.encode_jxl(input).await {
        Ok(JXLResult::EncodeSuccess { data }) => Some(data),
        Ok(JXLResult::Error { message }) => {
            warn!("jxl cell error: {}", message);
            None
        }
        Ok(JXLResult::DecodeSuccess { .. }) => {
            warn!("jxl cell returned decode result for encode operation");
            None
        }
        Err(e) => {
            warn!("jxl cell call failed: {:?}", e);
            None
        }
    }
}

/// Decode WebP to pixels using the cell.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_webp_cell(data: &[u8]) -> Option<DecodedImage> {
    let cell = all().await.webp.as_ref()?;

    match cell.decode_webp(data.to_vec()).await {
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
            warn!("webp decode cell error: {}", message);
            None
        }
        Ok(WebPResult::EncodeSuccess { .. }) => {
            warn!("webp cell returned encode result for decode operation");
            None
        }
        Err(e) => {
            warn!("webp decode cell call failed: {:?}", e);
            None
        }
    }
}

/// Decode JXL to pixels using the cell.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_jxl_cell(data: &[u8]) -> Option<DecodedImage> {
    let cell = all().await.jxl.as_ref()?;

    match cell.decode_jxl(data.to_vec()).await {
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
            warn!("jxl decode cell error: {}", message);
            None
        }
        Ok(JXLResult::EncodeSuccess { .. }) => {
            warn!("jxl cell returned encode result for decode operation");
            None
        }
        Err(e) => {
            warn!("jxl decode cell call failed: {:?}", e);
            None
        }
    }
}

/// Minify HTML using the cell.
///
/// # Panics
/// Returns an error if the minify cell is not loaded (graceful degradation).
#[tracing::instrument(level = "debug", skip(html), fields(html_len = html.len()))]
pub async fn minify_html_cell(html: &str) -> Result<String, String> {
    let Some(cell) = all().await.minify.as_ref() else {
        return Err("minify cell not loaded".to_string());
    };

    match cell.minify_html(html.to_string()).await {
        Ok(MinifyResult::Success { content }) => Ok(content),
        Ok(MinifyResult::Error { message }) => Err(message),
        Err(e) => Err(format!("cell call failed: {:?}", e)),
    }
}

/// Optimize SVG using the cell.
///
/// Returns an error if the svgo cell is not loaded (graceful degradation).
#[tracing::instrument(level = "debug", skip(svg), fields(svg_len = svg.len()))]
pub async fn optimize_svg_cell(svg: &str) -> Result<String, String> {
    let Some(cell) = all().await.svgo.as_ref() else {
        return Err("svgo cell not loaded".to_string());
    };

    match cell.optimize_svg(svg.to_string()).await {
        Ok(SvgoResult::Success { svg }) => Ok(svg),
        Ok(SvgoResult::Error { message }) => Err(message),
        Err(e) => Err(format!("cell call failed: {:?}", e)),
    }
}

/// Compile SASS/SCSS using the cell.
///
/// Returns an error if the sass cell is not loaded (graceful degradation).
#[tracing::instrument(level = "debug", skip(files), fields(num_files = files.len()))]
pub async fn compile_sass_cell(
    files: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    let Some(cell) = all().await.sass.as_ref() else {
        return Err("sass cell not loaded".to_string());
    };

    let input = SassInput {
        files: files.clone(),
    };

    match cell.compile_sass(input).await {
        Ok(SassResult::Success { css }) => Ok(css),
        Ok(SassResult::Error { message }) => Err(message),
        Err(e) => Err(format!("cell call failed: {:?}", e)),
    }
}

/// Rewrite URLs in CSS and minify using the cell.
///
/// Returns an error if the css cell is not loaded (graceful degradation).
#[tracing::instrument(level = "debug", skip(css, path_map), fields(css_len = css.len(), path_map_len = path_map.len()))]
pub async fn rewrite_urls_in_css_cell(
    css: &str,
    path_map: &HashMap<String, String>,
) -> Result<String, String> {
    let Some(cell) = all().await.css.as_ref() else {
        return Err("css cell not loaded".to_string());
    };

    match cell
        .rewrite_and_minify(css.to_string(), path_map.clone())
        .await
    {
        Ok(CssResult::Success { css }) => Ok(css),
        Ok(CssResult::Error { message }) => Err(message),
        Err(e) => Err(format!("cell call failed: {:?}", e)),
    }
}

/// Rewrite string literals in JS using the cell.
///
/// Returns an error if the js cell is not loaded (graceful degradation).
#[tracing::instrument(level = "debug", skip(js, path_map), fields(js_len = js.len(), path_map_len = path_map.len()))]
pub async fn rewrite_string_literals_in_js_cell(
    js: &str,
    path_map: &HashMap<String, String>,
) -> Result<String, String> {
    let Some(cell) = all().await.js.as_ref() else {
        return Err("js cell not loaded".to_string());
    };

    let input = JsRewriteInput {
        js: js.to_string(),
        path_map: path_map.clone(),
    };

    match cell.rewrite_string_literals(input).await {
        Ok(JsResult::Success { js }) => Ok(js),
        Ok(JsResult::Error { message }) => Err(message),
        Err(e) => Err(format!("cell call failed: {:?}", e)),
    }
}

/// Build a search index from HTML pages using the cell.
///
/// Returns an error if the pagefind cell is not loaded (graceful degradation).
#[tracing::instrument(level = "debug", skip(pages), fields(num_pages = pages.len()))]
pub async fn build_search_index_cell(pages: Vec<SearchPage>) -> Result<Vec<SearchFile>, String> {
    let Some(cell) = all().await.pagefind.as_ref() else {
        return Err("pagefind cell not loaded".to_string());
    };

    let input = SearchIndexInput { pages };

    match cell.build_search_index(input).await {
        Ok(SearchIndexResult::Success { output }) => Ok(output.files),
        Ok(SearchIndexResult::Error { message }) => Err(message),
        Err(e) => Err(format!("cell call failed: {:?}", e)),
    }
}

// ============================================================================
// Image processing cell functions
// ============================================================================

/// Decode a PNG image using the cell.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_png_cell(data: &[u8]) -> Option<DecodedImage> {
    let cell = all().await.image.as_ref()?;

    match cell.decode_png(data.to_vec()).await {
        Ok(ImageResult::Success { image }) => Some(DecodedImage {
            pixels: image.pixels,
            width: image.width,
            height: image.height,
            channels: image.channels,
        }),
        Ok(ImageResult::Error { message }) => {
            warn!("png decode cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("png cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("png decode cell call failed: {:?}", e);
            None
        }
    }
}

/// Decode a JPEG image using the cell.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_jpeg_cell(data: &[u8]) -> Option<DecodedImage> {
    let cell = all().await.image.as_ref()?;

    match cell.decode_jpeg(data.to_vec()).await {
        Ok(ImageResult::Success { image }) => Some(DecodedImage {
            pixels: image.pixels,
            width: image.width,
            height: image.height,
            channels: image.channels,
        }),
        Ok(ImageResult::Error { message }) => {
            warn!("jpeg decode cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("jpeg cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("jpeg decode cell call failed: {:?}", e);
            None
        }
    }
}

/// Decode a GIF image using the cell.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_gif_cell(data: &[u8]) -> Option<DecodedImage> {
    let cell = all().await.image.as_ref()?;

    match cell.decode_gif(data.to_vec()).await {
        Ok(ImageResult::Success { image }) => Some(DecodedImage {
            pixels: image.pixels,
            width: image.width,
            height: image.height,
            channels: image.channels,
        }),
        Ok(ImageResult::Error { message }) => {
            warn!("gif decode cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("gif cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("gif decode cell call failed: {:?}", e);
            None
        }
    }
}

/// Resize an image using the cell.
#[tracing::instrument(level = "debug", skip(pixels), fields(pixels_len = pixels.len()))]
pub async fn resize_image_cell(
    pixels: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    target_width: u32,
) -> Option<DecodedImage> {
    let cell = all().await.image.as_ref()?;

    let input = ResizeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        channels,
        target_width,
    };

    match cell.resize_image(input).await {
        Ok(ImageResult::Success { image }) => Some(DecodedImage {
            pixels: image.pixels,
            width: image.width,
            height: image.height,
            channels: image.channels,
        }),
        Ok(ImageResult::Error { message }) => {
            warn!("resize cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("resize cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("resize cell call failed: {:?}", e);
            None
        }
    }
}

/// Generate a thumbhash data URL using the cell.
#[tracing::instrument(level = "debug", skip(pixels), fields(pixels_len = pixels.len()))]
pub async fn generate_thumbhash_cell(pixels: &[u8], width: u32, height: u32) -> Option<String> {
    let cell = all().await.image.as_ref()?;

    let input = ThumbhashInput {
        pixels: pixels.to_vec(),
        width,
        height,
    };

    match cell.generate_thumbhash_data_url(input).await {
        Ok(ImageResult::ThumbhashSuccess { data_url }) => Some(data_url),
        Ok(ImageResult::Error { message }) => {
            warn!("thumbhash cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("thumbhash cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("thumbhash cell call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// Font processing cell functions
// ============================================================================

/// Analyze HTML and CSS to collect font usage information.
///
/// Returns `None` if the fonts cell is not loaded or on error (graceful degradation).
pub async fn analyze_fonts_cell(html: &str, css: &str) -> Option<FontAnalysis> {
    let cell = all().await.fonts.as_ref()?;

    match cell.analyze_fonts(html.to_string(), css.to_string()).await {
        Ok(FontResult::AnalysisSuccess { analysis }) => Some(analysis),
        Ok(FontResult::Error { message }) => {
            warn!("font analysis cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("font analysis cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("font analysis cell call failed: {:?}", e);
            None
        }
    }
}

/// Extract inline CSS from HTML (from `<style>` tags).
///
/// Returns `None` if the fonts cell is not loaded or on error (graceful degradation).
pub async fn extract_css_from_html_cell(html: &str) -> Option<String> {
    tracing::debug!(
        "[HOST] extract_css_from_html_cell: START (html_len={})",
        html.len()
    );
    let cell = all().await.fonts.as_ref()?;

    tracing::debug!("[HOST] extract_css_from_html_cell: calling RPC...");
    match cell.extract_css_from_html(html.to_string()).await {
        Ok(FontResult::CssSuccess { css }) => {
            tracing::debug!(
                "[HOST] extract_css_from_html_cell: SUCCESS (css_len={})",
                css.len()
            );
            Some(css)
        }
        Ok(FontResult::Error { message }) => {
            warn!("extract css cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("extract css cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("extract css cell call failed: {:?}", e);
            None
        }
    }
}

/// Decompress a WOFF2/WOFF font to TTF.
pub async fn decompress_font_cell(data: &[u8]) -> Option<Vec<u8>> {
    let cell = all().await.fonts.as_ref()?;

    match cell.decompress_font(data.to_vec()).await {
        Ok(FontResult::DecompressSuccess { data }) => Some(data),
        Ok(FontResult::Error { message }) => {
            warn!("decompress font cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("decompress font cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("decompress font cell call failed: {:?}", e);
            None
        }
    }
}

/// Subset a font to only include specified characters.
pub async fn subset_font_cell(data: &[u8], chars: &[char]) -> Option<Vec<u8>> {
    let cell = all().await.fonts.as_ref()?;

    let input = SubsetFontInput {
        data: data.to_vec(),
        chars: chars.to_vec(),
    };

    match cell.subset_font(input).await {
        Ok(FontResult::SubsetSuccess { data }) => Some(data),
        Ok(FontResult::Error { message }) => {
            warn!("subset font cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("subset font cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("subset font cell call failed: {:?}", e);
            None
        }
    }
}

/// Compress TTF font data to WOFF2.
pub async fn compress_to_woff2_cell(data: &[u8]) -> Option<Vec<u8>> {
    let cell = all().await.fonts.as_ref()?;

    match cell.compress_to_woff2(data.to_vec()).await {
        Ok(FontResult::CompressSuccess { data }) => Some(data),
        Ok(FontResult::Error { message }) => {
            warn!("compress to woff2 cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("compress to woff2 cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("compress to woff2 cell call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// Link checking cell functions
// ============================================================================

/// Status of an external link check (from cell)
/// Options for link checking
#[derive(Debug, Clone)]
pub struct CheckOptions {
    /// Domains to skip (e.g., ["localhost", "127.0.0.1"])
    #[allow(dead_code)] // TODO: pass to cell
    pub skip_domains: Vec<String>,
    /// Rate limiting between requests (milliseconds)
    pub rate_limit_ms: u64,
    /// Timeout for each request (seconds)
    pub timeout_secs: u64,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            skip_domains: vec!["localhost".to_string(), "127.0.0.1".to_string()],
            rate_limit_ms: 1000,
            timeout_secs: 10,
        }
    }
}

/// Result of link checking
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Status for each URL (in same order as input)
    pub statuses: Vec<LinkStatus>,
    /// Number of URLs that were actually checked (not skipped)
    pub checked_count: u32,
}

/// Check external URLs using the linkcheck cell.
///
/// Returns None if the cell is not loaded.
pub async fn check_urls_cell(urls: Vec<String>, options: CheckOptions) -> Option<CheckResult> {
    let cell = all().await.linkcheck.as_ref()?;

    let input = LinkCheckInput {
        urls: urls.clone(),
        delay_ms: options.rate_limit_ms,
        timeout_secs: options.timeout_secs,
    };

    match cell.check_links(input).await {
        Ok(LinkCheckResult::Success { output }) => {
            // Convert the HashMap results back to the expected format
            let statuses: Vec<LinkStatus> = urls
                .into_iter()
                .map(|url| {
                    output.results.get(&url).cloned().unwrap_or(LinkStatus {
                        status: "skipped".to_string(),
                        code: None,
                        message: Some("URL not in results".to_string()),
                    })
                })
                .collect();

            let checked_count = output.results.len() as u32;

            Some(CheckResult {
                statuses,
                checked_count,
            })
        }
        Ok(LinkCheckResult::Error { message }) => {
            warn!("linkcheck cell error: {}", message);
            None
        }
        Err(e) => {
            warn!("linkcheck cell call failed: {:?}", e);
            None
        }
    }
}

/// Check if the linkcheck cell is available.
pub async fn has_linkcheck_cell() -> bool {
    all().await.linkcheck.is_some()
}

// ============================================================================
// Code execution cell functions
// ============================================================================

/// Extract code samples from markdown using cell.
///
/// Returns None if cell is not loaded.
pub async fn extract_code_samples_cell(
    content: &str,
    source_path: &str,
) -> Option<Vec<cell_code_execution_proto::CodeSample>> {
    let cell = all().await.code_execution.as_ref()?;

    let input = ExtractSamplesInput {
        source_path: source_path.to_string(),
        content: content.to_string(),
    };

    match cell.extract_code_samples(input).await {
        Ok(CodeExecutionResult::ExtractSuccess { output }) => Some(output.samples),
        Ok(CodeExecutionResult::Error { message }) => {
            warn!("code execution cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("code execution cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("code execution cell call failed: {:?}", e);
            None
        }
    }
}

/// Execute code samples using cell.
///
/// Returns None if cell is not loaded.
pub async fn execute_code_samples_cell(
    samples: Vec<cell_code_execution_proto::CodeSample>,
    config: cell_code_execution_proto::CodeExecutionConfig,
) -> Option<
    Vec<(
        cell_code_execution_proto::CodeSample,
        cell_code_execution_proto::ExecutionResult,
    )>,
> {
    let cell = all().await.code_execution.as_ref()?;

    tracing::debug!(
        "[HOST] execute_code_samples_cell: START (num_samples={})",
        samples.len()
    );
    let input = ExecuteSamplesInput { samples, config };

    tracing::debug!("[HOST] execute_code_samples_cell: calling RPC...");
    match cell.execute_code_samples(input).await {
        Ok(CodeExecutionResult::ExecuteSuccess { output }) => {
            tracing::debug!(
                "[HOST] execute_code_samples_cell: SUCCESS (num_results={})",
                output.results.len()
            );
            Some(output.results)
        }
        Ok(CodeExecutionResult::Error { message }) => {
            warn!("code execution cell error: {}", message);
            None
        }
        Ok(_) => {
            warn!("code execution cell returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("code execution cell call failed: {:?}", e);
            None
        }
    }
}

/// Diff two HTML documents and produce patches using the cell.
/// Returns None if the cell is not loaded.
pub async fn diff_html_cell(old_html: &str, new_html: &str) -> Option<DiffResult> {
    let cell = all().await.html_diff.as_ref()?;

    let input = DiffInput {
        old_html: old_html.to_string(),
        new_html: new_html.to_string(),
    };

    match cell.diff_html(input).await {
        Ok(HtmlDiffResult::Success { result }) => Some(result),
        Ok(HtmlDiffResult::Error { message }) => {
            warn!("html diff cell error: {}", message);
            None
        }
        Err(e) => {
            warn!("html diff cell call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// HTML processing cell functions
// ============================================================================

/// Rewrite URLs in HTML attributes (href, src, srcset) using the cell.
///
/// Returns None if the html cell is not loaded.
#[tracing::instrument(level = "debug", skip(html, path_map), fields(html_len = html.len(), path_map_len = path_map.len()))]
pub async fn rewrite_urls_in_html_cell(
    html: &str,
    path_map: &HashMap<String, String>,
) -> Option<String> {
    let cell = all().await.html.as_ref()?;

    match cell.rewrite_urls(html.to_string(), path_map.clone()).await {
        Ok(cell_html_proto::HtmlResult::Success { html }) => Some(html),
        Ok(cell_html_proto::HtmlResult::SuccessWithFlag { html, .. }) => Some(html),
        Ok(cell_html_proto::HtmlResult::Error { message }) => {
            warn!("html rewrite_urls cell error: {}", message);
            None
        }
        Err(e) => {
            warn!("html rewrite_urls cell call failed: {:?}", e);
            None
        }
    }
}

/// Mark dead internal links in HTML using the cell.
///
/// Returns (modified_html, had_dead_links) or None if cell not loaded.
#[tracing::instrument(level = "debug", skip(html, known_routes), fields(html_len = html.len(), routes_count = known_routes.len()))]
pub async fn mark_dead_links_cell(
    html: &str,
    known_routes: &std::collections::HashSet<String>,
) -> Option<(String, bool)> {
    let cell = all().await.html.as_ref()?;

    match cell
        .mark_dead_links(html.to_string(), known_routes.clone())
        .await
    {
        Ok(cell_html_proto::HtmlResult::SuccessWithFlag { html, flag }) => Some((html, flag)),
        Ok(cell_html_proto::HtmlResult::Success { html }) => Some((html, false)),
        Ok(cell_html_proto::HtmlResult::Error { message }) => {
            warn!("html mark_dead_links cell error: {}", message);
            None
        }
        Err(e) => {
            warn!("html mark_dead_links cell call failed: {:?}", e);
            None
        }
    }
}

/// Inject copy buttons (and optionally build info buttons) into all pre blocks.
///
/// This is a single-pass operation that adds position:relative inline style
/// and copy/build-info buttons to every pre element.
///
/// Returns (modified_html, had_buttons) or None if cell not loaded.
#[tracing::instrument(level = "debug", skip(html, code_metadata), fields(html_len = html.len(), metadata_count = code_metadata.len()))]
pub async fn inject_code_buttons_cell(
    html: &str,
    code_metadata: &HashMap<String, cell_html_proto::CodeExecutionMetadata>,
) -> Option<(String, bool)> {
    let cell = all().await.html.as_ref()?;

    match cell
        .inject_code_buttons(html.to_string(), code_metadata.clone())
        .await
    {
        Ok(cell_html_proto::HtmlResult::SuccessWithFlag { html, flag }) => Some((html, flag)),
        Ok(cell_html_proto::HtmlResult::Success { html }) => Some((html, false)),
        Ok(cell_html_proto::HtmlResult::Error { message }) => {
            eprintln!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            eprintln!("FATAL: HTML processing failed");
            eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            eprintln!();
            eprintln!("Error: {}", message);
            eprintln!();
            eprintln!("This is typically caused by invalid HTML or a bug in facet-html.");
            eprintln!("Please check the HTML being processed and file an issue if needed.");
            eprintln!();
            std::process::exit(1);
        }
        Err(e) => {
            warn!("html inject_code_buttons cell call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// Markdown processing cell functions
// ============================================================================

/// Parsed markdown result from bearmark
pub struct ParsedMarkdown {
    /// The frontmatter parsed from the content
    pub frontmatter: cell_markdown_proto::Frontmatter,
    /// Fully rendered HTML output (code blocks already highlighted)
    pub html: String,
    /// Extracted headings
    pub headings: Vec<cell_markdown_proto::Heading>,
    /// Rule definitions for specification traceability
    pub reqs: Vec<cell_markdown_proto::ReqDefinition>,
}

/// Error from markdown parsing
#[derive(Debug, Clone, facet::Facet)]
#[repr(u8)]
pub enum MarkdownParseError {
    /// The markdown cell is not available
    CellNotAvailable,
    /// The cell returned an error (e.g., invalid frontmatter)
    ParseError(String),
    /// The cell call itself failed
    CellCallFailed(String),
}

impl std::fmt::Display for MarkdownParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarkdownParseError::CellNotAvailable => write!(f, "markdown cell not available"),
            MarkdownParseError::ParseError(msg) => write!(f, "{}", strip_ansi(msg)),
            MarkdownParseError::CellCallFailed(msg) => write!(f, "cell call failed: {}", msg),
        }
    }
}

impl std::error::Error for MarkdownParseError {}

/// Strip ANSI escape codes from a string (they don't render in logs)
fn strip_ansi(s: &str) -> String {
    // Match ANSI escape sequences: ESC [ ... m (and similar)
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until we hit the end of the escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Skip until we hit a letter (the terminator)
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse and render markdown content using the cell.
///
/// Returns frontmatter, HTML (with placeholders), headings, and code blocks.
/// The caller is responsible for highlighting code blocks and replacing placeholders.
#[tracing::instrument(level = "debug", skip(content), fields(content_len = content.len()))]
pub async fn parse_and_render_markdown_cell(
    source_path: &str,
    content: &str,
) -> Result<ParsedMarkdown, MarkdownParseError> {
    let cell = all()
        .await
        .markdown
        .as_ref()
        .ok_or(MarkdownParseError::CellNotAvailable)?;

    match cell
        .parse_and_render(source_path.to_string(), content.to_string())
        .await
    {
        Ok(ParseResult::Success {
            frontmatter,
            html,
            headings,
            reqs,
        }) => Ok(ParsedMarkdown {
            frontmatter,
            html,
            headings,
            reqs,
        }),
        Ok(ParseResult::Error { message }) => Err(MarkdownParseError::ParseError(message)),
        Err(e) => Err(MarkdownParseError::CellCallFailed(format!("{:?}", e))),
    }
}

/// Render markdown to HTML using the cell (without frontmatter parsing).
#[tracing::instrument(level = "debug", skip(markdown), fields(markdown_len = markdown.len()))]
#[allow(clippy::type_complexity)]
async fn _render_markdown_cell(
    source_path: &str,
    markdown: &str,
) -> Option<(
    String,
    Vec<cell_markdown_proto::Heading>,
    Vec<cell_markdown_proto::ReqDefinition>,
)> {
    let cell = all().await.markdown.as_ref()?;

    match cell
        .render_markdown(source_path.to_string(), markdown.to_string())
        .await
    {
        Ok(MarkdownResult::Success {
            html,
            headings,
            reqs,
        }) => Some((html, headings, reqs)),
        Ok(MarkdownResult::Error { message }) => {
            warn!("markdown render cell error: {}", message);
            None
        }
        Err(e) => {
            warn!("markdown render cell call failed: {:?}", e);
            None
        }
    }
}

/// Parse frontmatter from content using the cell.
#[tracing::instrument(level = "debug", skip(content), fields(content_len = content.len()))]
async fn _parse_frontmatter_cell(
    content: &str,
) -> Option<(cell_markdown_proto::Frontmatter, String)> {
    let cell = all().await.markdown.as_ref()?;

    match cell.parse_frontmatter(content.to_string()).await {
        Ok(FrontmatterResult::Success { frontmatter, body }) => Some((frontmatter, body)),
        Ok(FrontmatterResult::Error { message }) => {
            warn!("markdown parse_frontmatter cell error: {}", message);
            None
        }
        Err(e) => {
            warn!("markdown parse_frontmatter cell call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// Gingembre template rendering cell
// ============================================================================
//
// Gingembre is special: it uses bidirectional RPC where the cell calls back
// to the host for template loading and data resolution. This enables fine-grained
// picante dependency tracking while keeping the template engine in a separate process.

use crate::template_host::{TemplateHostImpl, render_context_registry};

/// Global gingembre cell client.
static GINGEMBRE_CELL: tokio::sync::OnceCell<Arc<TemplateRendererClient<AnyTransport>>> =
    tokio::sync::OnceCell::const_new();

/// Temporary wrapper for TemplateHostServer to implement ServiceDispatch.
///
/// This is needed because rapace's service macro generates a Server type
/// but the ServiceDispatch wrapper generation has crate detection issues.
/// Same pattern as TracingSinkService above.
struct TemplateHostService(Arc<TemplateHostServer<TemplateHostImpl>>);

impl ServiceDispatch for TemplateHostService {
    fn method_ids(&self) -> &'static [u32] {
        use cell_gingembre_proto::{
            TEMPLATE_HOST_METHOD_ID_KEYS_AT, TEMPLATE_HOST_METHOD_ID_LOAD_TEMPLATE,
            TEMPLATE_HOST_METHOD_ID_RESOLVE_DATA,
        };
        &[
            TEMPLATE_HOST_METHOD_ID_LOAD_TEMPLATE,
            TEMPLATE_HOST_METHOD_ID_RESOLVE_DATA,
            TEMPLATE_HOST_METHOD_ID_KEYS_AT,
        ]
    }

    fn dispatch(
        &self,
        method_id: u32,
        frame: Frame,
        buffer_pool: &rapace::BufferPool,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Frame, RpcError>> + Send + 'static>,
    > {
        let server = self.0.clone();
        let buffer_pool = buffer_pool.clone();
        Box::pin(async move { server.dispatch(method_id, &frame, &buffer_pool).await })
    }
}

/// Initialize the gingembre cell with TemplateHost service.
///
/// This must be called after the hub is initialized but before any render calls.
pub async fn init_gingembre_cell() -> Option<()> {
    tracing::info!("init_gingembre_cell: starting");
    let (hub, _hub_path) = get_hub().await.or_else(|| {
        tracing::warn!("init_gingembre_cell: no hub available");
        None
    })?;

    let binary_path = find_cell_binary("ddc-cell-gingembre").or_else(|| {
        tracing::warn!("init_gingembre_cell: could not find ddc-cell-gingembre binary");
        None
    })?;
    let binary_name = "ddc-cell-gingembre";

    let result = spawn_cell_core(&binary_path, binary_name, &hub, &CellSpawnConfig::default())?;

    // Set up dispatcher with TemplateHost in addition to standard services
    let tracing_sink = ForwardingTracingSink::new();
    let lifecycle_impl = HostCellLifecycle::new(cell_ready_registry().clone());
    let template_host_impl = TemplateHostImpl::new(render_context_registry());

    let buffer_pool = rapace::BufferPool::with_capacity(128, 256 * 1024);
    let dispatcher = DispatcherBuilder::new()
        .add_service(TracingSinkService(Arc::new(TracingSinkServer::new(
            tracing_sink,
        ))))
        .add_service(CellLifecycleServer::new(lifecycle_impl).into_dispatch())
        .add_service(TemplateHostService(Arc::new(TemplateHostServer::new(
            template_host_impl,
        ))))
        .build(buffer_pool);

    result.rpc_session.set_dispatcher(dispatcher);
    start_session_runner(&result.rpc_session, binary_name);

    // Create and store the client
    let client = Arc::new(TemplateRendererClient::new(result.rpc_session));
    let _ = GINGEMBRE_CELL.set(client);

    info!(
        "launched gingembre cell from {} (peer_id={})",
        binary_path.display(),
        result.peer_id
    );

    Some(())
}

/// Get the gingembre cell client, if available.
pub async fn gingembre_cell() -> Option<Arc<TemplateRendererClient<AnyTransport>>> {
    GINGEMBRE_CELL.get().cloned()
}

/// Render a template using the gingembre cell.
///
/// # Arguments
/// - `context_id`: The render context ID (from RenderContextGuard)
/// - `template_name`: Name of the template to render
/// - `initial_context`: Initial context variables (VObject)
///
/// Returns the rendered HTML or None on error.
pub async fn render_template_cell(
    context_id: ContextId,
    template_name: &str,
    initial_context: facet_value::Value,
) -> Option<Result<String, String>> {
    let client = gingembre_cell().await?;

    match client
        .render(context_id, template_name.to_string(), initial_context)
        .await
    {
        Ok(RenderResult::Success { html }) => Some(Ok(html)),
        Ok(RenderResult::Error { message }) => Some(Err(message)),
        Err(e) => {
            warn!("gingembre render RPC failed: {:?}", e);
            None
        }
    }
}

/// Evaluate a template expression using the gingembre cell.
///
/// # Arguments
/// - `context_id`: The render context ID
/// - `expression`: The expression to evaluate
/// - `context`: Context variables
///
/// Returns the result value or None on error.
#[allow(dead_code)] // Scaffolding for future expression evaluation
pub async fn eval_expression_cell(
    context_id: ContextId,
    expression: &str,
    context: facet_value::Value,
) -> Option<Result<facet_value::Value, String>> {
    let client = gingembre_cell().await?;

    match client
        .eval_expression(context_id, expression.to_string(), context)
        .await
    {
        Ok(EvalResult::Success { value }) => Some(Ok(value)),
        Ok(EvalResult::Error { message }) => Some(Err(message)),
        Err(e) => {
            warn!("gingembre eval_expression RPC failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// Dialoguer cell (interactive prompts)
// ============================================================================

/// Spawn the dialoguer cell and return a client.
///
/// This cell provides interactive terminal prompts (select, confirm).
/// It inherits stdio to interact directly with the terminal.
///
/// Returns None if the cell binary is not available.
pub async fn dialoguer_client() -> Option<DialoguerClient<AnyTransport>> {
    use rapace::Frame;

    // Spawn cell with inherited stdio for terminal access
    let (session, _child) = spawn_cell_with_dispatcher(
        "ddc-cell-dialoguer",
        |_session| {
            // No custom dispatcher needed - just use a passthrough that rejects all methods
            |_request: Frame| {
                Box::pin(async move {
                    Err(rapace::RpcError::Status {
                        code: rapace::ErrorCode::Unimplemented,
                        message: "no methods implemented".to_string(),
                    })
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<Output = Result<Frame, rapace::RpcError>>
                                + Send,
                        >,
                    >
            }
        },
        true, // inherit stdio for terminal access
    )
    .await?;

    // Wait for the cell to be ready
    let timeout = std::time::Duration::from_secs(5);
    let peer_id = PEER_DIAG_INFO
        .read()
        .ok()?
        .iter()
        .find(|info| info.name == "ddc-cell-dialoguer")
        .map(|info| info.peer_id)?;

    cell_ready_registry()
        .wait_for_all_ready(&[peer_id], timeout)
        .await
        .ok()?;

    Some(DialoguerClient::new(session))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi() {
        // No escapes
        assert_eq!(strip_ansi("hello world"), "hello world");

        // Simple color
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");

        // Multiple escapes
        assert_eq!(
            strip_ansi("\x1b[38;5;246m│\x1b[0m hello \x1b[31merror\x1b[0m"),
            "│ hello error"
        );

        // The actual error format from the issue
        let input = "\x1b[31mError:\x1b[0m Error at FrontmatterToml";
        assert_eq!(strip_ansi(input), "Error: Error at FrontmatterToml");
    }
}
