//! Browser-side query runtime for dodeca's from-scratch full-text search.
//!
//! Compiled to WebAssembly and loaded by `/search/search.js`. This is the
//! *reader* half of [`dodeca_search_format`]: it fetches the postcard index
//! files emitted by `cell-search`, decodes them, runs the BM25 query engine,
//! and hands back rendered results as JSON.
//!
//! Network shape mirrors pagefind: `meta` is fetched once at [`init`]; shards
//! and fragments are fetched lazily on first need and cached for the lifetime
//! of the page. A cold query touches at most one shard per query word plus one
//! fragment per shown result; a warm query touches the network not at all.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::collections::HashMap;

use dodeca_search_format::{
    FORMAT_VERSION, Fragment, SearchMeta, Shard, decode, rank, render, shards_for_query,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

/// Maximum results returned from one [`search`] call.
const DEFAULT_LIMIT: usize = 8;

/// Loaded index state. WASM is single-threaded, so a thread-local `RefCell` is
/// the entire synchronization story. No borrow is ever held across an `.await`.
struct SearchState {
    meta: SearchMeta,
    /// Inverted-index shards, keyed by term prefix; filled lazily.
    shards: HashMap<String, Shard>,
    /// Per-document display fragments, keyed by fragment path; filled lazily.
    fragments: HashMap<String, Fragment>,
}

thread_local! {
    static STATE: RefCell<Option<SearchState>> = const { RefCell::new(None) };
}

/// Load the search manifest. Must be awaited once before [`search`] is called;
/// `/search/search.js` does this on page load. Named `load_index` rather than
/// `init` so it never shadows the wasm-bindgen module loader (also `init`).
///
/// `meta_url` is the site-absolute path of the manifest, normally
/// `/search/meta`.
#[wasm_bindgen]
pub async fn load_index(meta_url: String) -> Result<(), JsValue> {
    let bytes = fetch_bytes(&meta_url).await.map_err(js_err)?;
    let meta: SearchMeta = decode(&bytes).map_err(js_err)?;
    // s[impl version.reject]
    if meta.version != FORMAT_VERSION {
        return Err(js_err(format!(
            "search index format {} unsupported (reader expects {FORMAT_VERSION})",
            meta.version
        )));
    }
    STATE.with(|s| {
        *s.borrow_mut() = Some(SearchState {
            meta,
            shards: HashMap::new(),
            fragments: HashMap::new(),
        });
    });
    Ok(())
}

/// Run a query and return a JSON array of results (`{url, title, excerpt,
/// score}`), best first. `/search/search.js` `JSON.parse`s the return value.
///
/// Rejects only on a genuine fault (network, malformed index); an empty query
/// or a query with no matches resolves to `"[]"`.
#[wasm_bindgen]
pub async fn search(query: String) -> Result<JsValue, JsValue> {
    let json = do_search(&query, DEFAULT_LIMIT).await.map_err(js_err)?;
    Ok(JsValue::from_str(&json))
}

/// The query pipeline: resolve+fetch the needed shards, rank, resolve+fetch
/// the result fragments, render. Caches make repeat queries network-free.
async fn do_search(query: &str, limit: usize) -> Result<String, String> {
    // Which shards does this query touch that aren't cached yet? A query-word
    // prefix with no matching `ShardRef` simply isn't fetched — `rank` then
    // sees `None` for that slot and the AND drops every document.
    let shard_fetches: Vec<(String, String)> =
        STATE.with(|s| -> Result<Vec<(String, String)>, String> {
            let s = s.borrow();
            let st = s.as_ref().ok_or("search not initialized")?;
            Ok(shards_for_query(query)
                .into_iter()
                .filter(|p| !st.shards.contains_key(p))
                .filter_map(|p| {
                    st.meta
                        .shards
                        .iter()
                        .find(|r| r.prefix == p)
                        .map(|r| (p, r.file.clone()))
                })
                .collect())
        })?;
    for (prefix, url) in shard_fetches {
        let bytes = fetch_bytes(&url).await?;
        let shard: Shard = decode(&bytes)?;
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.shards.insert(prefix, shard);
            }
        });
    }

    // Rank against the now-loaded shards.
    let hits = STATE.with(|s| {
        let s = s.borrow();
        let st = s.as_ref().expect("state present after shard load");
        rank(&st.meta, query, |p| st.shards.get(p), limit)
    });
    if hits.is_empty() {
        return Ok("[]".to_string());
    }

    // Fetch the fragments for the surviving hits (distinct, uncached only).
    let frag_fetches: Vec<String> = STATE.with(|s| {
        let s = s.borrow();
        let st = s.as_ref().expect("state present");
        let mut urls: Vec<String> = hits
            .iter()
            .filter_map(|h| st.meta.docs.get(h.doc as usize))
            .map(|dm| dm.fragment.clone())
            .filter(|f| !st.fragments.contains_key(f))
            .collect();
        urls.sort();
        urls.dedup();
        urls
    });
    for url in frag_fetches {
        let bytes = fetch_bytes(&url).await?;
        let fragment: Fragment = decode(&bytes)?;
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.fragments.insert(url, fragment);
            }
        });
    }

    // Render each hit into a display result.
    let results = STATE.with(|s| {
        let s = s.borrow();
        let st = s.as_ref().expect("state present");
        hits.iter()
            .filter_map(|h| {
                let dm = st.meta.docs.get(h.doc as usize)?;
                let fragment = st.fragments.get(&dm.fragment)?;
                Some(render(h, fragment))
            })
            .collect::<Vec<_>>()
    });
    facet_json::to_string(&results).map_err(|e| format!("serialize results: {e:?}"))
}

// ============================================================================
// Fetch helpers
// ============================================================================

/// GET `url` and return the raw response body. Errors carry the URL so a
/// failure is diagnosable from the browser console alone.
async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no global `window`")?;
    let resp_value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("fetch {url}: {}", err_text(&e)))?;
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| format!("fetch {url}: response was not a Response"))?;
    if !resp.ok() {
        return Err(format!("fetch {url}: HTTP {}", resp.status()));
    }
    let buffer = resp
        .array_buffer()
        .map_err(|e| format!("fetch {url}: array_buffer(): {}", err_text(&e)))?;
    let buffer = JsFuture::from(buffer)
        .await
        .map_err(|e| format!("fetch {url}: reading body: {}", err_text(&e)))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

fn err_text(value: &JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}

fn js_err(message: impl AsRef<str>) -> JsValue {
    JsValue::from_str(message.as_ref())
}
