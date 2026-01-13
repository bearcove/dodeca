//! Unified host service protocol for dodeca cells.
//!
//! This crate defines a single `HostService` trait that combines all host-facing
//! RPC methods. Every cell connects to the same host service - no per-cell
//! dispatcher configuration needed.

use roam::{Tunnel, Tx};

// Re-export types from other proto crates
pub use cell_gingembre_proto::{
    CallFunctionResult, ContextId, KeysAtResult, LoadTemplateResult, ResolveDataResult,
};
pub use cell_http_proto::{EvalResult, ScopeEntry, ServeContent};
pub use cell_lifecycle_proto::{ReadyAck, ReadyMsg};
pub use cell_tui_proto::{
    BindMode, BuildProgress, CommandResult, EventKind, LogEvent, LogLevel, ServerCommand,
    ServerStatus, TaskProgress, TaskStatus,
};
pub use facet_value::Value;

/// Unified host service that all cells can call.
///
/// This combines all host-facing RPC methods into a single service:
/// - Cell lifecycle (readiness handshake)
/// - Template host (for gingembre template rendering)
/// - Content service (for HTTP cell serving content)
/// - WebSocket tunnel (for devtools)
/// - TUI host (for TUI cell updates)
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
    // TUI Host (for TUI cell)
    // =========================================================================

    /// Subscribe to build progress updates.
    async fn subscribe_progress(&self, tx: Tx<BuildProgress>);

    /// Subscribe to log events.
    async fn subscribe_events(&self, tx: Tx<LogEvent>);

    /// Subscribe to server status updates.
    async fn subscribe_server_status(&self, tx: Tx<ServerStatus>);

    /// Send a command from TUI to the server.
    async fn send_command(&self, command: ServerCommand) -> CommandResult;
}
