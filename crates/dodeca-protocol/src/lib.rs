//! Shared protocol types for dodeca devtools
//!
//! This crate defines the protocol for communication between the dodeca
//! devtools overlay (running in the browser) and the dodeca server.
//!
//! # Architecture
//!
//! The devtools use roam RPC over WebSocket:
//! - Browser connects to `/_/ws` endpoint
//! - cell-http forwards roam RPC calls via `ForwardingDispatcher`
//! - Host implements `DevtoolsService`
//!

use facet::Facet;

mod ansi;
pub use ansi::ansi_to_html;

// Re-export for consumers
pub use facet_postcard;

// ============================================================================
// RPC Service Definition
// ============================================================================

/// Service implemented by the browser, called by the server to push events.
///
/// This is the reverse of the traditional client-server model - the server
/// calls methods on the browser when events occur (patches, errors, etc.)
#[roam::service]
pub trait BrowserService {
    /// Called by the server when a devtools event occurs.
    ///
    /// Events include:
    /// - Live reload notifications
    /// - CSS hot reload
    /// - DOM patches
    /// - Template errors
    async fn on_event(&self, event: DevtoolsEvent);
}

/// Service for devtools communication between browser and server.
///
/// This service is implemented by the dodeca host and called by the
/// browser-based devtools overlay via roam RPC over WebSocket.
#[roam::service]
pub trait DevtoolsService {
    /// Register this browser connection for a route.
    ///
    /// After calling this, the server will call `BrowserService::on_event()`
    /// on this connection whenever events occur for this route.
    async fn subscribe(&self, route: String);

    /// Get scope entries for the current route.
    ///
    /// - `path`: Optional path into the scope tree (e.g., `["data", "items"]`)
    ///   If None, returns top-level scope entries.
    async fn get_scope(&self, path: Option<Vec<String>>) -> Vec<ScopeEntry>;

    /// Evaluate an expression in a snapshot's context.
    ///
    /// - `snapshot_id`: The snapshot to evaluate in (from ErrorInfo)
    /// - `expression`: The expression to evaluate
    async fn eval(&self, snapshot_id: String, expression: String) -> EvalResult;

    /// Dismiss an error notification.
    ///
    /// Called when the user acknowledges an error.
    async fn dismiss_error(&self, route: String);
}

/// Events pushed from server to browser devtools.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum DevtoolsEvent {
    /// Full page reload requested
    Reload,

    /// CSS file changed - hot reload it
    CssChanged { path: String },

    /// DOM patches to apply incrementally (facet-postcard blob), for a specific route
    Patches { route: String, patches: Vec<u8> },

    /// A template error occurred
    Error(ErrorInfo),

    /// Error was resolved (template now renders successfully)
    ErrorResolved { route: String },
}

// ============================================================================
// Shared Types
// ============================================================================

/// Result of expression evaluation (facet-compatible)
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum EvalResult {
    Ok(ScopeValue),
    Err(String),
}

impl From<Result<ScopeValue, String>> for EvalResult {
    fn from(r: Result<ScopeValue, String>) -> Self {
        match r {
            Ok(v) => EvalResult::Ok(v),
            Err(e) => EvalResult::Err(e),
        }
    }
}

impl From<EvalResult> for Result<ScopeValue, String> {
    fn from(r: EvalResult) -> Self {
        match r {
            EvalResult::Ok(v) => Ok(v),
            EvalResult::Err(e) => Err(e),
        }
    }
}

/// Information about a template error
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct ErrorInfo {
    /// Route where the error occurred
    pub route: String,

    /// Error message
    pub message: String,

    /// Template file where error occurred (if known)
    pub template: Option<String>,

    /// Line number in template (if known)
    pub line: Option<u32>,

    /// Column number in template (if known)
    pub column: Option<u32>,

    /// Source code snippet around the error
    pub source_snippet: Option<SourceSnippet>,

    /// Snapshot ID for querying scope/evaluating expressions
    pub snapshot_id: String,

    /// Available variables at the error location
    pub available_variables: Vec<String>,
}

/// Source code snippet with context
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct SourceSnippet {
    /// Lines of source code
    pub lines: Vec<SourceLine>,
    /// Which line (1-indexed) contains the error
    pub error_line: u32,
}

/// A line of source code
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct SourceLine {
    /// Line number (1-indexed)
    pub number: u32,
    /// Line content
    pub content: String,
}

/// An entry in the scope tree
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct ScopeEntry {
    /// Variable name
    pub name: String,
    /// Value (or summary for complex types)
    pub value: ScopeValue,
    /// Whether this entry can be expanded (has children)
    pub expandable: bool,
}

/// A value in the template scope
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum ScopeValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array { length: usize, preview: String },
    Object { fields: usize, preview: String },
}

impl ScopeValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            ScopeValue::Null => "null",
            ScopeValue::Bool(_) => "bool",
            ScopeValue::Number(_) => "number",
            ScopeValue::String(_) => "string",
            ScopeValue::Array { .. } => "array",
            ScopeValue::Object { .. } => "object",
        }
    }

    pub fn display(&self) -> String {
        match self {
            ScopeValue::Null => "null".to_string(),
            ScopeValue::Bool(b) => b.to_string(),
            ScopeValue::Number(n) => n.to_string(),
            ScopeValue::String(s) => format!("\"{}\"", s),
            ScopeValue::Array { preview, .. } => preview.clone(),
            ScopeValue::Object { preview, .. } => preview.clone(),
        }
    }
}
