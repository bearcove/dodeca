//! Typed interface definitions for the gingembre template renderer.
//!
//! This processor handles template rendering with typed host callbacks:
//! - Dodeca calls `TemplateRenderer::render()` to render a template
//! - The renderer calls back to `TemplateHost` for template loading, data resolution, and function calls
//!
//! This preserves fine-grained dependency tracking via picante without a
//! separate renderer process.

use facet::Facet;
use facet_value::Value;
use futures::future::BoxFuture;

// ============================================================================
// Error types
// ============================================================================

/// Source location for error reporting.
///
/// Contains all the information needed to render a pretty error with source context.
#[derive(Facet, Debug, Clone)]
pub struct ErrorLocation {
    /// Name of the source file (template name)
    pub filename: String,
    /// The full source text
    pub source: String,
    /// Byte offset where error starts
    pub offset: usize,
    /// Length of the error span in bytes
    pub length: usize,
}

/// A structured template error with source location.
///
/// This can be formatted to ANSI (for CLI) or HTML (for web) by the receiver.
#[derive(Facet, Debug, Clone)]
pub struct TemplateRenderError {
    /// Primary error message (without location prefix)
    pub message: String,
    /// Location in source (if applicable)
    pub location: Option<ErrorLocation>,
    /// Help text (if any)
    pub help: Option<String>,
}

// ============================================================================
// Result types
// ============================================================================

/// Result of a template render operation
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum RenderResult {
    /// Successfully rendered HTML output
    Success { html: String },
    /// Render failed with a structured error
    Error { error: TemplateRenderError },
}

/// Result of loading a template
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum LoadTemplateResult {
    /// Template found and loaded
    Found {
        /// The template source code
        source: String,
        /// Absolute path to the template file (for error reporting)
        absolute_path: String,
    },
    /// Template not found
    NotFound,
}

/// Result of resolving a data path
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum ResolveDataResult {
    /// Value found at path
    Found { value: Value },
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
    /// Expression evaluated successfully
    Success { value: Value },
    /// Evaluation failed with error
    Error { message: String },
}

/// Result of calling a template function on the host
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum CallFunctionResult {
    /// Function returned a value
    Success { value: Value },
    /// Function call failed with error
    Error { message: String },
}

// ============================================================================
// Context identifiers
// ============================================================================

/// Identifies a render context on the host side.
///
/// When Dodeca calls `render()`, it creates a context with templates,
/// data resolvers, etc. The context_id allows the renderer to reference this
/// context when making callbacks.
#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContextId(pub u64);

// ============================================================================
// Services
// ============================================================================

/// Renderer interface implemented by the template processor
///
/// The template renderer receives render requests and produces HTML output,
/// calling back to the host as needed for templates, data, and functions.
pub trait TemplateRenderer {
    /// Render a template by name.
    ///
    /// The renderer will call back to `TemplateHost` to:
    /// - Load the template source (and any parent templates for inheritance)
    /// - Resolve data values as they're accessed during rendering
    /// - Call template functions (get_url, get_section, etc.)
    ///
    /// # Arguments
    /// - `context_id`: Identifies the render context on the host
    /// - `template_name`: Name of the template to render
    /// - `initial_context`: Initial context variables (VObject)
    fn render(
        &self,
        context_id: ContextId,
        template_name: String,
        initial_context: Value,
    ) -> BoxFuture<'_, RenderResult>;

    /// Evaluate a standalone expression (for devtools REPL).
    ///
    /// # Arguments
    /// - `context_id`: Identifies the render context on the host
    /// - `expression`: The expression to evaluate
    /// - `context`: Context variables
    fn eval_expression(
        &self,
        context_id: ContextId,
        expression: String,
        context_vars: Value,
    ) -> BoxFuture<'_, EvalResult>;
}

/// Service implemented by the HOST (cell calls these methods)
///
/// Provides template loading, data resolution, and function calls with picante tracking.
/// Each call creates dependencies that allow incremental rebuilds.
pub trait TemplateHost {
    /// Load a template by name.
    ///
    /// Called when the renderer needs a template (main template, parent
    /// templates for inheritance, included templates, imported macros).
    ///
    /// The host should track this as a dependency for incremental builds.
    fn load_template(
        &self,
        context_id: ContextId,
        name: String,
    ) -> BoxFuture<'_, LoadTemplateResult>;

    /// Resolve a data value by path.
    ///
    /// Called when the renderer evaluates a lazy data reference like
    /// `data.versions.dodeca.version`. Each unique path becomes a
    /// separate dependency for fine-grained cache invalidation.
    ///
    /// # Arguments
    /// - `context_id`: The render context
    /// - `path`: Path segments (e.g., ["versions", "dodeca", "version"])
    fn resolve_data(
        &self,
        context_id: ContextId,
        path: Vec<String>,
    ) -> BoxFuture<'_, ResolveDataResult>;

    /// Get child keys at a data path.
    ///
    /// Called when iterating over a lazy container (for loops).
    /// Returns the keys/indices available at the path.
    fn keys_at(&self, context_id: ContextId, path: Vec<String>) -> BoxFuture<'_, KeysAtResult>;

    /// Call a template function on the host.
    ///
    /// Called when the template invokes a function like `get_url(path="/foo")`
    /// or `get_section(path="/blog")`. The host implements these functions
    /// with access to the full site tree.
    ///
    /// # Arguments
    /// - `context_id`: The render context
    /// - `name`: Function name (e.g., "get_url", "get_section")
    /// - `args`: Positional arguments
    /// - `kwargs`: Keyword arguments as (name, value) pairs
    fn call_function(
        &self,
        context_id: ContextId,
        name: String,
        args: Vec<Value>,
        kwargs: Vec<(String, Value)>,
    ) -> BoxFuture<'_, CallFunctionResult>;
}
