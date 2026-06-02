//! Shared protocol types for dodeca devtools
//!
//! This crate defines the protocol for communication between the dodeca
//! devtools overlay (running in the browser) and the dodeca server.
//!
//! # Architecture
//!
//! The devtools use vox RPC over WebSocket:
//! - Browser connects to `/_/ws` endpoint
//! - cell-http forwards vox RPC calls via `ForwardingDispatcher`
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
#[vox::service]
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
/// browser-based devtools overlay via vox RPC over WebSocket.
#[vox::service]
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

    /// Open a rendered source location in the developer's editor.
    async fn open_source(&self, source_file: String, line: u32) -> OpenSourceResult;

    /// Open a rendered markdown element by route and `data-sid`.
    async fn open_source_id(&self, route: String, sid: String) -> OpenSourceResult;

    /// Create a source stub for a dead link target, then open it in the editor.
    async fn open_dead_link(&self, route: String, target: DeadLinkTarget) -> OpenSourceResult;

    /// Load the raw markdown of the page at `route` for in-browser editing.
    ///
    /// `token` is the editor session token embedded in the `/_dodeca/edit/<page>`
    /// shell (minted only for verified editors). An invalid/expired token, or a
    /// later config change that revokes edit rights, yields `EditLoad::Denied`.
    async fn edit_load(&self, token: String, route: String) -> EditLoad;

    /// Render `buffer` overlaid on `source_key` (from `edit_load`), isolated
    /// from live state via a db snapshot — the editor's live preview. Byte
    /// identical to what publishing the buffer would produce.
    async fn edit_preview(&self, token: String, source_key: String, buffer: String) -> EditPreview;

    /// Commit a buffer authored as the editing user, then push. See
    /// [`EditSaveReq`] for the fields and the conflict semantics.
    async fn edit_save(&self, token: String, req: EditSaveReq) -> EditSave;

    /// Store an uploaded image next to the page being edited (committed as the
    /// user), and return the markdown snippet to insert. The on-disk file flows
    /// through the normal image pipeline (responsive AVIF/WebP/JXL variants).
    /// See [`EditUploadReq`].
    async fn edit_upload(&self, token: String, req: EditUploadReq) -> EditUpload;

    /// Tunnel a Language Server session for the in-browser editor.
    ///
    /// The browser's `monaco-languageclient` pipes raw JSON-RPC messages in on
    /// `client_to_server` (one message per chunk) and reads the server's
    /// messages from `server_to_client`. The host runs the **same `ddc lsp`
    /// binary** a desktop editor would, pointed at the live workspace, so online
    /// editing matches offline exactly — the host only translates between this
    /// message-framed channel and the subprocess's `Content-Length` framing.
    ///
    /// `token` gates the session to a verified editor. The call runs for the
    /// session's lifetime; a dropped channel ends the subprocess, which the
    /// client treats as a server restart.
    async fn lsp(
        &self,
        token: String,
        client_to_server: vox::Rx<String>,
        server_to_client: vox::Tx<String>,
    );

    /// Read a source file by its `file://` URI — backs the editor's file-system
    /// provider so go-to-definition (and opening any page) works. Content comes
    /// from the live db, not disk.
    async fn edit_read(&self, token: String, uri: String) -> EditRead;

    /// List every editable page (for the file tree).
    async fn edit_list(&self, token: String) -> EditList;
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

#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum OpenSourceResult {
    Ok,
    Err(String),
}

#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum DeadLinkTarget {
    Wiki { key: String, title: String },
    Internal { href: String, title: String },
}

/// Result of `DevtoolsService::edit_load`.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum EditLoad {
    /// The page's raw markdown, plus the `source_key` to pass back to
    /// `edit_preview`/`edit_save`, the normalized route, and the `file://` URI
    /// of the on-disk source (so the editor's model URI matches what the LSP
    /// keys documents by).
    Ok {
        source_key: String,
        route: String,
        uri: String,
        content: String,
        /// Git blob oid of the on-disk file (empty if it doesn't exist yet).
        /// Pass back to `edit_save` for optimistic-concurrency conflict checks.
        base: String,
    },
    /// Not a verified editor (bad/expired token, or rights revoked).
    Denied,
    /// No editable source owns that route.
    NotFound,
}

/// Result of `DevtoolsService::edit_preview`.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum EditPreview {
    /// Rendered HTML for the overlaid buffer.
    Ok {
        html: String,
        /// `data-sid` → source line map for the rendered body, so the editor can
        /// synchronize scrolling between the markdown source and the preview.
        source_map: Vec<SidLine>,
    },
    Denied,
    NotFound,
}

/// One rendered element's `data-sid` and the 1-indexed source line it starts at.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct SidLine {
    pub sid: String,
    pub line: u32,
}

/// Result of `DevtoolsService::edit_read`.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum EditRead {
    Ok {
        content: String,
        /// Git blob oid of the on-disk file (see `EditLoad::Ok::base`).
        base: String,
    },
    Denied,
    NotFound,
}

/// One editable page in the file tree.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct EditUploadReq {
    /// The page the image is being added to — determines which directory the
    /// file lands in (alongside the page source).
    pub source_key: String,
    /// Original filename; its base name + extension are sanitized and reused.
    pub filename: String,
    /// The raw image bytes.
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum EditUpload {
    /// Stored + committed. `markdown` is ready to insert at the cursor; `path` is
    /// the page-relative path the file was written to.
    Ok { markdown: String, path: String },
    /// Not a verified editor.
    Denied,
    /// No editable page owns `source_key`.
    NotFound,
    /// Write/commit/push failed; `message` is safe to show.
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Facet)]
pub struct EditSaveReq {
    /// Mount-prefixed source key (from `edit_load`).
    pub source_key: String,
    /// The full edited buffer to commit.
    pub buffer: String,
    /// Git blob oid the buffer was opened against (empty if the file didn't
    /// exist yet). The save is rejected with `EditSave::Conflict` if the on-disk
    /// file no longer hashes to `base` — i.e. it changed since it was loaded.
    pub base: String,
    /// Commit subject; empty falls back to a generated one.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Facet)]
pub struct EditEntry {
    /// Mount-prefixed source key (pass to `edit_preview`/`edit_save`).
    pub source_key: String,
    /// Normalized route, e.g. `/company`.
    pub route: String,
    /// `file://` URI of the on-disk source (the editor model URI).
    pub uri: String,
    /// Page title, best-effort (falls back to the route).
    pub title: String,
}

/// Result of `DevtoolsService::edit_list`.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum EditList {
    Ok { entries: Vec<EditEntry> },
    Denied,
}

/// Result of `DevtoolsService::edit_save`.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
pub enum EditSave {
    /// Saved; `commit` is the new commit hash and `base` is the saved file's new
    /// blob oid (the editor adopts it so the next save on this tab compares against
    /// what it just wrote, not the pre-save state).
    Ok {
        commit: String,
        base: String,
    },
    Denied,
    NotFound,
    /// The on-disk file changed since it was loaded; `current` is its new blob
    /// oid. The editor should reload (or merge) rather than clobber the change.
    Conflict {
        current: String,
    },
    /// The write/commit/push failed; `message` is safe to show the editor.
    Error {
        message: String,
    },
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

#[cfg(test)]
mod editor_descriptor_tests {
    //! The browser editor's TypeScript client is generated from this service
    //! descriptor (vox-codegen). Guard that the methods the editor relies on are
    //! actually present, so a rename can't silently break codegen.
    #[test]
    fn devtools_descriptor_exposes_editor_and_lsp_methods() {
        let descriptor = super::devtools_service_service_descriptor();
        let names: Vec<&str> = descriptor.methods.iter().map(|m| m.method_name).collect();
        for expected in ["edit_load", "edit_preview", "edit_save", "lsp"] {
            assert!(
                names.contains(&expected),
                "DevtoolsService is missing `{expected}`; have {names:?}"
            );
        }
    }
}
