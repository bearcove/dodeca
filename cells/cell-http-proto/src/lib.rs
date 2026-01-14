//! RPC protocol for dodeca dev server cell
//!
//! Defines three RPC services:
//! - `ContentService`: Host implements, cell calls (for content from picante DB)
//! - `TcpTunnel`: Cell implements, host calls (for L4 TCP tunneling)
//! - `WebSocketTunnel`: Host implements, cell calls (for devtools WebSocket)
//!
//! # Tunnel Architecture
//!
//! Tunnels use roam's `Tunnel` type - a pair of `Tx<Vec<u8>>` and `Rx<Vec<u8>>`
//! channels for bidirectional byte streaming. The caller creates a tunnel pair
//! with `roam::tunnel_pair()`, passes one half via RPC, and uses the other locally.

use facet::Facet;
use roam::Tunnel;

// Re-export types from dodeca-protocol that are used in the RPC interface
pub use dodeca_protocol::{EvalResult, ScopeEntry, ScopeValue};

/// TCP tunnel service implemented by the cell.
///
/// The host calls `open()` for each incoming browser TCP connection,
/// passing a tunnel for bidirectional byte streaming.
///
/// Workflow:
/// 1. Host accepts TCP connection from browser
/// 2. Host creates `tunnel_pair()` → `(local, remote)`
/// 3. Host calls `TcpTunnelClient::open(remote)` via RPC
/// 4. Cell pumps `remote` ↔ internal HTTP server
/// 5. Host pumps `local` ↔ browser TCP socket
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait TcpTunnel {
    /// Open a new bidirectional TCP tunnel.
    ///
    /// The host passes a tunnel for data transfer. The cell serves HTTP
    /// on its end and pumps data through the tunnel.
    async fn open(&self, tunnel: Tunnel);
}

/// Content returned by the host for a given path
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ServeContent {
    /// HTML page content
    Html {
        content: String,
        route: String,
        generation: u64,
    },
    /// CSS stylesheet
    Css { content: String, generation: u64 },
    /// Static file with MIME type (immutable, cacheable)
    Static {
        content: Vec<u8>,
        mime: String,
        generation: u64,
    },
    /// Static file that should not be cached
    StaticNoCache {
        content: Vec<u8>,
        mime: String,
        generation: u64,
    },
    /// Redirect to another URL (302 temporary redirect)
    Redirect { location: String, generation: u64 },
    /// Not found - rendered 404 HTML page
    NotFound { html: String, generation: u64 },
}

/// Content service provided by the host
///
/// The cell calls these methods to get content from the host's picante DB.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait ContentService {
    /// Find content for a given path (HTML, CSS, static files, devtools assets)
    async fn find_content(&self, path: String) -> crate::ServeContent;

    /// Get scope entries for devtools (variable inspector)
    async fn get_scope(&self, route: String, path: Vec<String>) -> Vec<crate::ScopeEntry>;

    /// Evaluate an expression in the context of a route (REPL)
    async fn eval_expression(&self, route: String, expression: String) -> crate::EvalResult;
}

/// WebSocket tunnel service implemented by the host.
///
/// The cell calls `open()` when a browser opens a WebSocket connection
/// to the devtools endpoint (/_/ws), passing a tunnel for bidirectional
/// byte streaming. The host handles the devtools protocol directly.
///
/// Workflow:
/// 1. Browser opens WebSocket to cell at /_/ws
/// 2. Cell creates `tunnel_pair()` → `(local, remote)`
/// 3. Cell calls `WebSocketTunnelClient::open(remote)` via RPC
/// 4. Cell pumps `local` ↔ browser WebSocket
/// 5. Host handles devtools protocol on `remote`
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait WebSocketTunnel {
    /// Open a new WebSocket tunnel to the host.
    ///
    /// The cell passes a tunnel for data transfer. The host handles
    /// the devtools protocol (scope inspection, eval, reload broadcasts).
    async fn open(&self, tunnel: Tunnel);
}
