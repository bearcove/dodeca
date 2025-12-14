//! Dodeca Pagefind plugin (dodeca-mod-pagefind)
//!
//! This plugin handles search indexing using Pagefind.
//!
//! The pagefind crate uses html5ever's DomParser which is !Send, so the async
//! futures cannot be sent across threads. Since the plugin runtime already runs
//! in a tokio context, we spawn a separate OS thread with its own runtime to
//! avoid "Cannot start a runtime from within a runtime" errors.

use std::sync::mpsc;

use pagefind::api::PagefindIndex;

use mod_pagefind_proto::{SearchIndexer, SearchIndexResult, SearchIndexInput, SearchIndexOutput, SearchFile, SearchIndexerServer};

/// Search indexer implementation
pub struct SearchIndexerImpl;

impl SearchIndexer for SearchIndexerImpl {
    async fn build_search_index(&self, input: SearchIndexInput) -> SearchIndexResult {
        // Spawn a separate OS thread with its own runtime because:
        // 1. We're already inside the plugin's tokio runtime
        // 2. Pagefind futures are !Send (html5ever's DomParser)
        // 3. We need block_on but can't nest runtimes
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build runtime for pagefind");

            let result = rt.block_on(build_search_index_inner(input));
            let _ = tx.send(result);
        });

        match rx.recv() {
            Ok(Ok(output)) => SearchIndexResult::Success { output },
            Ok(Err(e)) => SearchIndexResult::Error { message: e },
            Err(_) => SearchIndexResult::Error { message: "pagefind thread panicked".to_string() },
        }
    }
}

async fn build_search_index_inner(input: SearchIndexInput) -> Result<SearchIndexOutput, String> {
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
