//! Unified Host for dodeca.
//!
//! The Host is a singleton that owns all shared state:
//! - Cell infrastructure (SHM, connection handles)
//! - Render context registry (for template callbacks)
//! - TUI command forwarding
//! - Pending cells (lazy spawning)
//!
//! Access via `Host::get()`. Get typed cell clients via `Host::client::<C>()`.
//! For async spawning on demand, use `Host::client_async::<C>()`.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use cell_gingembre_proto::ContextId;
use cell_host_proto::{CommandResult, ServerCommand};
use dashmap::DashMap;
use roam::session::{ConnectionHandle, RoutedDispatcher};
use roam_tracing::{HostTracingDispatcher, HostTracingState, TaggedRecord};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, Notify, mpsc};
use tracing::{debug, error, info, warn};

use crate::template_host::RenderContext;

// ============================================================================
// MaybeDevtoolsDispatcher - Either includes DevtoolsService or not
// ============================================================================

type BaseDispatcher = RoutedDispatcher<
    cell_host_proto::HostServiceDispatcher<crate::cells::HostServiceImpl>,
    HostTracingDispatcher<roam_tracing::HostTracingService>,
>;

type DevtoolsDispatcher = dodeca_protocol::DevtoolsServiceDispatcher<crate::cell_server::HostDevtoolsService>;

/// Enum dispatcher that includes DevtoolsService when site_server is available.
pub enum MaybeDevtoolsDispatcher {
    WithDevtools(RoutedDispatcher<DevtoolsDispatcher, BaseDispatcher>),
    WithoutDevtools(BaseDispatcher),
}

impl roam::session::ServiceDispatcher for MaybeDevtoolsDispatcher {
    fn method_ids(&self) -> Vec<u64> {
        match self {
            MaybeDevtoolsDispatcher::WithDevtools(d) => d.method_ids(),
            MaybeDevtoolsDispatcher::WithoutDevtools(d) => d.method_ids(),
        }
    }

    fn dispatch(
        &self,
        method_id: u64,
        message: Vec<u8>,
        channels: Vec<u64>,
        request_id: u64,
        registry: &mut roam::session::ChannelRegistry,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        match self {
            MaybeDevtoolsDispatcher::WithDevtools(d) => {
                d.dispatch(method_id, message, channels, request_id, registry)
            }
            MaybeDevtoolsDispatcher::WithoutDevtools(d) => {
                d.dispatch(method_id, message, channels, request_id, registry)
            }
        }
    }
}

// ============================================================================
// Pending Cell (Lazy Spawning)
// ============================================================================

/// A cell that has been registered but not yet spawned.
///
/// Stores metadata needed to spawn the cell on first access.
pub struct PendingCell {
    /// Path to the cell binary
    pub binary_path: PathBuf,
    /// Whether the cell inherits stdio (e.g., TUI)
    pub inherit_stdio: bool,
}

// ============================================================================
// Host Singleton
// ============================================================================

/// The unified Host that owns all shared state.
pub struct Host {
    // -------------------------------------------------------------------------
    // Mode & Shutdown
    // -------------------------------------------------------------------------
    /// Whether TUI mode is enabled (serve command with terminal).
    tui_mode: std::sync::atomic::AtomicBool,
    /// Signaled when Exit command is received.
    exit_notify: Notify,

    // -------------------------------------------------------------------------
    // Render Context Registry (from template_host.rs)
    // -------------------------------------------------------------------------
    /// Active render contexts, keyed by context ID.
    render_contexts: DashMap<u64, RenderContext>,
    /// Counter for generating unique context IDs.
    next_context_id: AtomicU64,

    // -------------------------------------------------------------------------
    // TUI Command Forwarding
    // -------------------------------------------------------------------------
    /// Channel to forward commands from TUI cell to main loop.
    command_tx: mpsc::UnboundedSender<ServerCommand>,
    /// Receiver end - taken by main.rs via `take_command_rx()`.
    command_rx: Mutex<Option<mpsc::UnboundedReceiver<ServerCommand>>>,

    // -------------------------------------------------------------------------
    // Cell Connection Handles
    // -------------------------------------------------------------------------
    /// Connection handles for cells, keyed by logical name (e.g., "sass", "gingembre").
    cell_handles: DashMap<String, ConnectionHandle>,

    // -------------------------------------------------------------------------
    // Pending Cells (Lazy Spawning)
    // -------------------------------------------------------------------------
    /// Cells that have been registered but not yet spawned.
    /// Keyed by logical name (e.g., "sass", "gingembre").
    /// The PendingCell holds the SpawnTicket which is consumed on spawn.
    pending_cells: std::sync::Mutex<std::collections::HashMap<String, PendingCell>>,
    /// Whether quiet mode is enabled (suppress cell output when TUI is active).
    quiet_mode: std::sync::atomic::AtomicBool,

    // -------------------------------------------------------------------------
    // Site Server (for HTTP cell)
    // -------------------------------------------------------------------------
    /// SiteServer for HTTP cell content serving.
    /// Set via `provide_site_server()` before cell initialization.
    site_server: std::sync::OnceLock<Arc<crate::serve::SiteServer>>,

    // -------------------------------------------------------------------------
    // Vite Dev Server
    // -------------------------------------------------------------------------
    /// Vite dev server port (if Vite is running).
    /// Set via `provide_vite_port()` after ViteServer starts.
    vite_port: std::sync::OnceLock<Option<u16>>,

    // -------------------------------------------------------------------------
    // Driver Handle (for lazy spawning)
    // -------------------------------------------------------------------------
    /// MultiPeerHostDriver handle for dynamically creating peers.
    /// Set during cell initialization via `set_driver_handle()`.
    driver_handle: std::sync::OnceLock<roam_shm::driver::MultiPeerHostDriverHandle>,

    // -------------------------------------------------------------------------
    // Cell Tracing
    // -------------------------------------------------------------------------
    /// Shared state for receiving tracing records from cells.
    tracing_state: Arc<HostTracingState>,

    // -------------------------------------------------------------------------
    // Build Steps
    // -------------------------------------------------------------------------
    /// Build step executor (set when config is loaded).
    build_step_executor: std::sync::OnceLock<Arc<crate::build_steps::BuildStepExecutor>>,
}

impl Host {
    /// Get the global Host singleton. Lazily initializes on first call.
    pub fn get() -> &'static Arc<Host> {
        static HOST: std::sync::OnceLock<Arc<Host>> = std::sync::OnceLock::new();
        HOST.get_or_init(|| {
            let (command_tx, command_rx) = mpsc::unbounded_channel();
            Arc::new(Host {
                tui_mode: std::sync::atomic::AtomicBool::new(false),
                exit_notify: Notify::new(),
                render_contexts: DashMap::new(),
                next_context_id: AtomicU64::new(1),
                command_tx,
                command_rx: Mutex::new(Some(command_rx)),
                cell_handles: DashMap::new(),
                pending_cells: std::sync::Mutex::new(std::collections::HashMap::new()),
                quiet_mode: std::sync::atomic::AtomicBool::new(false),
                site_server: std::sync::OnceLock::new(),
                vite_port: std::sync::OnceLock::new(),
                driver_handle: std::sync::OnceLock::new(),
                tracing_state: HostTracingState::new(8192),
                build_step_executor: std::sync::OnceLock::new(),
            })
        })
    }

    /// Enable TUI mode. Call this before initializing cells in serve mode.
    pub fn enable_tui_mode(&self) {
        self.tui_mode.store(true, Ordering::SeqCst);
    }

    /// Check if TUI mode is enabled.
    pub fn is_tui_mode(&self) -> bool {
        self.tui_mode.load(Ordering::SeqCst)
    }

    /// Signal that exit was requested (called when Exit command is received).
    pub fn signal_exit(&self) {
        self.exit_notify.notify_waiters();
    }

    /// Wait for exit to be signaled.
    pub async fn wait_for_exit(&self) {
        self.exit_notify.notified().await;
    }

    // =========================================================================
    // Render Context Registry
    // =========================================================================

    /// Register a render context and return its unique ID.
    pub fn register_render_context(&self, context: RenderContext) -> ContextId {
        let id = self.next_context_id.fetch_add(1, Ordering::SeqCst);
        self.render_contexts.insert(id, context);
        ContextId(id)
    }

    /// Unregister a render context.
    pub fn unregister_render_context(&self, id: ContextId) {
        self.render_contexts.remove(&id.0);
    }

    /// Look up a render context by ID.
    pub fn get_render_context(
        &self,
        id: ContextId,
    ) -> Option<dashmap::mapref::one::Ref<'_, u64, RenderContext>> {
        self.render_contexts.get(&id.0)
    }

    // =========================================================================
    // TUI Command Forwarding
    // =========================================================================

    /// Take the command receiver. Call this once from main.rs.
    pub async fn take_command_rx(&self) -> Option<mpsc::UnboundedReceiver<ServerCommand>> {
        self.command_rx.lock().await.take()
    }

    /// Handle a command from the TUI cell (called by HostService impl).
    pub fn handle_tui_command(&self, command: ServerCommand) -> CommandResult {
        match self.command_tx.send(command) {
            Ok(_) => CommandResult::Ok,
            Err(e) => CommandResult::Error {
                message: format!("Failed to send command: {}", e),
            },
        }
    }

    // =========================================================================
    // Cell Handle Management
    // =========================================================================

    /// Register a cell's connection handle.
    ///
    /// Called by `cells::init_cells_inner()` after spawning cells.
    /// Uses logical cell name (e.g., "sass", "gingembre").
    pub fn register_cell_handle(&self, cell_name: String, handle: ConnectionHandle) {
        self.cell_handles.insert(cell_name, handle);
    }

    /// Get a cell's connection handle by logical name (e.g., "sass", "gingembre").
    fn get_cell_handle(&self, cell_name: &str) -> Option<ConnectionHandle> {
        self.cell_handles.get(cell_name).map(|r| r.clone())
    }

    /// Get all registered cell names.
    #[allow(dead_code)] // Utility method for future use
    pub fn cell_names(&self) -> Vec<String> {
        self.cell_handles.iter().map(|r| r.key().clone()).collect()
    }

    // =========================================================================
    // Quiet Mode
    // =========================================================================

    /// Enable quiet mode for spawned cells (call this when TUI is active).
    pub fn set_quiet_mode(&self, quiet: bool) {
        self.quiet_mode.store(quiet, Ordering::SeqCst);
    }

    /// Check if quiet mode is enabled.
    pub fn is_quiet_mode(&self) -> bool {
        self.quiet_mode.load(Ordering::SeqCst)
    }

    // =========================================================================
    // Site Server
    // =========================================================================

    /// Provide the SiteServer for HTTP cell content serving.
    /// This must be called before cell initialization when the HTTP cell needs to serve content.
    /// For build-only commands, this can be skipped.
    pub fn provide_site_server(&self, server: Arc<crate::serve::SiteServer>) {
        let _ = self.site_server.set(server);
    }

    /// Get the SiteServer reference, if provided.
    pub fn site_server(&self) -> Option<&Arc<crate::serve::SiteServer>> {
        self.site_server.get()
    }

    /// Provide the Vite dev server port.
    /// Call this after ViteServer starts, or with None if Vite is not enabled.
    pub fn provide_vite_port(&self, port: Option<u16>) {
        let _ = self.vite_port.set(port);
    }

    /// Get the Vite dev server port, if Vite is running.
    pub fn get_vite_port(&self) -> Option<u16> {
        self.vite_port.get().copied().flatten()
    }

    /// Set the driver handle for dynamic peer creation (lazy spawning).
    /// This must be called during cell initialization.
    pub fn set_driver_handle(&self, handle: roam_shm::driver::MultiPeerHostDriverHandle) {
        let _ = self.driver_handle.set(handle);
    }

    /// Get the driver handle for creating peers dynamically.
    pub fn driver_handle(&self) -> Option<&roam_shm::driver::MultiPeerHostDriverHandle> {
        self.driver_handle.get()
    }

    // =========================================================================
    // Cell Tracing
    // =========================================================================

    /// Get the tracing state for creating per-cell tracing services.
    pub fn tracing_state(&self) -> &Arc<HostTracingState> {
        &self.tracing_state
    }

    /// Take the tracing record receiver.
    ///
    /// Call this once during initialization to get the stream of tracing
    /// records from all cells. Returns `None` if already taken.
    pub fn take_tracing_receiver(
        &self,
    ) -> Option<tokio::sync::mpsc::Receiver<TaggedRecord>> {
        self.tracing_state.take_receiver()
    }

    // =========================================================================
    // Build Steps
    // =========================================================================

    /// Set the build step executor (call when config is loaded).
    pub fn set_build_step_executor(&self, executor: Arc<crate::build_steps::BuildStepExecutor>) {
        let _ = self.build_step_executor.set(executor);
    }

    /// Get the build step executor.
    pub fn build_step_executor(&self) -> Option<&Arc<crate::build_steps::BuildStepExecutor>> {
        self.build_step_executor.get()
    }

    // =========================================================================
    // Lazy Spawning
    // =========================================================================

    /// Register a pending cell (not yet spawned).
    ///
    /// The cell will be spawned on first access via `client_async::<C>()`.
    pub fn register_pending_cell(&self, cell_name: String, pending: PendingCell) {
        if let Ok(mut cells) = self.pending_cells.lock() {
            cells.insert(cell_name, pending);
        }
    }

    /// Take a pending cell (removes it from pending, for spawning).
    fn take_pending_cell(&self, cell_name: &str) -> Option<PendingCell> {
        if let Ok(mut cells) = self.pending_cells.lock() {
            return cells.remove(cell_name);
        }
        None
    }

    /// Spawn a pending cell and wait for it to be ready.
    ///
    /// This is called internally by `client_async()` when a cell needs to be spawned.
    /// Returns Some(handle) on success, None if cell couldn't be spawned.
    pub async fn spawn_pending_cell(&self, cell_name: &str) -> Option<ConnectionHandle> {
        debug!(cell = cell_name, "spawn_pending_cell: taking pending cell");

        // Take the pending cell atomically (prevents race conditions)
        let pending = match self.take_pending_cell(cell_name) {
            Some(p) => {
                debug!(
                    cell = cell_name,
                    binary = %p.binary_path.display(),
                    "spawn_pending_cell: got pending cell"
                );
                p
            }
            None => {
                debug!(
                    cell = cell_name,
                    "spawn_pending_cell: no pending cell, already spawned by another caller"
                );
                // Already spawned by another caller - just wait for ready
                wait_for_cell_ready(cell_name).await;
                return self.get_cell_handle(cell_name);
            }
        };

        // Spawn the cell process
        debug!(
            cell = cell_name,
            "spawn_pending_cell: calling spawn_cell_process"
        );
        spawn_cell_process(cell_name, pending, self.is_quiet_mode()).await;

        // Wait for the cell to be ready
        debug!(
            cell = cell_name,
            "spawn_pending_cell: waiting for cell ready"
        );
        wait_for_cell_ready(cell_name).await;

        debug!(cell = cell_name, "spawn_pending_cell: done");
        self.get_cell_handle(cell_name)
    }
}

// ============================================================================
// CellClient Trait
// ============================================================================

/// Trait for type-safe cell client access.
///
/// Implement this for each cell client type to enable `Host::client::<C>()`.
pub trait CellClient: Sized {
    /// The cell's logical name (e.g., "sass", "markdown", "gingembre").
    /// Binary name is derived at spawn time: `ddc-cell-{name}`.
    const CELL_NAME: &'static str;

    /// Create a client from a connection handle.
    fn from_handle(handle: roam::session::ConnectionHandle) -> Self;
}

// ============================================================================
// Client Implementations
// ============================================================================

// Macro to implement CellClient for roam-generated clients
macro_rules! impl_cell_client {
    ($client:ty, $name:literal) => {
        impl CellClient for $client {
            const CELL_NAME: &'static str = $name;

            fn from_handle(handle: roam::session::ConnectionHandle) -> Self {
                Self::new(handle)
            }
        }
    };
}

// Implement for all cell clients (using logical names, not binary names)
impl_cell_client!(cell_sass_proto::SassCompilerClient, "sass");
impl_cell_client!(cell_markdown_proto::MarkdownProcessorClient, "markdown");
impl_cell_client!(cell_html_proto::HtmlProcessorClient, "html");
impl_cell_client!(cell_css_proto::CssProcessorClient, "css");
impl_cell_client!(cell_image_proto::ImageProcessorClient, "image");
impl_cell_client!(cell_webp_proto::WebPProcessorClient, "webp");
impl_cell_client!(cell_jxl_proto::JXLProcessorClient, "jxl");
impl_cell_client!(cell_minify_proto::MinifierClient, "minify");
impl_cell_client!(cell_js_proto::JsProcessorClient, "js");
impl_cell_client!(cell_svgo_proto::SvgoOptimizerClient, "svgo");
impl_cell_client!(cell_fonts_proto::FontProcessorClient, "fonts");
impl_cell_client!(cell_linkcheck_proto::LinkCheckerClient, "linkcheck");
impl_cell_client!(cell_html_diff_proto::HtmlDifferClient, "html-diff");
impl_cell_client!(cell_dialoguer_proto::DialoguerClient, "dialoguer");
impl_cell_client!(
    cell_code_execution_proto::CodeExecutorClient,
    "code-execution"
);
impl_cell_client!(cell_http_proto::TcpTunnelClient, "http");
impl_cell_client!(cell_gingembre_proto::TemplateRendererClient, "gingembre");
impl_cell_client!(cell_tui_proto::TuiDisplayClient, "tui");
impl_cell_client!(cell_term_proto::TermRecorderClient, "term");

// ============================================================================
// Client Access
// ============================================================================

impl Host {
    /// Get a typed cell client, spawning if needed (async).
    ///
    /// This will spawn the cell process if it's pending and wait for it to be ready.
    pub async fn client_async<C: CellClient>(&self) -> Option<C> {
        // Ensure cell registry is initialized (idempotent, registers for lazy spawning, doesn't spawn)
        if let Err(e) = crate::cells::ensure_cell_registry_initialized().await {
            tracing::error!(cell = C::CELL_NAME, error = %e, "Cell registry initialization failed");
            return None;
        }

        // Fast path: cell is already ready (spawned and reported ready)
        if crate::cells::cell_ready_registry().is_ready(C::CELL_NAME) {
            let handle = self.get_cell_handle(C::CELL_NAME)?;
            debug!(
                cell = C::CELL_NAME,
                "client_async: already ready (fast path)"
            );
            return Some(C::from_handle(handle));
        }

        debug!(cell = C::CELL_NAME, "client_async: not ready, spawning");

        // Slow path: spawn the cell (creates and registers handle)
        self.spawn_pending_cell(C::CELL_NAME).await?;

        // Get the handle that was just registered during spawn
        let handle = self.get_cell_handle(C::CELL_NAME)?;
        debug!(
            cell = C::CELL_NAME,
            "client_async: spawn complete, returning client"
        );
        Some(C::from_handle(handle))
    }
}

// ============================================================================
// Lazy Spawning Helpers
// ============================================================================

/// Spawn a cell process from a PendingCell using dynamic peer creation.
///
/// This function:
/// 1. Creates a peer dynamically (gets fresh SpawnTicket with valid FDs)
/// 2. Spawns the process with the ticket
/// 3. Registers the peer with the driver
async fn spawn_cell_process(cell_name: &str, pending: PendingCell, quiet_mode: bool) {
    let PendingCell {
        binary_path,
        inherit_stdio,
    } = pending;

    // Get driver handle for dynamic peer creation
    let driver_handle = match Host::get().driver_handle() {
        Some(h) => h,
        None => {
            error!(
                cell = cell_name,
                "No driver handle available for lazy spawning"
            );
            return;
        }
    };

    // Create peer dynamically - this gives us a fresh SpawnTicket with valid FDs
    let binary_name = format!("ddc-cell-{}", cell_name);
    let cell_name_for_death = cell_name.to_string();
    let ticket = match driver_handle
        .create_peer(roam_shm::spawn::AddPeerOptions {
            peer_name: Some(binary_name.clone()),
            on_death: Some(Arc::new(move |peer_id| {
                eprintln!(
                    "FATAL: {} cell died unexpectedly (peer_id={:?})",
                    cell_name_for_death, peer_id
                );
                std::process::exit(1);
            })),
        })
        .await
    {
        Ok(t) => t,
        Err(e) => {
            error!(cell = cell_name, error = ?e, "Failed to create peer dynamically");
            return;
        }
    };

    let peer_id = ticket.peer_id;
    let args = ticket.to_args();

    debug!(
        cell = cell_name,
        ?peer_id,
        binary = %binary_path.display(),
        ?args,
        inherit_stdio,
        quiet_mode,
        "spawn_cell_process: building command"
    );

    // Build the command
    let mut cmd = Command::new(&binary_path);
    for arg in &args {
        cmd.arg(arg);
    }

    // Configure stdio
    if inherit_stdio {
        debug!(cell = cell_name, "spawn_cell_process: inheriting stdio");
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    } else if quiet_mode {
        debug!(
            cell = cell_name,
            "spawn_cell_process: quiet mode (null stdio)"
        );
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("DODECA_QUIET", "1");
    } else {
        debug!(cell = cell_name, "spawn_cell_process: piped stdio");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }

    // Register the peer with the driver BEFORE spawning the child.
    // On Windows, this starts the named pipe accept() task so the child can connect.
    // On Unix, accept() is a no-op since socketpairs are already connected.
    debug!(
        cell = cell_name,
        ?peer_id,
        "spawn_cell_process: registering peer with driver (before spawn)"
    );

    // Create the host service dispatcher (handles ready(), template calls, etc.)
    let host_service = crate::cells::HostServiceImpl::new(
        crate::cells::HostCellLifecycle::new(crate::cells::cell_ready_registry().clone()),
        crate::template_host::TemplateHostImpl::new(),
        Host::get().site_server().cloned(),
    );
    let host_service_dispatcher = cell_host_proto::HostServiceDispatcher::new(host_service);

    // Create the tracing service dispatcher (handles emit_tracing(), get_tracing_config())
    let tracing_service = Host::get()
        .tracing_state()
        .service_for_peer(peer_id.get() as u64, Some(cell_name.to_string()));
    let tracing_dispatcher = HostTracingDispatcher::new(tracing_service);

    // Compose dispatchers: Tracing -> HostService -> DevtoolsService (if available)
    // We always route to the DevtoolsService if site_server is available
    let base_dispatcher = RoutedDispatcher::new(host_service_dispatcher, tracing_dispatcher);

    // Create the devtools service dispatcher (handles browser devtools RPC calls)
    // Only active if site_server is available (i.e., serve command, not build)
    let dispatcher = match Host::get().site_server() {
        Some(server) => {
            let devtools_service = crate::cell_server::HostDevtoolsService::new(server.clone());
            let devtools_dispatcher = dodeca_protocol::DevtoolsServiceDispatcher::new(devtools_service);
            MaybeDevtoolsDispatcher::WithDevtools(RoutedDispatcher::new(devtools_dispatcher, base_dispatcher))
        }
        None => MaybeDevtoolsDispatcher::WithoutDevtools(base_dispatcher),
    };

    match driver_handle.add_peer(peer_id, dispatcher).await {
        Ok(handle) => {
            debug!(
                cell = cell_name,
                ?peer_id,
                "spawn_cell_process: peer registered, storing handle"
            );
            Host::get().register_cell_handle(cell_name.to_string(), handle);
        }
        Err(e) => {
            error!(cell = cell_name, ?peer_id, error = ?e, "Failed to register peer with driver");
            return;
        }
    }

    // Spawn the process (after driver is ready to accept doorbell connection)
    debug!(
        cell = cell_name,
        "spawn_cell_process: spawning child process"
    );
    let mut child = match ur_taking_me_with_you::spawn_dying_with_parent_async(cmd) {
        Ok(c) => {
            debug!(cell = cell_name, pid = ?c.id(), "spawn_cell_process: child spawned successfully");
            c
        }
        Err(e) => {
            warn!(cell = cell_name, error = ?e, "spawn_cell_process: failed to spawn");
            return;
        }
    };

    // Capture stdio if not inheriting and not quiet
    if !inherit_stdio && !quiet_mode {
        capture_cell_stdio(cell_name, &mut child);
    }

    debug!(
        cell = cell_name,
        ?peer_id,
        binary = %binary_path.display(),
        "spawn_cell_process: dropping ticket to close doorbell"
    );

    // Drop ticket to close our end of the doorbell
    drop(ticket);

    debug!(
        cell = cell_name,
        "spawn_cell_process: spawning child monitor task"
    );

    // Spawn child management task
    let cell_label = cell_name.to_string();
    tokio::spawn(async move {
        debug!(cell = %cell_label, "child monitor: waiting for exit");
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
        info!(cell = %cell_label, "child monitor: cell exited normally");
    });

    debug!(cell = cell_name, "spawn_cell_process: done");
}

/// Capture cell stdout/stderr and log it.
fn capture_cell_stdio(label: &str, child: &mut tokio::process::Child) {
    if let Some(stdout) = child.stdout.take() {
        spawn_stdio_pump(label.to_string(), "stdout", stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_stdio_pump(label.to_string(), "stderr", stderr);
    }
}

/// Pump a stdio stream to the logger.
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

/// Wait for a cell to be ready.
///
/// This checks the cell ready registry from cells.rs.
async fn wait_for_cell_ready(cell_name: &str) {
    debug!(cell = cell_name, "wait_for_cell_ready: starting");

    // Wait for the cell to report ready via CellLifecycle::ready()
    let timeout = Duration::from_secs(1);
    let start = std::time::Instant::now();

    let mut check_count = 0u32;
    loop {
        check_count += 1;

        // Check if cell reported ready via the registry
        if crate::cells::cell_ready_registry().is_ready(cell_name) {
            debug!(
                cell = cell_name,
                elapsed_ms = start.elapsed().as_millis(),
                check_count,
                "wait_for_cell_ready: cell is ready"
            );
            break;
        }

        if start.elapsed() >= timeout {
            error!(
                cell = cell_name,
                elapsed_ms = start.elapsed().as_millis(),
                check_count,
                "Cell failed to start within 1 second - exiting"
            );
            std::process::exit(1);
        }

        // Log every 100 checks (roughly every second)
        if check_count.is_multiple_of(100) {
            debug!(
                cell = cell_name,
                elapsed_ms = start.elapsed().as_millis(),
                check_count,
                "wait_for_cell_ready: still waiting..."
            );
        }

        // Small delay between checks
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
