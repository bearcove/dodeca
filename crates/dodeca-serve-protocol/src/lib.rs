//! RPC protocol for dodeca dev server plugin
//!
//! Defines the ContentService trait that the host implements and the plugin calls.

use facet::Facet;

// Re-export types from dodeca-protocol that are used in the RPC interface
pub use dodeca_protocol::{EvalResult, ScopeEntry, ScopeValue};

/// Content returned by the host for a given path
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ServeContent {
    /// HTML page content
    Html { content: String, route: String },
    /// CSS stylesheet
    Css { content: String },
    /// Static file with MIME type (immutable, cacheable)
    Static { content: Vec<u8>, mime: String },
    /// Static file that should not be cached
    StaticNoCache { content: Vec<u8>, mime: String },
    /// Search index file (pagefind)
    Search { content: Vec<u8>, mime: String },
    /// Not found - includes similar routes for suggestions
    NotFound { similar_routes: Vec<(String, String)> },
}

// Generate RPC client/server code
rapace::service! {
    /// Content service provided by the host
    ///
    /// The plugin calls these methods to get content from the host's Salsa DB.
    pub trait ContentService {
        /// Find content for a given path (HTML, CSS, static files, devtools assets)
        async fn find_content(path: String) -> ServeContent;

        /// Get scope entries for devtools inspector
        async fn get_scope(route: String, path: Vec<String>) -> Vec<ScopeEntry>;

        /// Evaluate an expression in the template context
        async fn eval_expression(route: String, expression: String) -> EvalResult;
    }
}
