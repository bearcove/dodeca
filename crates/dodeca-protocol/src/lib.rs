//! Shared protocol types for dodeca devtools
//!
//! Uses facet-postcard for binary serialization over WebSocket.

use facet::Facet;

mod ansi;
pub use ansi::ansi_to_html;

// Re-export for consumers
pub use facet_postcard;

/// Messages sent from server to client
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum ServerMessage {
    /// Full page reload requested
    Reload,

    /// CSS hot reload
    CssChanged { path: String },

    /// DOM patches to apply
    Patches(Vec<Patch>),

    /// A template error occurred
    Error(ErrorInfo),

    /// Error was resolved (template now renders successfully)
    ErrorResolved { route: String },

    /// Response to a scope query
    ScopeResponse {
        request_id: u32,
        scope: Vec<ScopeEntry>,
    },

    /// Response to an expression evaluation
    EvalResponse {
        request_id: u32,
        result: EvalResult,
    },
}

/// A path to a node in the DOM tree
/// e.g., [0, 2, 1] means: body's child 0, then child 2, then child 1
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct NodePath(pub Vec<usize>);

/// Operations to transform the DOM
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum Patch {
    /// Replace node at path with new HTML
    Replace { path: NodePath, html: String },

    /// Insert HTML before the node at path
    InsertBefore { path: NodePath, html: String },

    /// Insert HTML after the node at path
    InsertAfter { path: NodePath, html: String },

    /// Append HTML as last child of node at path
    AppendChild { path: NodePath, html: String },

    /// Remove the node at path
    Remove { path: NodePath },

    /// Update text content of node at path
    SetText { path: NodePath, text: String },

    /// Set attribute on node at path
    SetAttribute {
        path: NodePath,
        name: String,
        value: String,
    },

    /// Remove attribute from node at path
    RemoveAttribute { path: NodePath, name: String },
}

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

/// Messages sent from client to server
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum ClientMessage {
    /// Tell server which route we're viewing
    Route { path: String },

    /// Request the scope for a route/snapshot
    GetScope {
        request_id: u32,
        snapshot_id: Option<String>,
        path: Option<Vec<String>>, // Path into the scope tree
    },

    /// Evaluate an expression in a snapshot's context
    Eval {
        request_id: u32,
        snapshot_id: String,
        expression: String,
    },

    /// Dismiss an error (user acknowledged it)
    DismissError { route: String },
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
