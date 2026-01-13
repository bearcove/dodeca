//! Unified Host for dodeca.
//!
//! The Host is a singleton that owns all shared state:
//! - Cell infrastructure (SHM, connection handles)
//! - Render context registry (for template callbacks)
//! - TUI command forwarding
//!
//! Access via `Host::get()`. Get typed cell clients via `Host::client::<C>()`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cell_gingembre_proto::ContextId;
use cell_host_proto::{CommandResult, ServerCommand};
use dashmap::DashMap;
use roam::session::ConnectionHandle;
use tokio::sync::{Mutex, Notify, mpsc};

use crate::template_host::RenderContext;

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
// Client Access (bridges to cells.rs for now)
// ============================================================================

impl Host {
    /// Get a typed cell client.
    ///
    /// This looks up the cell by binary name and returns a client.
    pub fn client<C: CellClient>(&self) -> Option<C> {
        let handle = self.get_cell_handle(C::CELL_NAME)?;
        Some(C::from_handle(handle))
    }
}
