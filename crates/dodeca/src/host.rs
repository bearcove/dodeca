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
use tokio::sync::{Mutex, Notify, mpsc};

use crate::template_host::RenderContext;

// ============================================================================
// Host Singleton
// ============================================================================

static HOST: tokio::sync::OnceCell<Arc<Host>> = tokio::sync::OnceCell::const_new();

/// The unified Host that owns all shared state.
pub struct Host {
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
    // Cell Readiness
    // -------------------------------------------------------------------------
    /// Cells that have completed the ready handshake.
    ready_cells: DashMap<String, cell_host_proto::ReadyMsg>,
    /// Notifier for when cells become ready.
    ready_notify: Notify,
}

impl Host {
    /// Get the global Host singleton.
    ///
    /// Panics if `Host::init()` hasn't been called.
    pub fn get() -> &'static Arc<Host> {
        HOST.get()
            .expect("Host::init() must be called before Host::get()")
    }

    /// Initialize the Host singleton. Call this once at startup.
    pub async fn init() -> &'static Arc<Host> {
        HOST.get_or_init(|| async {
            let (command_tx, command_rx) = mpsc::unbounded_channel();
            Arc::new(Host {
                render_contexts: DashMap::new(),
                next_context_id: AtomicU64::new(1),
                command_tx,
                command_rx: Mutex::new(Some(command_rx)),
                ready_cells: DashMap::new(),
                ready_notify: Notify::new(),
            })
        })
        .await
    }

    /// Check if the Host has been initialized.
    pub fn is_initialized() -> bool {
        HOST.get().is_some()
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
    // Cell Readiness
    // =========================================================================

    /// Record that a cell is ready.
    pub fn mark_cell_ready(&self, cell_name: String, msg: cell_host_proto::ReadyMsg) {
        self.ready_cells.insert(cell_name, msg);
        self.ready_notify.notify_waiters();
    }

    /// Check if a cell is ready.
    pub fn is_cell_ready(&self, cell_name: &str) -> bool {
        self.ready_cells.contains_key(cell_name)
    }

    /// Wait for a specific cell to be ready.
    pub async fn wait_for_cell_ready(&self, cell_name: &str) {
        loop {
            if self.ready_cells.contains_key(cell_name) {
                return;
            }
            self.ready_notify.notified().await;
        }
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
    /// This looks up the cell by name and returns a client. Currently delegates
    /// to `cells::get_cell_session()`. In the future, this will handle lazy
    /// spawning.
    pub fn client<C: CellClient>(&self) -> Option<C> {
        let handle = crate::cells::get_cell_session(C::CELL_NAME)?;
        Some(C::from_handle(handle))
    }

    /// Get a typed cell client, panicking if not available.
    pub fn client_or_panic<C: CellClient>(&self) -> C {
        self.client::<C>()
            .unwrap_or_else(|| panic!("Cell {} not available", C::CELL_NAME))
    }
}
