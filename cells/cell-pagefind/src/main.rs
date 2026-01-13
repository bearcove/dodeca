//! Dodeca Pagefind cell (cell-pagefind)
//!
//! This cell handles search indexing using Pagefind.
//!
//! The pagefind crate uses html5ever's DomParser which is !Send, so the async
//! futures cannot be sent across threads. Since the cell runtime already runs
//! in a tokio context, we spawn a separate OS thread with its own runtime to
//! avoid "Cannot start a runtime from within a runtime" errors.
//!
//! IMPORTANT: This is intentionally an OS thread + dedicated current_thread runtime
//! with a tokio oneshot back to the async caller. Do NOT replace it with
//! tokio::spawn/spawn_blocking or a std::sync::mpsc recv in async code.

use pagefind::api::PagefindIndex;
use roam_shm::driver::establish_guest;
use roam_shm::guest::ShmGuest;
use roam_shm::spawn::SpawnArgs;
use roam_shm::transport::ShmGuestTransport;
use tokio::sync::oneshot;

use cell_pagefind_proto::{
    SearchFile, SearchIndexInput, SearchIndexOutput, SearchIndexResult, SearchIndexer,
    SearchIndexerDispatcher,
};

/// Search indexer implementation
#[derive(Clone)]
pub struct SearchIndexerImpl;

impl SearchIndexer for SearchIndexerImpl {
    async fn build_search_index(&self, input: SearchIndexInput) -> SearchIndexResult {
        // Spawn a separate OS thread with its own runtime because:
        // 1. We're already inside the cell's tokio runtime
        // 2. Pagefind futures are !Send (html5ever's DomParser)
        // 3. We need block_on but can't nest runtimes
        //
        // IMPORTANT: Do NOT "simplify" this by using tokio::spawn/spawn_blocking.
        // Pagefind's futures are !Send, so we must run them on a dedicated OS thread with its
        // own current_thread runtime and communicate across threads. We use a tokio oneshot
        // so the async caller can await without blocking the outer runtime.
        // This has been accidentally reverted multiple times and reintroduced failures.
        let (tx, rx) = oneshot::channel();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build runtime for pagefind");

            let result = rt.block_on(build_search_index_inner(input));
            let _ = tx.send(result);
        });

        match rx.await {
            Ok(Ok(output)) => SearchIndexResult::Success { output },
            Ok(Err(e)) => SearchIndexResult::Error { message: e },
            Err(_) => SearchIndexResult::Error {
                message: "pagefind thread panicked".to_string(),
            },
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = SpawnArgs::from_env()?;
    let guest = ShmGuest::attach_with_ticket(&args)?;
    let transport = ShmGuestTransport::new(guest);
    let dispatcher = SearchIndexerDispatcher::new(SearchIndexerImpl);
    let (_handle, driver) = establish_guest(transport, dispatcher);
    driver.run().await?;
    Ok(())
}
