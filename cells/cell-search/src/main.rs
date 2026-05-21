//! Dodeca full-text search indexing cell (cell-search).
//!
//! Receives the rendered HTML of every page, extracts searchable text and the
//! heading structure with hotmeal, and builds a sharded inverted index in the
//! `dodeca-search-format` postcard layout. The host writes the returned files
//! under `/search/` as static site assets.

use std::collections::BTreeMap;

use hotmeal::{Document, NodeId, NodeKind, StrTendril};

use cell_search_proto::{
    SearchFile, SearchIndexResult, SearchIndexer, SearchIndexerDispatcher, SearchPage,
};
use dodeca_search_format as fmt;

/// Tags whose subtrees carry no page content worth indexing (site chrome,
/// scripts, styling). Their descendants are skipped entirely.
// s[impl index.skip-chrome]
const SKIP_TAGS: &[&str] = &[
    "script", "style", "nav", "header", "footer", "aside", "template", "noscript",
];

/// Search indexer implementation. Stateless — every `build_index` call is
/// self-contained.
#[derive(Clone)]
struct SearchIndexerImpl;

impl SearchIndexer for SearchIndexerImpl {
    async fn build_index(&self, pages: Vec<SearchPage>) -> SearchIndexResult {
        let page_count = pages.len();
        match build(pages) {
            Ok(files) => {
                tracing::info!(
                    pages = page_count,
                    files = files.len(),
                    "search index built"
                );
                SearchIndexResult::Success { files }
            }
            Err(message) => {
                tracing::error!(error = %message, "search indexing failed");
                SearchIndexResult::Error { message }
            }
        }
    }
}

// ============================================================================
// HTML extraction
// ============================================================================

/// Text + structure pulled out of one page's HTML.
struct ExtractedDoc {
    title: String,
    /// Display (original-cased) tokens in document order.
    words: Vec<String>,
    /// Stemmed tokens, parallel to `words`.
    stems: Vec<String>,
    anchors: Vec<fmt::Anchor>,
}

/// Accumulator for the in-order DOM walk.
#[derive(Default)]
struct Walk {
    words: Vec<String>,
    stems: Vec<String>,
    anchors: Vec<fmt::Anchor>,
}

fn is_heading(tag: &str) -> bool {
    matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

/// Recursively collect text and headings under `node`, in document order.
fn walk(doc: &Document, node: NodeId, out: &mut Walk) {
    match &doc.get(node).kind {
        NodeKind::Text(t) => {
            for tok in fmt::analyze(t.as_ref()) {
                out.words.push(tok.display);
                out.stems.push(tok.stem);
            }
        }
        NodeKind::Element(elem) => {
            let tag = elem.tag.as_ref();
            if SKIP_TAGS.contains(&tag) {
                return;
            }
            // A heading with an `id` becomes a deep-link anchor positioned at
            // the word that follows it.
            // s[impl index.anchors]
            if is_heading(tag) {
                let id = elem
                    .attrs
                    .iter()
                    .find(|(n, _)| n.local.as_ref() == "id")
                    .map(|(_, v)| v.as_ref().to_string());
                if let Some(id) = id
                    && !id.is_empty()
                {
                    out.anchors.push(fmt::Anchor {
                        id,
                        text: collapse_ws(&text_content(doc, node)),
                        position: out.words.len() as u32,
                    });
                }
            }
            for child in doc.children(node) {
                walk(doc, child, out);
            }
        }
        _ => {}
    }
}

/// Concatenate all text under `node`.
fn text_content(doc: &Document, node: NodeId) -> String {
    let mut out = String::new();
    collect_text(doc, node, &mut out);
    out
}

fn collect_text(doc: &Document, node: NodeId, out: &mut String) {
    match &doc.get(node).kind {
        NodeKind::Text(t) => out.push_str(t.as_ref()),
        NodeKind::Element(_) => {
            for child in doc.children(node) {
                collect_text(doc, child, out);
            }
        }
        _ => {}
    }
}

/// Collapse runs of whitespace to single spaces and trim.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// First descendant element of `node` with the given tag, depth-first.
fn find_descendant(doc: &Document, node: NodeId, tag: &str) -> Option<NodeId> {
    for child in doc.children(node) {
        if let NodeKind::Element(elem) = &doc.get(child).kind
            && elem.tag.as_ref() == tag
        {
            return Some(child);
        }
        if let Some(found) = find_descendant(doc, child, tag) {
            return Some(found);
        }
    }
    None
}

/// The element whose text the page is "about": `<title>`, else the first
/// `<h1>`. Returns `None` when neither yields non-empty text.
// s[impl index.title]
fn extract_title(doc: &Document) -> Option<String> {
    if let Some(head) = doc.head()
        && let Some(title) = find_descendant(doc, head, "title")
    {
        let s = collapse_ws(&text_content(doc, title));
        if !s.is_empty() {
            return Some(s);
        }
    }
    if let Some(body) = doc.body()
        && let Some(h1) = find_descendant(doc, body, "h1")
    {
        let s = collapse_ws(&text_content(doc, h1));
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

/// The subtree to index: the first `<main>` if present, else the whole body.
// s[impl index.content-root]
fn content_root(doc: &Document) -> Option<NodeId> {
    let body = doc.body()?;
    Some(find_descendant(doc, body, "main").unwrap_or(body))
}

fn extract(page: &SearchPage) -> ExtractedDoc {
    let tendril = StrTendril::from(page.html.as_str());
    let doc = hotmeal::parse(&tendril);
    let title = extract_title(&doc).unwrap_or_else(|| page.url.clone());
    let mut out = Walk::default();
    if let Some(root) = content_root(&doc) {
        walk(&doc, root, &mut out);
    }
    ExtractedDoc {
        title,
        words: out.words,
        stems: out.stems,
        anchors: out.anchors,
    }
}

// ============================================================================
// Index construction
// ============================================================================

/// Build the full set of `/search/` files from the given pages.
fn build(pages: Vec<SearchPage>) -> Result<Vec<SearchFile>, String> {
    let mut docs: Vec<fmt::DocMeta> = Vec::with_capacity(pages.len());
    let mut fragments: Vec<fmt::Fragment> = Vec::with_capacity(pages.len());
    // term -> doc -> ascending positions
    let mut inverted: BTreeMap<String, BTreeMap<fmt::DocId, Vec<u32>>> = BTreeMap::new();
    let mut total_len: u64 = 0;

    for (i, page) in pages.iter().enumerate() {
        let doc_id = i as fmt::DocId;
        let ex = extract(page);
        // s[impl index.doc-length]
        let len = ex.words.len() as u32;
        total_len += u64::from(len);

        for (pos, stem) in ex.stems.iter().enumerate() {
            if stem.is_empty() {
                continue;
            }
            inverted
                .entry(stem.clone())
                .or_default()
                .entry(doc_id)
                .or_default()
                .push(pos as u32);
        }

        docs.push(fmt::DocMeta {
            url: page.url.clone(),
            title: ex.title.clone(),
            len,
            fragment: format!("/search/fragment/{doc_id}"),
        });
        fragments.push(fmt::Fragment {
            url: page.url.clone(),
            title: ex.title,
            words: ex.words,
            anchors: ex.anchors,
        });
    }

    let avg_doc_len = if docs.is_empty() {
        0.0
    } else {
        total_len as f32 / docs.len() as f32
    };

    // Shard the inverted index by term prefix. `inverted` is a BTreeMap, so
    // terms arrive sorted; each shard's `terms` vec is therefore sorted, which
    // the reader's binary search / range scan relies on.
    let mut shards: BTreeMap<String, Vec<fmt::TermPostings>> = BTreeMap::new();
    for (term, docmap) in inverted {
        let prefix = fmt::shard_prefix(&term);
        let postings = docmap
            .into_iter()
            .map(|(doc, positions)| fmt::Posting { doc, positions })
            .collect();
        shards
            .entry(prefix)
            .or_default()
            .push(fmt::TermPostings { term, postings });
    }

    // The fixed `/search/` paths the served index lives at.
    // s[impl serve.index-paths]
    let mut files: Vec<SearchFile> = Vec::new();
    let mut shard_refs: Vec<fmt::ShardRef> = Vec::new();
    for (prefix, terms) in shards {
        // The empty catch-all prefix can't be a filename; use `_`.
        let name = if prefix.is_empty() {
            "_"
        } else {
            prefix.as_str()
        };
        let path = format!("/search/index/{name}");
        let shard = fmt::Shard { terms };
        files.push(SearchFile {
            path: path.clone(),
            contents: fmt::encode(&shard).map_err(|e| format!("encode shard {name}: {e}"))?,
        });
        shard_refs.push(fmt::ShardRef { prefix, file: path });
    }

    for (i, fragment) in fragments.iter().enumerate() {
        files.push(SearchFile {
            path: format!("/search/fragment/{i}"),
            contents: fmt::encode(fragment).map_err(|e| format!("encode fragment {i}: {e}"))?,
        });
    }

    // s[impl version.stamp]
    let meta = fmt::SearchMeta {
        version: fmt::FORMAT_VERSION,
        avg_doc_len,
        docs,
        shards: shard_refs,
    };
    files.push(SearchFile {
        path: "/search/meta".to_string(),
        contents: fmt::encode(&meta).map_err(|e| format!("encode meta: {e}"))?,
    });

    Ok(files)
}

dodeca_cell_runtime::declare_cell!("search", |_host| {
    SearchIndexerDispatcher::new(SearchIndexerImpl)
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_shards_index() {
        let pages = vec![
            SearchPage {
                url: "/cells/".into(),
                html: "<main><h1 id=\"intro\">Cells</h1><p>Cells communicate over RPC.</p>\
                       <nav>skip me</nav></main>"
                    .into(),
            },
            SearchPage {
                url: "/markdown/".into(),
                html: "<main><h1>Markdown</h1><p>Markdown rendering pipeline.</p></main>".into(),
            },
        ];
        let files = build(pages).unwrap();

        let meta_bytes = &files
            .iter()
            .find(|f| f.path == "/search/meta")
            .unwrap()
            .contents;
        let meta: fmt::SearchMeta = fmt::decode(meta_bytes).unwrap();
        assert_eq!(meta.version, fmt::FORMAT_VERSION);
        assert_eq!(meta.docs.len(), 2);
        assert_eq!(meta.docs[0].title, "Cells");

        // The "c" shard exists and holds the stemmed term "cell".
        let cref = meta
            .shards
            .iter()
            .find(|s| s.prefix == "c")
            .expect("c shard");
        let shard_bytes = &files.iter().find(|f| f.path == cref.file).unwrap().contents;
        let shard: fmt::Shard = fmt::decode(shard_bytes).unwrap();
        assert!(shard.terms.iter().any(|t| t.term == "cell"));

        // Fragments exist for both docs; the "skip me" nav text is excluded.
        let frag0_bytes = &files
            .iter()
            .find(|f| f.path == "/search/fragment/0")
            .unwrap()
            .contents;
        let frag0: fmt::Fragment = fmt::decode(frag0_bytes).unwrap();
        assert!(!frag0.anchors.is_empty());
        assert!(frag0.words.iter().all(|w| w.to_lowercase() != "skip"));
    }

    #[test]
    fn end_to_end_query_against_built_index() {
        let pages = vec![
            SearchPage {
                url: "/a/".into(),
                html: "<main><h1>Searching</h1><p>The search engine ranks documents.</p></main>"
                    .into(),
            },
            SearchPage {
                url: "/b/".into(),
                html: "<main><h1>Images</h1><p>Image processing is unrelated.</p></main>".into(),
            },
        ];
        let files = build(pages).unwrap();
        let meta: fmt::SearchMeta = fmt::decode(
            &files
                .iter()
                .find(|f| f.path == "/search/meta")
                .unwrap()
                .contents,
        )
        .unwrap();

        // Decode every shard so the query engine can borrow them.
        let shard_map: std::collections::HashMap<String, fmt::Shard> = meta
            .shards
            .iter()
            .map(|r| {
                let bytes = &files.iter().find(|f| f.path == r.file).unwrap().contents;
                (r.prefix.clone(), fmt::decode(bytes).unwrap())
            })
            .collect();

        let hits = fmt::rank(&meta, "search", |prefix| shard_map.get(prefix), 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(meta.docs[hits[0].doc as usize].url, "/a/");
    }
}
