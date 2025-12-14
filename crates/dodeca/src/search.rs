//! Search indexing via pagefind plugin
//!
//! Builds a full-text search index from HTML content.
//! Works entirely in memory - no files need to be written to disk.

use crate::db::{OutputFile, SiteOutput};
use crate::cells::build_search_index_plugin;
use eyre::eyre;
use cell_pagefind_proto::SearchPage;
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
/// Creates a current-thread runtime to call the async plugin RPC.
pub fn build_search_index(output: &SiteOutput) -> eyre::Result<SearchFiles> {
    let pages = collect_search_pages(output);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| eyre!("failed to build runtime for search indexing: {e}"))?;

    let files = rt
        .block_on(build_search_index_plugin(pages))
        .map_err(|e| eyre!("pagefind: {}", e))?;

    // Convert to HashMap
    let mut result = HashMap::new();
    for file in files {
        result.insert(file.path, file.contents);
    }

    Ok(result)
}
