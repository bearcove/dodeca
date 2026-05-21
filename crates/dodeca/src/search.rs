//! Full-text search integration.
//!
//! Ties the `cell-search` indexer into the build graph and serves the
//! resulting assets under `/search/`. Two kinds of file land there:
//!
//! - **Runtime assets** ([`RUNTIME_ASSETS`]) — the WASM query core, its
//!   wasm-bindgen loader, the UI script and the stylesheet. These are version-
//!   static: embedded into `ddc` at compile time and emitted verbatim.
//! - **Index files** ([`search_index_files`]) — the postcard manifest, shards
//!   and fragments built by `cell-search` from the rendered HTML. These are
//!   content-derived, so the function is a tracked query: it re-runs only when
//!   page content actually changes.
//!
//! Both `ddc build` (via `build_site`) and `ddc serve` (via `find_content`)
//! pull from here, so the index is identical in dev and production.

use cell_search_proto::SearchPage;
use picante::PicanteResult;

use crate::db::{Db, OutputFile};
use crate::queries::{build_tree, serve_html};
use crate::types::{Route, StaticPath};

// ============================================================================
// Runtime assets — embedded at compile time, emitted verbatim.
// ============================================================================

/// The hand-written UI layer (input box, dropdown, keyboard handling).
const SEARCH_JS: &str = include_str!("../../dodeca-search-wasm/ui/search.js");
/// Widget styling.
const SEARCH_CSS: &str = include_str!("../../dodeca-search-wasm/ui/search.css");
/// wasm-bindgen JS loader for the query core.
const WASM_JS: &str = include_str!("../../dodeca-search-wasm/pkg/dodeca_search_wasm.js");
/// The compiled WASM query core itself.
const WASM_BG: &[u8] = include_bytes!("../../dodeca-search-wasm/pkg/dodeca_search_wasm_bg.wasm");

/// Version-static search runtime files: `(output path without leading slash,
/// bytes)`. Served at fixed `/search/` URLs — `search.js` hard-codes them.
// s[impl serve.runtime]
pub const RUNTIME_ASSETS: &[(&str, &[u8])] = &[
    ("search/search.js", SEARCH_JS.as_bytes()),
    ("search/search.css", SEARCH_CSS.as_bytes()),
    ("search/dodeca_search_wasm.js", WASM_JS.as_bytes()),
    ("search/dodeca_search_wasm_bg.wasm", WASM_BG),
];

/// The runtime assets as build outputs, for `build_site` to emit to disk.
pub fn runtime_output_files() -> Vec<OutputFile> {
    RUNTIME_ASSETS
        .iter()
        .map(|(path, bytes)| OutputFile::Static {
            path: StaticPath::new((*path).to_string()),
            content: bytes.to_vec(),
        })
        .collect()
}

/// The `<head>` markup that activates the search widget on every page: the
/// stylesheet and the ES module driving the WASM query core. Both resolve to
/// [`RUNTIME_ASSETS`] entries. `render.rs` injects this into every page.
// s[impl serve.inject]
pub const SEARCH_ASSETS: &str = concat!(
    r#"<link rel="stylesheet" href="/search/search.css">"#,
    r#"<script type="module" src="/search/search.js"></script>"#,
);

// ============================================================================
// Index files — content-derived, built by cell-search.
// ============================================================================

/// The canonical, trailing-slashed URL of a route — the form a browser
/// actually visits, and the form stored in the index so result links resolve.
fn route_to_url(route: &Route) -> String {
    if route.as_str() == "/" {
        "/".to_string()
    } else {
        format!("{}/", route.as_str().trim_end_matches('/'))
    }
}

/// Build the `/search/` index files (manifest, shards, fragments) from every
/// rendered page, via the `cell-search` indexer.
///
/// Tracked: `serve_html` is memoized per route, so this shares phase-1 renders
/// with `build_site` and only re-runs when page content changes. A missing or
/// failing `cell-search` cdylib degrades gracefully to an empty index — the
/// build still succeeds, search just has nothing to answer with.
//
// `build_site` calls this to emit the index to disk; `serve`'s `find_content`
// calls it to serve the index live — the single shared path behind both modes.
// s[impl serve.both-modes]
#[picante::tracked]
pub async fn search_index_files<DB: Db>(db: &DB) -> PicanteResult<Vec<OutputFile>> {
    let tree = match build_tree(db).await? {
        Ok(tree) => tree,
        // Tree errors are surfaced by `build_site`; here we just skip indexing.
        Err(_) => return Ok(Vec::new()),
    };

    let mut pages: Vec<SearchPage> = Vec::new();
    for route in tree.sections.keys().chain(tree.pages.keys()) {
        if let Ok(Some(served)) = serve_html(db, route.clone()).await? {
            pages.push(SearchPage {
                url: route_to_url(route),
                html: served.html,
            });
        }
    }

    let files = match crate::cells::build_search_index_cell(pages).await {
        Ok(files) => files,
        Err(e) => {
            tracing::warn!(error = %e, "search index unavailable; serving empty index");
            return Ok(Vec::new());
        }
    };

    Ok(files
        .into_iter()
        .map(|f| OutputFile::Static {
            path: StaticPath::new(f.path.trim_start_matches('/').to_string()),
            content: f.contents,
        })
        .collect())
}
