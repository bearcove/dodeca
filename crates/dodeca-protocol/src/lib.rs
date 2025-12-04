//! Shared protocol types for dodeca devtools
//!
//! Uses a JSON-RPC style protocol for bidirectional communication.

use serde::{Deserialize, Serialize};

mod ansi;
pub use ansi::ansi_to_html;

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Full page reload requested
    Reload,

    /// CSS hot reload
    CssChanged { path: String },

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
        result: Result<ScopeValue, String>,
    },
}

/// Messages sent from client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceSnippet {
    /// Lines of source code
    pub lines: Vec<SourceLine>,
    /// Which line (1-indexed) contains the error
    pub error_line: u32,
}

/// A line of source code
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceLine {
    /// Line number (1-indexed)
    pub number: u32,
    /// Line content
    pub content: String,
}

/// An entry in the scope tree
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopeEntry {
    /// Variable name
    pub name: String,
    /// Value (or summary for complex types)
    pub value: ScopeValue,
    /// Whether this entry can be expanded (has children)
    pub expandable: bool,
}

/// A value in the template scope
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
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
