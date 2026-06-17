//! Full-text search integration tests.
//!
//! These exercise the whole pipeline through the real production path: a
//! served site triggers `search_index_files`, which renders every page and
//! drives the in-process search indexer; the resulting postcard files are
//! served under `/search/` and decoded + queried here with the very same
//! `dodeca-search-format` crate the browser WASM uses.

use super::*;
use dodeca_search_format as fmt;
use std::collections::HashMap;

/// A content page with distinctive vocabulary unlikely to collide with the
/// `sample-site` fixture, so query assertions are deterministic.
const FIXTURE_PAGE: &str = r#"+++
title = "Search Fixture Page"
+++

# Search Fixture Page

The quintessential platypus paragraph mentions semaphores repeatedly. A
semaphore coordinates concurrent access between cooperating tasks.

## Telemetry section

Telemetry and observability tooling make debugging tractable.
"#;

/// Run a query exactly as the browser WASM core does: fetch the manifest,
/// fetch the shards the query touches, rank, then fetch + render fragments.
async fn run_query(site: &TestSite, query: &str) -> Vec<fmt::SearchResult> {
    let meta: fmt::SearchMeta =
        fmt::decode(&site.get_bytes("/search/meta").await).expect("decode meta");

    let mut shards: HashMap<String, fmt::Shard> = HashMap::new();
    for prefix in fmt::shards_for_query(query) {
        if let Some(shard_ref) = meta.shards.iter().find(|r| r.prefix == prefix) {
            let shard = fmt::decode(&site.get_bytes(&shard_ref.file).await).expect("decode shard");
            shards.insert(prefix, shard);
        }
    }

    let hits = fmt::rank(&meta, query, |prefix| shards.get(prefix), 10);
    let mut results = Vec::with_capacity(hits.len());
    for hit in &hits {
        let doc = &meta.docs[hit.doc as usize];
        let fragment: fmt::Fragment =
            fmt::decode(&site.get_bytes(&doc.fragment).await).expect("decode fragment");
        results.push(fmt::render(hit, &fragment));
    }
    results
}

/// The index is built, served, and answers single-term, AND, and no-match
/// queries correctly.
// s[verify serve.index-paths]
// s[verify serve.both-modes]
// s[verify format.manifest]
// s[verify index.title]
// s[verify index.anchors]
// s[verify query.and]
// s[verify query.bm25]
// s[verify query.shard-selection]
// s[verify render.mark]
// s[verify render.deeplink]
// s[verify render.text-fragment]
// s[verify version.stamp]
pub async fn search_index_answers_queries() {
    let site = TestSite::with_files(
        "sample-site",
        &[("content/search-fixture.md", FIXTURE_PAGE)],
    );

    // The manifest is well-formed and indexes the fixture page with its title.
    let meta: fmt::SearchMeta =
        fmt::decode(&site.get_bytes("/search/meta").await).expect("decode meta");
    assert_eq!(meta.version, fmt::FORMAT_VERSION, "format version");
    let fixture_doc = meta
        .docs
        .iter()
        .find(|d| d.url == "/search-fixture/")
        .unwrap_or_else(|| {
            panic!(
                "fixture page should be indexed; indexed urls: {:?}",
                meta.docs.iter().map(|d| &d.url).collect::<Vec<_>>()
            )
        });
    assert_eq!(
        fixture_doc.title, "Search Fixture Page",
        "title is taken from the page heading"
    );

    // A distinctive single term resolves to exactly the fixture page, with the
    // matched word highlighted in the excerpt.
    let hits = run_query(&site, "platypus").await;
    assert_eq!(hits.len(), 1, "exactly one page mentions 'platypus'");
    assert_eq!(
        hits[0].url.split('#').next().unwrap(),
        "/search-fixture/",
        "the platypus hit is the fixture page"
    );
    assert!(
        hits[0].excerpt.contains("<mark>"),
        "excerpt should highlight the match: {}",
        hits[0].excerpt
    );

    // A term that occurs only under the "Telemetry section" heading deep-links
    // into that section.
    let deep = run_query(&site, "observability").await;
    assert_eq!(deep.len(), 1, "one page mentions 'observability'");
    assert!(
        deep[0].url.contains("/search-fixture/#"),
        "result should deep-link to the heading anchor: {}",
        deep[0].url
    );
    assert!(
        deep[0].url.contains(":~:text="),
        "result should carry a text-fragment directive: {}",
        deep[0].url
    );

    // AND semantics: every query word must occur in the same document.
    let both = run_query(&site, "platypus telemetry").await;
    assert_eq!(both.len(), 1, "the fixture page contains both words");

    let absent = run_query(&site, "platypus nonexistentxyzzy").await;
    assert!(
        absent.is_empty(),
        "no page contains 'nonexistentxyzzy', so the AND yields nothing"
    );
}

/// Every page injects the widget, and the runtime assets (WASM core, loader,
/// UI, stylesheet) are served from their content-versioned directory.
// s[verify serve.runtime]
// s[verify serve.inject]
pub async fn search_runtime_assets_served() {
    let site = TestSite::new("sample-site");

    // Every page injects the widget at a content-versioned asset URL.
    let html = site.get("/").await;
    html.assert_ok();
    let js_url = html
        .extract(r#"src="(/search/asset/[^"]+/search\.js)""#)
        .expect("page should inject a versioned search.js");
    let dir = js_url
        .strip_suffix("/search.js")
        .expect("the injected url ends in /search.js");

    // All four runtime assets are served, non-empty, under that directory.
    for name in [
        "search.js",
        "search.css",
        "dodeca_search_wasm.js",
        "dodeca_search_wasm_bg.wasm",
    ] {
        let bytes = site.get_bytes(&format!("{dir}/{name}")).await;
        assert!(
            !bytes.is_empty(),
            "{dir}/{name} should be served and non-empty"
        );
    }
}
