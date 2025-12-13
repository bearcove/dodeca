//! Search indexing via pagefind plugin
//!
//! Builds a full-text search index from HTML content.
//! Works entirely in memory - no files need to be written to disk.

use crate::db::{OutputFile, SiteOutput};
use crate::plugins::build_search_index_plugin;
use eyre::eyre;
use mod_pagefind_proto::SearchPage;
use std::collections::HashMap;
use tokio::runtime::Handle;

/// Search index files (path -> content)
pub type SearchFiles = HashMap<String, Vec<u8>>;

/// Build a search index from site output (one-shot, for build mode)
///
/// Note: This is now synchronous since it uses the plugin which blocks internally.
pub fn build_search_index(output: &SiteOutput) -> eyre::Result<SearchFiles> {
    // Collect HTML pages
    let pages: Vec<SearchPage> = output
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
        .collect();

    // Build index via plugin (block in current runtime)
    let handle = Handle::current();
    let files = handle
        .block_on(build_search_index_plugin(pages))
        .map_err(|e| eyre!("pagefind: {}", e))?;

    // Convert to HashMap
    let mut result = HashMap::new();
    for file in files {
        result.insert(file.path, file.contents);
    }

    Ok(result)
}
