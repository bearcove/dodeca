//! Protocol definitions for the gingembre template rendering cell.
//!
//! This cell handles template rendering with bidirectional RPC:
//! - Host calls `TemplateRenderer::render()` to render a template
//! - Cell calls back to `TemplateHost` for template loading and data resolution
//!
//! This enables fine-grained dependency tracking via picante while keeping
//! the template engine in a separate process (reducing main binary compile time).

use facet::Facet;

// ============================================================================
// Result types
// ============================================================================

/// Result of a template render operation
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum RenderResult {
    /// Successfully rendered HTML output
    Success { html: String },
    /// Render failed with an error message (may contain ANSI formatting)
    Error { message: String },
}

/// Result of loading a template
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum LoadTemplateResult {
    /// Template found and loaded
    Found { source: String },
    /// Template not found
    NotFound,
}

/// Result of resolving a data path
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum ResolveDataResult {
    /// Value found at path (serialized as JSON for cross-process transport)
    Found { json_value: String },
    /// Path not found in data tree
    NotFound,
}

/// Result of getting keys at a data path
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum KeysAtResult {
    /// Keys found at path
    Found { keys: Vec<String> },
    /// Path not found or not a container
    NotFound,
}

/// Result of evaluating an expression (for devtools)
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum EvalResult {
    /// Expression evaluated successfully (serialized as JSON)
    Success { json_value: String },
    /// Evaluation failed with error
    Error { message: String },
}

// ============================================================================
// Context identifiers
// ============================================================================

/// Identifies a render context on the host side.
///
/// When the host calls `render()`, it creates a context with templates,
/// data resolvers, etc. The context_id allows the cell to reference
/// this context when making callbacks.
#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContextId(pub u64);

// ============================================================================
// Services
// ============================================================================

/// Service implemented by the CELL (host calls these methods)
///
/// The template renderer receives render requests and produces HTML output,
/// calling back to the host as needed for templates and data.
#[rapace::service]
pub trait TemplateRenderer {
    /// Render a template by name.
    ///
    /// The cell will call back to `TemplateHost` to:
    /// - Load the template source (and any parent templates for inheritance)
    /// - Resolve data values as they're accessed during rendering
    ///
    /// # Arguments
    /// - `context_id`: Identifies the render context on the host
    /// - `template_name`: Name of the template to render
    /// - `initial_context_json`: JSON-serialized initial context variables
    async fn render(
        &self,
        context_id: ContextId,
        template_name: String,
        initial_context_json: String,
    ) -> RenderResult;

    /// Evaluate a standalone expression (for devtools REPL).
    ///
    /// # Arguments
    /// - `context_id`: Identifies the render context on the host
    /// - `expression`: The expression to evaluate
    /// - `context_json`: JSON-serialized context variables
    async fn eval_expression(
        &self,
        context_id: ContextId,
        expression: String,
        context_json: String,
    ) -> EvalResult;
}

/// Service implemented by the HOST (cell calls these methods)
///
/// Provides template loading and data resolution with picante tracking.
/// Each call creates dependencies that allow incremental rebuilds.
#[rapace::service]
pub trait TemplateHost {
    /// Load a template by name.
    ///
    /// Called when the renderer needs a template (main template, parent
    /// templates for inheritance, included templates, imported macros).
    ///
    /// The host should track this as a dependency for incremental builds.
    async fn load_template(&self, context_id: ContextId, name: String) -> LoadTemplateResult;

    /// Resolve a data value by path.
    ///
    /// Called when the renderer evaluates a lazy data reference like
    /// `data.versions.dodeca.version`. Each unique path becomes a
    /// separate dependency for fine-grained cache invalidation.
    ///
    /// # Arguments
    /// - `context_id`: The render context
    /// - `path`: Path segments (e.g., ["versions", "dodeca", "version"])
    async fn resolve_data(&self, context_id: ContextId, path: Vec<String>) -> ResolveDataResult;

    /// Get child keys at a data path.
    ///
    /// Called when iterating over a lazy container (for loops).
    /// Returns the keys/indices available at the path.
    async fn keys_at(&self, context_id: ContextId, path: Vec<String>) -> KeysAtResult;
}
