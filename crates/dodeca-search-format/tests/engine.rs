//! Query-engine and format tests for `dodeca-search-format`.
//!
//! Exercises the public API exactly as `cell-search` and `dodeca-search-wasm`
//! drive it. Kept in `tests/` rather than inline so tracey scans it as
//! verification of `docs/spec/search.md`.

use dodeca_search_format::*;

// s[verify analyze.lowercase]
// s[verify analyze.stem]
// s[verify analyze.display-form]
// s[verify analyze.tokenize]
// s[verify analyze.consistent]
#[test]
fn analyze_lowercases_and_stems() {
    let toks = analyze("Running RUNS runner");
    let stems: Vec<&str> = toks.iter().map(|t| t.stem.as_str()).collect();
    // English Snowball collapses these to the same root.
    assert_eq!(stems, vec!["run", "run", "runner"]);
    assert_eq!(toks[0].display, "Running");
}

// s[verify format.shard-prefix]
#[test]
fn shard_prefix_buckets() {
    assert_eq!(shard_prefix("run"), "r");
    assert_eq!(shard_prefix("3d"), "3");
    assert_eq!(shard_prefix("中文"), "");
}

fn tp(term: &str, postings: &[(u32, &[u32])]) -> TermPostings {
    TermPostings {
        term: term.into(),
        postings: postings
            .iter()
            .map(|(d, ps)| Posting {
                doc: *d,
                positions: ps.to_vec(),
            })
            .collect(),
    }
}

// s[verify query.and]
// s[verify query.bm25]
// s[verify query.prefix]
#[test]
fn rank_is_and_and_orders_by_bm25() {
    let meta = SearchMeta {
        version: FORMAT_VERSION,
        avg_doc_len: 10.0,
        docs: vec![
            DocMeta {
                url: "/a/".into(),
                title: "A".into(),
                source: String::new(),
                len: 10,
                fragment: "a".into(),
            },
            DocMeta {
                url: "/b/".into(),
                title: "B".into(),
                source: String::new(),
                len: 10,
                fragment: "b".into(),
            },
        ],
        shards: vec![],
    };
    // "cell" in both docs (twice in doc 0), "search" only in doc 0.
    let s_c = Shard {
        terms: vec![tp("cell", &[(0, &[1, 5]), (1, &[2])])],
    };
    let s_s = Shard {
        terms: vec![tp("search", &[(0, &[2])])],
    };
    let shard_for = |p: &str| match p {
        "c" => Some(&s_c),
        "s" => Some(&s_s),
        _ => None,
    };

    // Single term: both match, doc 0 ranks first (higher tf).
    let hits = rank(&meta, "cell", shard_for, 10);
    assert_eq!(hits.iter().map(|h| h.doc).collect::<Vec<_>>(), vec![0, 1]);

    // AND: only doc 0 has both "cell" and "search".
    let hits = rank(&meta, "cell search", shard_for, 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc, 0);
}

// s[verify query.shard-selection]
#[test]
fn shards_for_query_dedups_prefixes() {
    // Two words sharing a prefix yield one shard; distinct prefixes yield more.
    assert_eq!(shards_for_query("cell css"), vec!["c"]);
    let mut both = shards_for_query("cell search");
    both.sort();
    assert_eq!(both, vec!["c", "s"]);
}

// s[verify render.excerpt]
// s[verify render.mark]
// s[verify render.deeplink]
// s[verify render.text-fragment]
#[test]
fn render_marks_matches_and_truncates() {
    let frag = Fragment {
        url: "/p/".into(),
        title: "Page".into(),
        words: (0..50).map(|i| format!("w{i}")).collect(),
        anchors: vec![Anchor {
            id: "sec".into(),
            text: "Section".into(),
            position: 5,
        }],
    };
    let hit = Hit {
        doc: 0,
        score: 1.0,
        match_positions: vec![10],
    };
    let r = render(&hit, &frag);
    assert!(r.excerpt.contains("<mark>w10</mark>"));
    assert!(r.excerpt.contains('…'));
    // Heading anchor plus a locally-contextualized text-fragment directive at
    // the match.
    assert_eq!(r.url, "/p/#sec:~:text=w9-,w10,-w11");
}

// s[verify render.text-fragment]
#[test]
fn render_text_fragment_uses_one_contextualized_match_and_escapes() {
    let frag = Fragment {
        url: "/p/".into(),
        title: "Page".into(),
        // A space-bearing word forces percent-encoding of the directive. The
        // two matches are intentionally non-adjacent: the URL should not use a
        // `start,end` range that would highlight the unrelated word between.
        words: vec!["alpha".into(), "mid".into(), "om ga".into(), "tail".into()],
        anchors: vec![],
    };
    let hit = Hit {
        doc: 0,
        score: 1.0,
        match_positions: vec![0, 2],
    };
    let r = render(&hit, &frag);
    assert_eq!(r.url, "/p/#:~:text=mid-,om%20ga,-tail");
}

// s[verify format.encoding]
// s[verify format.manifest]
// s[verify format.terms-sorted]
// s[verify format.postings]
// s[verify format.fragment]
#[test]
fn postcard_roundtrips() {
    let shard = Shard {
        terms: vec![tp("cell", &[(0, &[1, 2])])],
    };
    let bytes = encode(&shard).unwrap();
    let back: Shard = decode(&bytes).unwrap();
    assert_eq!(back.terms[0].term, "cell");
    assert_eq!(back.terms[0].postings[0].positions, vec![1, 2]);
}
