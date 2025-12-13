//! Dodeca Pagefind plugin (dodeca-mod-pagefind)
//!
//! This plugin handles search indexing using Pagefind.

use std::sync::OnceLock;

use pagefind::api::PagefindIndex;

use mod_pagefind_proto::{SearchIndexer, SearchIndexResult, SearchIndexInput, SearchIndexOutput, SearchFile, SearchIndexerServer};

/// Search indexer implementation
pub struct SearchIndexerImpl;

impl SearchIndexer for SearchIndexerImpl {
    async fn build_search_index(&self, input: SearchIndexInput) -> SearchIndexResult {
        // Use blocking approach like the original plugin
        build_search_index_blocking(input)
    }
}

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

fn build_search_index_blocking(input: SearchIndexInput) -> SearchIndexResult {
    match runtime().block_on(async { build_search_index_async(input).await }) {
        Ok(output) => SearchIndexResult::Success { output },
        Err(e) => SearchIndexResult::Error { message: e },
    }
}

async fn build_search_index_async(input: SearchIndexInput) -> Result<SearchIndexOutput, String> {
    // Create pagefind index
    let mut index = match PagefindIndex::new(None) {
        Ok(idx) => idx,
        Err(e) => return Err(format!("Failed to create pagefind index: {}", e)),
    };

    // Add all pages
    for page in input.pages {
        if let Err(e) = index
            .add_html_file(None, Some(page.url.clone()), page.html)
            .await
        {
            return Err(format!("Failed to add page {}: {}", page.url, e));
        }
    }

    // Get output files
    let files = match index.get_files().await {
        Ok(files) => files,
        Err(e) => return Err(format!("Failed to build search index: {}", e)),
    };

    // Convert to our output format
    let output_files: Vec<SearchFile> = files
        .into_iter()
        .map(|f| SearchFile {
            path: format!("/pagefind/{}", f.filename.display()),
            contents: f.contents,
        })
        .collect();

    Ok(SearchIndexOutput {
        files: output_files,
    })
}

dodeca_plugin_runtime::plugin_service!(
    SearchIndexerServer<SearchIndexerImpl>,
    SearchIndexerImpl
);

dodeca_plugin_runtime::run_plugin!(SearchIndexerImpl);
