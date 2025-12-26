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

use cell_arborium_proto::{HighlightResult, SyntaxHighlightServiceClient};
use cell_code_execution_proto::{
    CodeExecutionResult, CodeExecutorClient, ExecuteSamplesInput, ExtractSamplesInput,
};
use cell_css_proto::{CssProcessorClient, CssResult};
use cell_fonts_proto::{FontAnalysis, FontProcessorClient, FontResult, SubsetFontInput};
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
use rapace::transport::shm::{HubConfig, HubHost, HubHostPeerTransport, close_peer_fd};
use rapace::{Frame, RpcError, RpcSession};
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
    let filter_str = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

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
    rpc_session: Arc<RpcSession>,
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
fn register_peer_diag(peer_id: u16, name: &str, rpc_session: Arc<RpcSession>) {
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
pub fn get_cell_session(name: &str) -> Option<Arc<RpcSession>> {
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

/// Spawn a cell binary through the hub with a custom dispatcher.
///
/// This is used for cells like TUI that need a custom dispatcher rather than
/// just being clients that we call methods on.
pub async fn spawn_cell_with_dispatcher<D>(
    binary_name: &str,
    dispatcher_factory: impl FnOnce(Arc<RpcSession>) -> D,
) -> Option<(Arc<RpcSession>, tokio::process::Child)>
where
    D: Fn(
            Frame,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Frame, rapace::RpcError>> + Send>,
        > + Send
        + Sync
        + 'static,
{
    let (hub, hub_path) = get_hub().await?;

    // Find the binary
    let exe_path = std::env::current_exe().ok()?;
    let dir = exe_path.parent()?;

    #[cfg(target_os = "windows")]
    let executable = format!("{binary_name}.exe");
    #[cfg(not(target_os = "windows"))]
    let executable = binary_name.to_string();

    let path = dir.join(&executable);
    if !path.exists() {
        warn!("cell binary not found: {}", path.display());
        return None;
    }

    // Add peer to hub
    let peer_info = match hub.add_peer() {
        Ok(info) => info,
        Err(e) => {
            warn!("failed to add peer for {}: {}", binary_name, e);
            return None;
        }
    };

    let peer_id = peer_info.peer_id;
    let peer_doorbell_fd = peer_info.peer_doorbell_fd;

    // Build command with hub args
    let mut cmd = Command::new(&path);
    cmd.arg(format!("--hub-path={}", hub_path.display()))
        .arg(format!("--peer-id={}", peer_id))
        .arg(format!("--doorbell-fd={}", peer_doorbell_fd))
        .stdin(Stdio::null());

    if is_quiet_mode() {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        cmd.env("DODECA_QUIET", "1");
    } else {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    }

    let mut child = match ur_taking_me_with_you::spawn_dying_with_parent_async(cmd) {
        Ok(child) => child,
        Err(e) => {
            warn!("failed to spawn {}: {}", executable, e);
            return None;
        }
    };

    // Register child PID for SIGUSR1 forwarding
    if let Some(pid) = child.id() {
        dodeca_debug::register_child_pid(pid);
    }

    if !is_quiet_mode() {
        capture_cell_stdio(binary_name, &mut child);
    }

    // Close our end of the peer's doorbell
    close_peer_fd(peer_doorbell_fd);

    // Create transport
    let peer_transport = HubHostPeerTransport::new(hub.clone(), peer_id, peer_info.doorbell);
    let transport = rapace::Transport::Shm(rapace::transport::shm::ShmTransport::HubHostPeer(
        peer_transport.clone(),
    ));

    let rpc_session = Arc::new(RpcSession::with_channel_start(transport.clone(), 1));

    // Register for diagnostics
    register_peer_diag(peer_id, binary_name, rpc_session.clone());

    // Set up the custom dispatcher
    rpc_session.set_dispatcher(dispatcher_factory(rpc_session.clone()));

    // Spawn the RPC session runner
    {
        let session_runner = rpc_session.clone();
        let cell_label = binary_name.to_string();
        tokio::spawn(async move {
            if let Err(e) = session_runner.run().await {
                warn!("{} RPC session error: {}", cell_label, e);
            }
        });
    }

    info!(
        "launched {} from {} (peer_id={})",
        binary_name,
        path.display(),
        peer_id
    );

    Some((rpc_session, child))
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
    rpc_session: Arc<RpcSession>,
}

macro_rules! define_cells {
    ( $( $key:ident => $Client:ident ),* $(,)? ) => {
        #[derive(Default)]
        pub struct CellRegistry {
            $(
                pub $key: Option<Arc<$Client>>,
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
    syntax_highlight=> SyntaxHighlightServiceClient,
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
        hub_path: &Path,
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

        // Add peer to hub and get peer info (peer_id, doorbells)
        let peer_info = match hub.add_peer() {
            Ok(info) => info,
            Err(e) => {
                warn!("failed to add peer for {} cell: {}", binary_name, e);
                return None;
            }
        };

        let peer_id = peer_info.peer_id;
        let peer_doorbell_fd = peer_info.peer_doorbell_fd;

        // Build command with hub args
        let mut cmd = Command::new(&path);
        cmd.arg(format!("--hub-path={}", hub_path.display()))
            .arg(format!("--peer-id={}", peer_id))
            .arg(format!("--doorbell-fd={}", peer_doorbell_fd))
            .stdin(Stdio::null());

        if is_quiet_mode() {
            cmd.stdout(Stdio::null())
                .stderr(Stdio::null())
                .env("DODECA_QUIET", "1");
        } else {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        }

        let mut child = match ur_taking_me_with_you::spawn_dying_with_parent_async(cmd) {
            Ok(child) => child,
            Err(e) => {
                warn!("failed to spawn {}: {}", executable, e);
                return None;
            }
        };

        // Register child PID for SIGUSR1 forwarding (debugging)
        if let Some(pid) = child.id() {
            dodeca_debug::register_child_pid(pid);
        }

        if !is_quiet_mode() {
            capture_cell_stdio(binary_name, &mut child);
        }

        // Close our end of the peer's doorbell (cell inherits it)
        close_peer_fd(peer_doorbell_fd);

        // CRITICAL: Create transport and start RPC session IMMEDIATELY after spawn.
        // This allows the host to process incoming messages (tracing events, etc.)
        // from the cell right away, preventing slot exhaustion on the current_thread
        // tokio runtime.
        let peer_transport = HubHostPeerTransport::new(hub.clone(), peer_id, peer_info.doorbell);
        let transport = rapace::Transport::Shm(rapace::transport::shm::ShmTransport::HubHostPeer(
            peer_transport.clone(),
        ));

        let rpc_session = Arc::new(RpcSession::with_channel_start(transport, 1));

        // Register for SIGUSR1 diagnostics
        register_peer_diag(peer_id, binary_name, rpc_session.clone());

        // Set up multi-service dispatcher for tracing and cell lifecycle
        let tracing_sink = ForwardingTracingSink::new();
        let lifecycle_impl = HostCellLifecycle::new(cell_ready_registry().clone());

        // Use 256KB buffer pool to handle larger payloads (fonts, search indexes, etc.)
        // Default 64KB is too small for some assets (e.g., fonts/test.woff2 is 73KB)
        let buffer_pool = rapace::BufferPool::with_capacity(128, 256 * 1024);
        let dispatcher = DispatcherBuilder::new()
            .add_service(TracingSinkService(Arc::new(TracingSinkServer::new(
                tracing_sink,
            ))))
            .add_service(CellLifecycleServer::new(lifecycle_impl).into_dispatch())
            .build(buffer_pool);

        rpc_session.set_dispatcher(dispatcher);

        // NOTE: Filter push is disabled during startup to avoid slot contention.
        // Cells default to "trace" level anyway, and we'd need to add the tracing service
        // back to their dispatcher for filter updates to work.
        // TODO: Add a separate post-startup filter push mechanism if needed.

        // Start the RPC session runner (processes incoming messages)
        {
            let session_runner = rpc_session.clone();
            let cell_label = binary_name.to_string();
            tokio::spawn(async move {
                if let Err(e) = session_runner.run().await {
                    warn!("{} cell RPC session error: {}", cell_label, e);
                }
            });
        }

        // Wait on the child asynchronously and reclaim slots when it dies
        {
            let cell_label = binary_name.to_string();
            let hub_for_cleanup = hub.clone();
            tokio::spawn(async move {
                match child.wait().await {
                    Ok(status) => {
                        if !status.success() {
                            warn!("{} cell exited with status: {}", cell_label, status);
                        }
                    }
                    Err(e) => {
                        warn!("{} cell wait error: {}", cell_label, e);
                    }
                }
                // Reclaim peer slots when cell dies
                hub_for_cleanup
                    .allocator()
                    .reclaim_peer_slots(peer_id as u32);
                info!(
                    "{} cell exited, reclaimed slots for peer {}",
                    cell_label, peer_id
                );
            });
        }

        debug!(
            "launched {} cell from {} (peer_id={})",
            binary_name,
            path.display(),
            peer_id
        );

        Some(SpawnedCell {
            peer_id,
            rpc_session,
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
                registry.syntax_highlight.is_some(),
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
) -> Option<Vec<dodeca_code_execution_types::CodeSample>> {
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
    samples: Vec<dodeca_code_execution_types::CodeSample>,
    config: dodeca_code_execution_types::CodeExecutionConfig,
) -> Option<
    Vec<(
        dodeca_code_execution_types::CodeSample,
        dodeca_code_execution_types::ExecutionResult,
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

/// Inject build info buttons into code blocks using the cell.
///
/// Returns (modified_html, had_buttons) or None if cell not loaded.
#[tracing::instrument(level = "debug", skip(html, code_metadata), fields(html_len = html.len(), metadata_count = code_metadata.len()))]
pub async fn inject_build_info_cell(
    html: &str,
    code_metadata: &HashMap<String, cell_html_proto::CodeExecutionMetadata>,
) -> Option<(String, bool)> {
    let cell = all().await.html.as_ref()?;

    match cell
        .inject_build_info(html.to_string(), code_metadata.clone())
        .await
    {
        Ok(cell_html_proto::HtmlResult::SuccessWithFlag { html, flag }) => Some((html, flag)),
        Ok(cell_html_proto::HtmlResult::Success { html }) => Some((html, false)),
        Ok(cell_html_proto::HtmlResult::Error { message }) => {
            warn!("html inject_build_info cell error: {}", message);
            None
        }
        Err(e) => {
            warn!("html inject_build_info cell call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// Syntax highlighting cell functions
// ============================================================================

/// Highlight source code using the rapace syntax highlight service.
///
/// Returns the code with syntax highlighting applied as HTML, or None if no service is available.
pub async fn highlight_code(code: &str, language: &str) -> Option<HighlightResult> {
    let client = syntax_highlight_client().await?;
    let code_len = code.len();
    tracing::debug!(
        language = language,
        code_len = code_len,
        "highlight_code: sending RPC request"
    );
    match client
        .highlight_code(code.to_string(), language.to_string())
        .await
    {
        Ok(result) => {
            tracing::debug!(
                language = language,
                code_len = code_len,
                html_len = result.html.len(),
                "highlight_code: RPC success"
            );
            Some(result)
        }
        Err(e) => {
            warn!(
                language = language,
                code_len = code_len,
                error = %e,
                "highlight_code: RPC failed"
            );
            None
        }
    }
}

/// Get the syntax highlight service client, if available
async fn syntax_highlight_client() -> Option<Arc<SyntaxHighlightServiceClient>> {
    all().await.syntax_highlight.clone()
}

// ============================================================================
// Markdown processing cell functions
// ============================================================================

/// Parsed markdown result with code blocks that need highlighting
pub struct ParsedMarkdown {
    /// The frontmatter parsed from the content
    pub frontmatter: cell_markdown_proto::Frontmatter,
    /// HTML output (with code block placeholders)
    pub html: String,
    /// Extracted headings
    pub headings: Vec<cell_markdown_proto::Heading>,
    /// Code blocks that need syntax highlighting
    pub code_blocks: Vec<cell_markdown_proto::CodeBlock>,
}

/// Parse and render markdown content using the cell.
///
/// Returns frontmatter, HTML (with placeholders), headings, and code blocks.
/// The caller is responsible for highlighting code blocks and replacing placeholders.
#[tracing::instrument(level = "debug", skip(content), fields(content_len = content.len()))]
pub async fn parse_and_render_markdown_cell(content: &str) -> Option<ParsedMarkdown> {
    let cell = all().await.markdown.as_ref()?;

    match cell.parse_and_render(content.to_string()).await {
        Ok(ParseResult::Success {
            frontmatter,
            html,
            headings,
            code_blocks,
        }) => Some(ParsedMarkdown {
            frontmatter,
            html,
            headings,
            code_blocks,
        }),
        Ok(ParseResult::Error { message }) => {
            warn!("markdown parse_and_render cell error: {}", message);
            None
        }
        Err(e) => {
            warn!("markdown parse_and_render cell call failed: {:?}", e);
            None
        }
    }
}

/// Render markdown to HTML using the cell (without frontmatter parsing).
#[tracing::instrument(level = "debug", skip(markdown), fields(markdown_len = markdown.len()))]
async fn _render_markdown_cell(
    markdown: &str,
) -> Option<(
    String,
    Vec<cell_markdown_proto::Heading>,
    Vec<cell_markdown_proto::CodeBlock>,
)> {
    let cell = all().await.markdown.as_ref()?;

    match cell.render_markdown(markdown.to_string()).await {
        Ok(MarkdownResult::Success {
            html,
            headings,
            code_blocks,
        }) => Some((html, headings, code_blocks)),
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
