//! Full-text search integration.
//!
//! Ties the `cell-search` indexer into the build graph and serves the
//! resulting assets under `/search/`. Two kinds of file land there:
//!
//! - **Runtime assets** — the WASM query core, its wasm-bindgen loader, the UI
//!   script and the stylesheet. These change only when `ddc` itself does, so
//!   they live under a content-versioned directory (`/search/asset/<v>/…`) and
//!   are cached immutably. The version segment also means every cross-reference
//!   between the JS files is a plain *relative* URL that inherits `<v>` for
//!   free — no path rewriting.
//! - **Index files** ([`search_index_files`]) — the postcard manifest, shards
//!   and fragments built by `cell-search` from the rendered HTML. These are
//!   content-derived; the function is a tracked query that re-runs only when
//!   page content changes. They keep stable paths and are served revalidated.
//!
//! Both `ddc build` (via `build_site`) and `ddc serve` (via `find_content`)
//! pull from here, so the index is identical in dev and production.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use cell_search_proto::SearchPage;
use picante::PicanteResult;

use crate::db::{Db, OutputFile};
use crate::queries::{build_tree, serve_html};
use crate::types::{Route, StaticPath};

// ============================================================================
// Runtime assets — embedded at compile time, served content-versioned.
// ============================================================================

/// The hand-written UI layer (input box, dropdown, keyboard handling).
const SEARCH_JS: &str = include_str!("../../dodeca-search-wasm/ui/search.js");
/// Widget styling.
const SEARCH_CSS: &str = include_str!("../../dodeca-search-wasm/ui/search.css");
/// wasm-bindgen JS loader for the query core.
const WASM_JS: &str = include_str!("../../dodeca-search-wasm/pkg/dodeca_search_wasm.js");
/// The compiled WASM query core itself.
const WASM_BG: &[u8] = include_bytes!("../../dodeca-search-wasm/pkg/dodeca_search_wasm_bg.wasm");

/// Runtime asset files: `(filename, bytes)`. They are served together under one
/// versioned directory; `search.js` and the wasm-bindgen loader reference each
/// other (and the `.wasm`) with relative URLs, so the directory is all that
/// needs versioning.
const RUNTIME_FILES: &[(&str, &[u8])] = &[
    ("search.js", SEARCH_JS.as_bytes()),
    ("search.css", SEARCH_CSS.as_bytes()),
    ("dodeca_search_wasm.js", WASM_JS.as_bytes()),
    ("dodeca_search_wasm_bg.wasm", WASM_BG),
];

/// Content hash of the runtime assets — changes only when `ddc` itself does.
/// Forms the version segment of every runtime asset URL, so a new `ddc` serves
/// the assets at fresh URLs and browsers never use a stale cached copy.
fn asset_version() -> &'static str {
    static VERSION: LazyLock<String> = LazyLock::new(|| {
        let mut hasher = DefaultHasher::new();
        for (name, bytes) in RUNTIME_FILES {
            name.hash(&mut hasher);
            bytes.hash(&mut hasher);
        }
        format!("{:012x}", hasher.finish())
    });
    VERSION.as_str()
}

/// The output-relative directory the runtime assets live in (no leading slash),
/// e.g. `search/asset/a1b2c3d4e5f6`.
fn runtime_dir() -> String {
    format!("search/asset/{}", asset_version())
}

/// The runtime assets as build outputs, for `build_site` to emit to disk.
// s[impl serve.runtime]
pub fn runtime_output_files() -> Vec<OutputFile> {
    let dir = runtime_dir();
    RUNTIME_FILES
        .iter()
        .map(|(name, bytes)| OutputFile::Static {
            path: StaticPath::new(format!("{dir}/{name}")),
            content: bytes.to_vec(),
        })
        .collect()
}

/// If `rel` (a path without leading slash) addresses a runtime asset, return
/// its bytes. Any version segment matches — the segment is purely a cache key,
/// so serving the current bytes for it is always correct.
pub fn runtime_asset(rel: &str) -> Option<&'static [u8]> {
    let (_version, name) = rel.strip_prefix("search/asset/")?.split_once('/')?;
    RUNTIME_FILES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, bytes)| *bytes)
}

/// The `<head>` markup that activates the search widget on every page: the
/// stylesheet and the ES module driving the WASM query core, at their
/// content-versioned URLs. `render.rs` injects this into every page.
// s[impl serve.inject]
pub fn search_head_injection() -> String {
    let dir = runtime_dir();
    format!(
        "<link rel=\"stylesheet\" href=\"/{dir}/search.css\">\
         <script type=\"module\" src=\"/{dir}/search.js\"></script>"
    )
}

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
