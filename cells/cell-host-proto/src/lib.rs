//! Unified host service protocol for dodeca cells.
//!
//! This crate defines a single `HostService` trait that combines all host-facing
//! RPC methods. Every cell connects to the same host service - no per-cell
//! dispatcher configuration needed.

use facet::Facet;
use roam::Tunnel;
use std::collections::HashMap;

// Re-export types from other proto crates
pub use cell_gingembre_proto::{
    CallFunctionResult, ContextId, KeysAtResult, LoadTemplateResult, ResolveDataResult,
};
pub use cell_http_proto::{EvalResult, ScopeEntry, ServeContent};
pub use cell_lifecycle_proto::{ReadyAck, ReadyMsg};
pub use cell_tui_proto::{CommandResult, ServerCommand};
pub use facet_value::Value;

// ============================================================================
// HTML Host Result Types (for HTML cell callbacks)
// ============================================================================

/// Result of CSS minification
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum MinifyCssResult {
    Success { css: String },
    Error { message: String },
}

/// Result of JS minification
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum MinifyJsResult {
    Success { js: String },
    Error { message: String },
}

/// Result of CSS URL rewriting
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ProcessCssResult {
    Success { css: String },
    Error { message: String },
}

/// Result of JS URL rewriting
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ProcessJsResult {
    Success { js: String },
    Error { message: String },
}

/// Unified host service that all cells can call.
///
/// This combines all host-facing RPC methods into a single service:
/// - Cell lifecycle (readiness handshake)
/// - Template host (for gingembre template rendering)
/// - Content service (for HTTP cell serving content)
/// - WebSocket tunnel (for devtools)
/// - TUI commands (TUI cell → host direction)
///
/// Note: Host → TUI updates use TuiDisplay service (defined in cell-tui-proto).
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait HostService {
    // =========================================================================
    // Cell Lifecycle
    // =========================================================================

    /// Cell calls this after starting its demux loop to signal it's ready for RPC requests.
    async fn ready(&self, msg: ReadyMsg) -> ReadyAck;

    // =========================================================================
    // Template Host (for gingembre)
    // =========================================================================

    /// Load a template by name.
    async fn load_template(&self, context_id: ContextId, name: String) -> LoadTemplateResult;

    /// Resolve a data value by path.
    async fn resolve_data(&self, context_id: ContextId, path: Vec<String>) -> ResolveDataResult;

    /// Get child keys at a data path.
    async fn keys_at(&self, context_id: ContextId, path: Vec<String>) -> KeysAtResult;

    /// Call a template function on the host.
    async fn call_function(
        &self,
        context_id: ContextId,
        name: String,
        args: Vec<Value>,
        kwargs: Vec<(String, Value)>,
    ) -> CallFunctionResult;

    // =========================================================================
    // Content Service (for HTTP cell)
    // =========================================================================

    /// Find content for a given path.
    async fn find_content(&self, path: String) -> ServeContent;

    /// Get scope entries for devtools.
    async fn get_scope(&self, route: String, path: Vec<String>) -> Vec<ScopeEntry>;

    /// Evaluate an expression in the context of a route.
    async fn eval_expression(&self, route: String, expression: String) -> EvalResult;

    // =========================================================================
    // WebSocket Tunnel (for devtools)
    // =========================================================================

    /// Open a WebSocket tunnel to the host for devtools.
    async fn open_websocket(&self, tunnel: Tunnel);

    // =========================================================================
    // TUI Commands (TUI cell → host)
    // =========================================================================

    /// Send a command from TUI to the server.
    async fn send_command(&self, command: ServerCommand) -> CommandResult;

    /// Request graceful shutdown.
    async fn quit(&self);

    // =========================================================================
    // Vite Integration
    // =========================================================================

    /// Get the Vite dev server port if one is running.
    /// Returns None if Vite is not enabled for this project.
    async fn get_vite_port(&self) -> Option<u16>;

    // =========================================================================
    // HTML Host (for HTML cell callbacks)
    // =========================================================================

    /// Minify CSS content via CSS cell.
    async fn minify_css(&self, css: String) -> MinifyCssResult;

    /// Minify JavaScript content via JS cell.
    async fn minify_js(&self, js: String) -> MinifyJsResult;

    /// Process inline CSS: rewrite URLs using path_map via CSS cell.
    async fn process_inline_css(
        &self,
        css: String,
        path_map: HashMap<String, String>,
    ) -> ProcessCssResult;

    /// Process inline JS: rewrite string literals using path_map via JS cell.
    async fn process_inline_js(
        &self,
        js: String,
        path_map: HashMap<String, String>,
    ) -> ProcessJsResult;
}
