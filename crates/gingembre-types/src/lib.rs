//! Shared types for gingembre template engine plugin interface
//!
//! This crate defines the multi-round protocol for template rendering.
//! The plugin can suspend rendering to request templates or data from the host,
//! enabling precise dependency tracking.
//!
//! # Protocol Overview
//!
//! The plugin uses a trampoline pattern:
//! 1. Host calls `render_start` with the initial template and context
//! 2. Plugin attempts to render, caching templates and data as it goes
//! 3. When a cache miss occurs, plugin returns `NeedTemplate` or `NeedData`
//! 4. Host provides the requested resource via `render_continue`
//! 5. Plugin resumes rendering with the new resource cached
//! 6. Repeat until rendering completes with `Done` or fails with `Error`

use facet::Facet;
use facet_value::Value;

/// Opaque continuation token containing cached state for resuming render.
/// This includes all templates and data loaded so far.
#[derive(Facet, Clone, Debug)]
pub struct RenderContinuation {
    /// Serialized render cache state
    pub data: Vec<u8>,
}

/// Input to start rendering a template
#[derive(Facet, Debug)]
pub struct RenderStartInput {
    /// Name/path of the template to render
    pub template_name: String,
    /// Source code of the template
    pub template_source: String,
    /// Initial context variables (concrete values only, for non-lazy data)
    pub context: Value,
    /// Names of context variables that should use lazy data resolution.
    /// When any of these variables (or their fields) are accessed,
    /// the plugin will request the data path from the host.
    pub lazy_roots: Vec<String>,
}

/// Input to continue rendering after providing requested data
#[derive(Facet, Debug)]
pub struct RenderContinueInput {
    /// The continuation token from the previous response
    pub continuation: RenderContinuation,
    /// The response to the previous request
    pub response: DataResponse,
}

/// Response from the host to a data request
#[derive(Facet, Debug)]
#[repr(u8)]
pub enum DataResponse {
    /// Response to a template request
    Template {
        /// The requested template name
        name: String,
        /// The template source, or None if not found
        source: Option<String>,
    },
    /// Response to a data path request
    Data {
        /// The requested path
        path: Vec<String>,
        /// The value at that path, or None if not found
        value: Option<Value>,
    },
    /// Response to a data keys request (for iteration)
    DataKeys {
        /// The requested path
        path: Vec<String>,
        /// The keys/indices at that path, or None if not a container
        keys: Option<Vec<String>>,
    },
    /// Response to a data length request
    DataLen {
        /// The requested path
        path: Vec<String>,
        /// The length at that path, or None if not a container
        len: Option<u64>,
    },
}

/// Result from the plugin
#[derive(Facet, Debug)]
#[repr(u8)]
pub enum RenderResult {
    /// Rendering completed successfully
    Done {
        /// The rendered HTML output
        html: String,
    },

    /// Plugin needs a template to continue rendering (extends, include, import)
    NeedTemplate {
        /// Path/name of the needed template
        name: String,
        /// Continuation token to resume after providing the template
        continuation: RenderContinuation,
    },

    /// Plugin needs a data value to continue rendering
    NeedData {
        /// Path through the data tree (e.g., ["data", "versions", "dodeca", "version"])
        path: Vec<String>,
        /// Continuation token to resume after providing the value
        continuation: RenderContinuation,
    },

    /// Plugin needs the keys at a data path (for iteration)
    NeedDataKeys {
        /// Path through the data tree
        path: Vec<String>,
        /// Continuation token to resume after providing the keys
        continuation: RenderContinuation,
    },

    /// Plugin needs the length at a data path (for |length filter, etc.)
    NeedDataLen {
        /// Path through the data tree
        path: Vec<String>,
        /// Continuation token to resume after providing the length
        continuation: RenderContinuation,
    },

    /// Rendering failed with an error
    Error {
        /// Error message
        message: String,
    },
}
