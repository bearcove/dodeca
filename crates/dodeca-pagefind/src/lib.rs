//! Search indexing plugin for dodeca using pagefind
//!
//! This plugin wraps pagefind's async API with blocking calls,
//! allowing it to be used from dodeca's synchronous plugin system.

use facet::Facet;
use pagefind::api::PagefindIndex;
use plugcard::{plugcard, PlugResult};
use std::sync::OnceLock;

plugcard::export_plugin!();

/// Global tokio runtime for blocking on async pagefind calls
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
    })
}

/// A page to be indexed
#[derive(Facet, Debug)]
pub struct SearchPage {
    /// URL of the page (e.g., "/guide/")
    pub url: String,
    /// HTML content of the page
    pub html: String,
}

/// Output file from pagefind
#[derive(Facet, Debug)]
pub struct SearchFile {
    /// Path where the file should be served (e.g., "/pagefind/pagefind.js")
    pub path: String,
    /// File contents
    pub contents: Vec<u8>,
}

/// Input for building search index
#[derive(Facet, Debug)]
pub struct SearchIndexInput {
    /// Pages to index
    pub pages: Vec<SearchPage>,
}

/// Output from building search index
#[derive(Facet, Debug)]
pub struct SearchIndexOutput {
    /// Generated search files
    pub files: Vec<SearchFile>,
}

/// Build a search index from HTML pages
///
/// Takes a list of pages (url + html) and returns the pagefind output files.
#[plugcard]
pub fn build_search_index(input: SearchIndexInput) -> PlugResult<SearchIndexOutput> {
    runtime().block_on(async {
        build_search_index_async(input).await
    })
}

async fn build_search_index_async(input: SearchIndexInput) -> PlugResult<SearchIndexOutput> {
    // Create pagefind index
    let mut index = match PagefindIndex::new(None) {
        Ok(idx) => idx,
        Err(e) => return PlugResult::Err(format!("Failed to create pagefind index: {}", e)),
    };

    // Add all pages
    for page in input.pages {
        if let Err(e) = index
            .add_html_file(None, Some(page.url.clone()), page.html)
            .await
        {
            return PlugResult::Err(format!("Failed to add page {}: {}", page.url, e));
        }
    }

    // Get output files
    let files = match index.get_files().await {
        Ok(files) => files,
        Err(e) => return PlugResult::Err(format!("Failed to build search index: {}", e)),
    };

    // Convert to our output format
    let output_files: Vec<SearchFile> = files
        .into_iter()
        .map(|f| SearchFile {
            path: format!("/pagefind/{}", f.filename.display()),
            contents: f.contents,
        })
        .collect();

    PlugResult::Ok(SearchIndexOutput { files: output_files })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_empty_index() {
        let input = SearchIndexInput { pages: vec![] };
        let result = build_search_index(input);
        // Even an empty index produces some files (pagefind.js, etc.)
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got {:?}", result);
        };
        assert!(!output.files.is_empty());
    }

    #[test]
    fn test_build_with_page() {
        let input = SearchIndexInput {
            pages: vec![SearchPage {
                url: "/test/".to_string(),
                html: "<html><body><h1>Test Page</h1><p>Hello world</p></body></html>".to_string(),
            }],
        };
        let result = build_search_index(input);
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got {:?}", result);
        };
        // Should have pagefind.js and index files
        assert!(output.files.iter().any(|f| f.path.contains("pagefind.js")));
    }
}
