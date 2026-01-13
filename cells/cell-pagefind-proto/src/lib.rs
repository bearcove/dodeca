//! RPC protocol for dodeca Pagefind cell
//!
//! Defines services for search indexing.

use facet::Facet;

/// A page to be indexed
#[derive(Debug, Clone, Facet)]
pub struct SearchPage {
    /// URL of the page (e.g., "/guide/")
    pub url: String,
    /// HTML content of the page
    pub html: String,
}

/// Output file from pagefind
#[derive(Debug, Clone, Facet)]
pub struct SearchFile {
    /// Path where the file should be served (e.g., "/pagefind/pagefind.js")
    pub path: String,
    /// File contents
    pub contents: Vec<u8>,
}

/// Input for building search index
#[derive(Debug, Clone, Facet)]
pub struct SearchIndexInput {
    /// Pages to index
    pub pages: Vec<SearchPage>,
}

/// Output from building search index
#[derive(Debug, Clone, Facet)]
pub struct SearchIndexOutput {
    /// Generated search files
    pub files: Vec<SearchFile>,
}

/// Result of search indexing
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum SearchIndexResult {
    /// Successfully built search index
    Success { output: SearchIndexOutput },
    /// Error during indexing
    Error { message: String },
}

/// Search indexing service implemented by the cell.
///
/// The host calls these methods to build search indexes.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait SearchIndexer {
    /// Build a search index from HTML pages
    ///
    /// Takes a list of pages (url + html) and returns the pagefind output files.
    async fn build_search_index(&self, input: SearchIndexInput) -> SearchIndexResult;
}
