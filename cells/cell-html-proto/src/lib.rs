//! RPC protocol for dodeca HTML processing cell
//!
//! This cell handles HTML transformations:
//! - URL rewriting (href, src, srcset attributes)
//! - Dead link marking
//! - Build info button injection

use facet::Facet;
use std::collections::{HashMap, HashSet};

// ============================================================================
// Result types
// ============================================================================

/// Result of HTML processing operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum HtmlResult {
    /// Successfully processed HTML
    Success { html: String },
    /// Successfully processed HTML with flag (e.g., had_dead_links, had_buttons)
    SuccessWithFlag { html: String, flag: bool },
    /// Error during processing
    Error { message: String },
}

/// Code execution metadata for build info buttons
#[derive(Debug, Clone, Facet)]
pub struct CodeExecutionMetadata {
    /// Rust compiler version
    pub rustc_version: String,
    /// Cargo version
    pub cargo_version: String,
    /// Target triple
    pub target: String,
    /// Build timestamp (ISO 8601)
    pub timestamp: String,
    /// Whether shared target cache was used
    pub cache_hit: bool,
    /// Platform (linux, macos, windows)
    pub platform: String,
    /// CPU architecture
    pub arch: String,
    /// Dependencies with versions
    pub dependencies: Vec<ResolvedDependency>,
}

/// A resolved dependency
#[derive(Debug, Clone, Facet)]
pub struct ResolvedDependency {
    pub name: String,
    pub version: String,
    pub source: DependencySource,
}

/// Source of a dependency
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum DependencySource {
    CratesIo,
    Git { url: String, commit: String },
    Path { path: String },
}

// ============================================================================
// Cell service (host calls these)
// ============================================================================

/// HTML processing service implemented by the CELL.
///
/// The host calls these methods to process HTML content.
/// All required data is passed as parameters.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait HtmlProcessor {
    /// Rewrite URLs in HTML (href, src, srcset attributes).
    ///
    /// Takes the HTML and a path map for rewriting URLs.
    async fn rewrite_urls(&self, html: String, path_map: HashMap<String, String>) -> HtmlResult;

    /// Mark dead internal links by adding `data-dead` attribute.
    ///
    /// Takes the HTML and a set of known routes.
    /// Returns the modified HTML and whether any dead links were found.
    async fn mark_dead_links(&self, html: String, known_routes: HashSet<String>) -> HtmlResult;

    /// Inject build info buttons into code blocks.
    ///
    /// Takes the HTML and a map of normalized code -> metadata.
    /// Returns the modified HTML and whether any buttons were added.
    async fn inject_build_info(
        &self,
        html: String,
        code_metadata: HashMap<String, CodeExecutionMetadata>,
    ) -> HtmlResult;
}
