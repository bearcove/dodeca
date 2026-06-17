//! Unified Host for dodeca.
//!
//! The Host is a singleton that owns all shared state:
//! - Render context registry (for template callbacks)
//! - TUI command forwarding
//!
//! Access via `Host::get()`.

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
                site_server: std::sync::OnceLock::new(),
                vite_port: std::sync::OnceLock::new(),
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
}
