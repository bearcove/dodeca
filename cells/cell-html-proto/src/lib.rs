//! RPC protocol for dodeca HTML processing cell
//!
//! This cell handles all HTML transformations:
//! - Parsing and serialization (via facet-format-html)
//! - URL rewriting (href, src, srcset attributes)
//! - Dead link marking
//! - Code button injection (copy + build info)
//! - Script/style injection
//! - Inline CSS/JS minification (via callbacks to host)
//! - HTML structural minification
//! - DOM diffing for live reload

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

/// Result of CSS minification (from host)
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum MinifyCssResult {
    /// Successfully minified
    Success { css: String },
    /// Minification failed (return original)
    Error { message: String },
}

/// Result of JS minification (from host)
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum MinifyJsResult {
    /// Successfully minified
    Success { js: String },
    /// Minification failed (return original)
    Error { message: String },
}

// ============================================================================
// Processing input types
// ============================================================================

/// Options for HTML minification
#[derive(Debug, Clone, Default, Facet)]
pub struct MinifyOptions {
    /// Minify inline `<style>` content via host callback
    pub minify_inline_css: bool,
    /// Minify inline `<script>` content via host callback
    pub minify_inline_js: bool,
    /// Minify HTML structure (remove unnecessary whitespace)
    pub minify_html: bool,
}

/// Typed injection for HTML documents
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum Injection {
    /// Inject a `<style>` element into `<head>`
    HeadStyle { css: String },
    /// Inject a `<script>` element into `<head>`
    HeadScript { js: String, module: bool },
    /// Inject a `<script>` element at end of `<body>` (for deferred loading)
    BodyScript { js: String, module: bool },
}

/// Input for the unified process() method
#[derive(Debug, Clone, Facet)]
pub struct HtmlProcessInput {
    /// The HTML to process
    pub html: String,

    /// URL rewriting map (old path -> new path)
    #[facet(default)]
    pub path_map: Option<HashMap<String, String>>,

    /// Known routes for dead link detection
    #[facet(default)]
    pub known_routes: Option<HashSet<String>>,

    /// Code execution metadata for build info buttons
    #[facet(default)]
    pub code_metadata: Option<HashMap<String, CodeExecutionMetadata>>,

    /// Content to inject into the document
    #[facet(default)]
    pub injections: Vec<Injection>,

    /// Minification options
    #[facet(default)]
    pub minify: Option<MinifyOptions>,
}

/// Result of the unified process() method
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum HtmlProcessResult {
    /// Successfully processed HTML
    Success {
        html: String,
        /// Whether any dead links were found
        had_dead_links: bool,
        /// Whether any code buttons were injected
        had_code_buttons: bool,
        /// All href values from <a> elements (for link checking)
        hrefs: Vec<String>,
        /// All id attributes from any element (for fragment validation)
        element_ids: Vec<String>,
    },
    /// Error during processing
    Error { message: String },
}

// ============================================================================
// Code execution metadata (for build info buttons)
// ============================================================================

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
/// For operations that need CSS/JS minification, the cell calls back
/// to the HtmlHost service.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait HtmlProcessor {
    /// Unified HTML processing: parse, transform, serialize.
    ///
    /// This method handles all HTML transformations in a single call:
    /// - URL rewriting (if path_map provided)
    /// - Dead link marking (if known_routes provided)
    /// - Code button injection (if code_metadata provided)
    /// - Content injection (if injections provided)
    /// - Inline CSS/JS minification (if minify options set, calls HtmlHost)
    /// - HTML structural minification (if minify.minify_html set)
    async fn process(&self, input: HtmlProcessInput) -> HtmlProcessResult;

    // === Legacy methods (for backward compatibility during migration) ===

    /// Rewrite URLs in HTML (href, src, srcset attributes).
    async fn rewrite_urls(&self, html: String, path_map: HashMap<String, String>) -> HtmlResult;

    /// Mark dead internal links by adding `data-dead` attribute.
    async fn mark_dead_links(&self, html: String, known_routes: HashSet<String>) -> HtmlResult;

    /// Inject copy buttons (and optionally build info buttons) into all pre blocks.
    async fn inject_code_buttons(
        &self,
        html: String,
        code_metadata: HashMap<String, CodeExecutionMetadata>,
    ) -> HtmlResult;

    /// Extract links and element IDs from HTML for link checking.
    ///
    /// Returns all href values from `<a>` elements and all id attributes from any element.
    /// Use this instead of regex-based extraction.
    async fn extract_links(&self, html: String) -> ExtractedLinks;
}

/// Links and element IDs extracted from HTML
#[derive(Debug, Clone, Default, Facet)]
pub struct ExtractedLinks {
    /// All href values from `<a>` elements
    pub hrefs: Vec<String>,
    /// All id attribute values from any element
    pub element_ids: Vec<String>,
}

// ============================================================================
// Host service (cell calls these)
// ============================================================================

/// Service implemented by the HOST (dodeca) that the cell can call.
///
/// This enables the cell to delegate CSS/JS minification to specialized cells
/// without needing direct cell-to-cell communication.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait HtmlHost {
    /// Minify CSS content.
    ///
    /// The host dispatches this to cell-css.
    async fn minify_css(&self, css: String) -> MinifyCssResult;

    /// Minify JavaScript content.
    ///
    /// The host dispatches this to cell-js.
    async fn minify_js(&self, js: String) -> MinifyJsResult;
}
