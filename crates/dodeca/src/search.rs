//! Search indexing via pagefind cell
//!
//! Builds a full-text search index from HTML content.
//! Works entirely in memory - no files need to be written to disk.

use crate::cells::build_search_index_cell;
use crate::db::{OutputFile, SiteOutput};
use cell_pagefind_proto::SearchPage;
use eyre::eyre;
use std::collections::HashMap;

/// Search index files (path -> content)
pub type SearchFiles = HashMap<String, Vec<u8>>;

/// Collect HTML pages from site output for search indexing.
pub fn collect_search_pages(output: &SiteOutput) -> Vec<SearchPage> {
    output
        .files
        .iter()
        .filter_map(|file| {
            if let OutputFile::Html { route, content } = file {
                let url = if route.as_str() == "/" {
                    "/".to_string()
                } else {
                    format!("{}/", route.as_str().trim_end_matches('/'))
                };
                Some(SearchPage {
                    url,
                    html: content.clone(),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Build a search index from site output (one-shot, for build mode)
///
/// This is an async function that should be called from the main runtime
/// where the cell RPC sessions are running.
pub async fn build_search_index_async(output: &SiteOutput) -> eyre::Result<SearchFiles> {
    use cell_pagefind_proto::SearchIndexResult;

    let pages = collect_search_pages(output);

    let search_result = build_search_index_cell(cell_pagefind_proto::SearchIndexInput { pages })
        .await
        .map_err(|e| eyre!("pagefind cell error: {}", e))?;

    let files = match search_result {
        SearchIndexResult::Success { output } => output.files,
        SearchIndexResult::Error { message } => {
            return Err(eyre!("pagefind: {}", message));
        }
    };

    // Convert to HashMap
    let mut result = HashMap::new();
    for file in files {
        result.insert(file.path, file.contents);
    }

    Ok(result)
}
