//! RPC protocol for dodeca's full-text search indexing cell.
//!
//! The cell receives the rendered HTML of every page and returns the complete
//! set of search-index files to write under `/search/`. The on-disk format of
//! those file *contents* lives in `dodeca-search-format`; this proto only
//! describes the RPC envelope.

use facet::Facet;

/// A rendered page to be indexed.
#[derive(Debug, Clone, Facet)]
pub struct SearchPage {
    /// Site-absolute URL of the page, e.g. `/guide/intro/`.
    pub url: String,
    /// Name of the source this page came from (empty for a single-source site).
    /// Carried into the index so search can scope to the current site.
    pub source: String,
    /// Rendered HTML of the page. The cell extracts title, body text and
    /// headings from it.
    pub html: String,
}

/// One generated search-index file, to be written as a static site asset.
#[derive(Debug, Clone, Facet)]
pub struct SearchFile {
    /// Site-absolute path the file is served at, e.g. `/search/meta`.
    pub path: String,
    /// Postcard-encoded file contents (see `dodeca-search-format`).
    pub contents: Vec<u8>,
}

/// Result of building a search index.
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum SearchIndexResult {
    /// Index built; `files` are the static assets to emit under `/search/`.
    Success { files: Vec<SearchFile> },
    /// Indexing failed.
    Error { message: String },
}

/// Search indexing service implemented by the cell.
///
/// The host calls this once per build with every rendered HTML page.
#[allow(async_fn_in_trait)]
#[vox::service]
pub trait SearchIndexer {
    /// Build a full-text search index from the given pages.
    async fn build_index(&self, pages: Vec<SearchPage>) -> SearchIndexResult;
}
