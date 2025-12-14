//! RPC protocol for dodeca JS plugin
//!
//! Defines services for JavaScript string literal rewriting.

use facet::Facet;
use std::collections::HashMap;

/// Result of JS processing operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum JsResult {
    /// Successfully processed JS
    Success { js: String },
    /// Error during processing
    Error { message: String },
}

/// Input for JS string literal rewriting
#[derive(Debug, Clone, Facet)]
pub struct JsRewriteInput {
    /// The JavaScript source code
    pub js: String,
    /// Map of old paths to new paths
    pub path_map: HashMap<String, String>,
}

/// JS processing service implemented by the plugin.
///
/// The host calls these methods to process JavaScript content.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait JsProcessor {
    /// Rewrite string literals in JavaScript that contain asset paths
    ///
    /// Parses JavaScript, finds string literals matching paths in path_map,
    /// and replaces them with the new paths.
    async fn rewrite_string_literals(&self, input: JsRewriteInput) -> JsResult;
}