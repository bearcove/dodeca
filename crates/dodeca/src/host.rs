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
use roam::session::ConnectionHandle;
use roam_shm::SpawnTicket;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, Notify, mpsc};
use tracing::{debug, info, warn};

use crate::template_host::RenderContext;

// ============================================================================
// Pending Cell (Lazy Spawning)
// ============================================================================

/// A cell that has been registered but not yet spawned.
///
/// Holds the SpawnTicket which keeps the doorbell fd alive until spawn.
pub struct PendingCell {
    /// Path to the cell binary
    pub binary_path: PathBuf,
    /// Whether the cell inherits stdio (e.g., TUI)
    pub inherit_stdio: bool,
    /// The spawn ticket from roam-shm (owns the doorbell fd)
    pub ticket: SpawnTicket,
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
    /// Connection handles for spawned cells, keyed by binary name (e.g., "ddc-cell-sass").
    cell_handles: DashMap<String, ConnectionHandle>,

    // -------------------------------------------------------------------------
    // Pending Cells (Lazy Spawning)
    // -------------------------------------------------------------------------
    /// Cells that have been registered but not yet spawned.
    /// Keyed by binary name (e.g., "ddc-cell-sass").
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
    pub fn register_cell_handle(&self, binary_name: String, handle: ConnectionHandle) {
        self.cell_handles.insert(binary_name, handle);
    }

    /// Get a cell's connection handle by binary name.
    pub fn get_cell_handle(&self, binary_name: &str) -> Option<ConnectionHandle> {
        self.cell_handles.get(binary_name).map(|r| r.clone())
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

    // =========================================================================
    // Lazy Spawning
    // =========================================================================

    /// Register a pending cell (not yet spawned).
    ///
    /// The cell will be spawned on first access via `client_async::<C>()`.
    #[allow(dead_code)] // Will be used when lazy spawning is fully integrated
    pub fn register_pending_cell(&self, binary_name: String, pending: PendingCell) {
        if let Ok(mut cells) = self.pending_cells.lock() {
            cells.insert(binary_name, pending);
        }
    }

    /// Check if a cell is available (either spawned or pending).
    #[allow(dead_code)] // Will be used when lazy spawning is fully integrated
    pub fn has_cell(&self, binary_name: &str) -> bool {
        if self.cell_handles.contains_key(binary_name) {
            return true;
        }
        if let Ok(cells) = self.pending_cells.lock() {
            return cells.contains_key(binary_name);
        }
        false
    }

    /// Check if a cell is pending (registered but not spawned).
    #[allow(dead_code)] // Will be used when lazy spawning is fully integrated
    pub fn is_cell_pending(&self, binary_name: &str) -> bool {
        if let Ok(cells) = self.pending_cells.lock() {
            return cells.contains_key(binary_name);
        }
        false
    }

    /// Take a pending cell (removes it from pending, for spawning).
    fn take_pending_cell(&self, binary_name: &str) -> Option<PendingCell> {
        if let Ok(mut cells) = self.pending_cells.lock() {
            return cells.remove(binary_name);
        }
        None
    }

    /// Spawn a pending cell and return its handle.
    ///
    /// This is called internally by `client_async()` when a cell needs to be spawned.
    /// The handle must already be registered (from init time).
    pub async fn spawn_pending_cell(&self, binary_name: &str) -> Option<ConnectionHandle> {
        // The handle should already exist (registered during init)
        let handle = self.get_cell_handle(binary_name)?;

        // Take the pending cell (if not already spawned)
        let pending = match self.take_pending_cell(binary_name) {
            Some(p) => p,
            None => {
                // Already spawned (or never registered)
                return Some(handle);
            }
        };

        // Spawn the cell process
        spawn_cell_process(binary_name, pending, self.is_quiet_mode()).await;

        // Wait for the cell to be ready
        wait_for_cell_ready(binary_name).await;

        Some(handle)
    }
}

// ============================================================================
// CellClient Trait
// ============================================================================

/// Trait for type-safe cell client access.
///
/// Implement this for each cell client type to enable `Host::client::<C>()`.
pub trait CellClient: Sized {
    /// The cell's binary name (e.g., "ddc-cell-sass").
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

// Implement for all cell clients
impl_cell_client!(cell_sass_proto::SassCompilerClient, "ddc-cell-sass");
impl_cell_client!(
    cell_markdown_proto::MarkdownProcessorClient,
    "ddc-cell-markdown"
);
impl_cell_client!(cell_html_proto::HtmlProcessorClient, "ddc-cell-html");
impl_cell_client!(cell_css_proto::CssProcessorClient, "ddc-cell-css");
impl_cell_client!(cell_image_proto::ImageProcessorClient, "ddc-cell-image");
impl_cell_client!(cell_webp_proto::WebPProcessorClient, "ddc-cell-webp");
impl_cell_client!(cell_jxl_proto::JXLProcessorClient, "ddc-cell-jxl");
impl_cell_client!(cell_minify_proto::MinifierClient, "ddc-cell-minify");
impl_cell_client!(cell_js_proto::JsProcessorClient, "ddc-cell-js");
impl_cell_client!(cell_svgo_proto::SvgoOptimizerClient, "ddc-cell-svgo");
impl_cell_client!(cell_fonts_proto::FontProcessorClient, "ddc-cell-fonts");
impl_cell_client!(
    cell_linkcheck_proto::LinkCheckerClient,
    "ddc-cell-linkcheck"
);
impl_cell_client!(cell_html_diff_proto::HtmlDifferClient, "ddc-cell-html-diff");
impl_cell_client!(cell_dialoguer_proto::DialoguerClient, "ddc-cell-dialoguer");
impl_cell_client!(
    cell_pagefind_proto::SearchIndexerClient,
    "ddc-cell-pagefind"
);
impl_cell_client!(
    cell_code_execution_proto::CodeExecutorClient,
    "ddc-cell-code-execution"
);
impl_cell_client!(cell_http_proto::TcpTunnelClient, "ddc-cell-http");
impl_cell_client!(
    cell_gingembre_proto::TemplateRendererClient,
    "ddc-cell-gingembre"
);
impl_cell_client!(cell_tui_proto::TuiDisplayClient, "ddc-cell-tui");

// ============================================================================
// Client Access
// ============================================================================

impl Host {
    /// Get a typed cell client (sync, returns None if not yet spawned).
    ///
    /// This looks up the cell by binary name and returns a client.
    /// Use `client_async()` if you want lazy spawning on demand.
    pub fn client<C: CellClient>(&self) -> Option<C> {
        let handle = self.get_cell_handle(C::CELL_NAME)?;
        Some(C::from_handle(handle))
    }

    /// Get a typed cell client, spawning if needed (async).
    ///
    /// This will spawn the cell process if it's pending and wait for it to be ready.
    pub async fn client_async<C: CellClient>(&self) -> Option<C> {
        // Fast path: already spawned
        if let Some(handle) = self.get_cell_handle(C::CELL_NAME) {
            return Some(C::from_handle(handle));
        }

        // Slow path: spawn if pending
        let handle = self.spawn_pending_cell(C::CELL_NAME).await?;
        Some(C::from_handle(handle))
    }
}

// ============================================================================
// Lazy Spawning Helpers
// ============================================================================

/// Spawn a cell process from a PendingCell.
///
/// The handle must already be registered before calling this.
/// This function spawns the process and sets up child monitoring.
async fn spawn_cell_process(binary_name: &str, pending: PendingCell, quiet_mode: bool) {
    let PendingCell {
        binary_path,
        inherit_stdio,
        ticket,
    } = pending;

    // Build the command
    let mut cmd = Command::new(&binary_path);
    for arg in ticket.to_args() {
        cmd.arg(arg);
    }

    // Configure stdio
    if inherit_stdio {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    } else if quiet_mode {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("DODECA_QUIET", "1");
    } else {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }

    // Spawn the process
    let mut child = match ur_taking_me_with_you::spawn_dying_with_parent_async(cmd) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to spawn {}: {:?}", binary_name, e);
            return;
        }
    };

    // Capture stdio if not inheriting and not quiet
    if !inherit_stdio && !quiet_mode {
        capture_cell_stdio(binary_name, &mut child);
    }

    debug!(
        "Spawned {} cell (peer_id={:?}) from {}",
        binary_name,
        ticket.peer_id,
        binary_path.display()
    );

    // Drop ticket to close our end of the doorbell
    drop(ticket);

    // Spawn child management task
    let cell_label = binary_name.to_string();
    tokio::spawn(async move {
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
        info!("{} cell exited", cell_label);
    });
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
async fn wait_for_cell_ready(binary_name: &str) {
    // Wait for the cell to report ready via CellLifecycle::ready()
    // The timeout is generous since cells may take time to initialize
    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();

    loop {
        // Check if cell is ready (via cells.rs registry)
        // For now, we just wait a short time since the cell initialization
        // should happen quickly after spawn
        if start.elapsed() >= timeout {
            warn!("Timeout waiting for {} to be ready", binary_name);
            break;
        }

        // Small delay between checks
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Check if we got a handle (meaning cell connected)
        if Host::get().get_cell_handle(binary_name).is_some() {
            debug!("{} cell is ready", binary_name);
            break;
        }
    }
}
