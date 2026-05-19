//! RPC protocol for dodeca dev server cell
//!
//! Defines two RPC services:
//! - `ContentService`: Host implements, cell calls (for content from picante DB)
//! - `TcpTunnel`: Cell implements, host calls (for L4 TCP tunneling)
//!
//! # Tunnel Architecture
//!
//! A tunnel is just a pair of vox channels for bidirectional byte streaming:
//! the host passes the cell an `Rx<Vec<u8>>` (browser→cell bytes) and a
//! `Tx<Vec<u8>>` (cell→browser bytes). The host keeps the opposite halves and
//! pumps them against the browser TCP socket.

use facet::Facet;
use vox::{Rx, Tx};

// Re-export types from dodeca-protocol that are used in the RPC interface
pub use dodeca_protocol::{EvalResult, ScopeEntry, ScopeValue};

/// TCP tunnel service implemented by the cell.
///
/// The host calls `open()` for each incoming browser TCP connection,
/// passing a tunnel for bidirectional byte streaming.
///
/// Workflow:
/// 1. Host accepts TCP connection from browser
/// 2. Host creates two `vox::channel::<Vec<u8>>()` pairs (one per direction)
/// 3. Host calls `TcpTunnelClient::open(inbound_rx, outbound_tx)` via RPC,
///    keeping `inbound_tx` (browser→cell) and `outbound_rx` (cell→browser)
/// 4. Cell pumps `inbound`/`outbound` ↔ its internal HTTP server
/// 5. Host pumps its halves ↔ the browser TCP socket
#[allow(async_fn_in_trait)]
#[vox::service]
pub trait TcpTunnel {
    /// Open a new bidirectional TCP tunnel.
    ///
    /// `inbound` carries browser→cell bytes; `outbound` carries cell→browser
    /// bytes. The cell serves HTTP on its end and pumps data through them.
    async fn open(&self, inbound: Rx<Vec<u8>>, outbound: Tx<Vec<u8>>);
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
#[vox::service]
pub trait ContentService {
    /// Find content for a given path (HTML, CSS, static files, devtools assets)
    async fn find_content(&self, path: String) -> crate::ServeContent;

    /// Get scope entries for devtools (variable inspector)
    async fn get_scope(&self, route: String, path: Vec<String>) -> Vec<crate::ScopeEntry>;

    /// Evaluate an expression in the context of a route (REPL)
    async fn eval_expression(&self, route: String, expression: String) -> crate::EvalResult;
}
