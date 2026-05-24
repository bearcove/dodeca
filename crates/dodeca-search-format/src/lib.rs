//! On-disk format and query engine for dodeca's from-scratch full-text search.
//!
//! This is the single source of truth shared by the *writer* (`cell-search`,
//! which builds the index at site-build time) and the *reader*
//! (`dodeca-search-wasm`, which runs queries in the browser). Because both
//! sides are compiled from the same dodeca version, the postcard schema is
//! always in sync — there is no cross-version compatibility surface, and the
//! cache-busted asset paths handle staleness.
//!
//! Layout (all files are postcard-serialized, see [`encode`]/[`decode`]):
//!
//! - `/search/index/meta` — [`SearchMeta`], the stable manifest. Lists every
//!   document and which inverted-index shard holds which term prefix.
//! - `/search/index/<hash>` — a [`Shard`]: the postings for all terms sharing
//!   a one-character prefix.
//! - `/search/fragment/<hash>` — a [`Fragment`]: per-document display data
//!   (title, word list, headings) used to render results and excerpts.
//!
//! The split mirrors pagefind's design: the browser fetches `meta` once, then
//! lazily pulls only the shards a query touches and only the fragments for the
//! results it actually shows.

use facet::Facet;
use unicode_segmentation::UnicodeSegmentation;

/// Bumped on any change to the structs below. The writer stamps it into
/// [`SearchMeta::version`]; the reader refuses anything it doesn't recognize.
pub const FORMAT_VERSION: u32 = 1;

/// Stable manifest. Path: `/search/index/meta`.
// s[impl format.manifest]
#[derive(Debug, Clone, Facet)]
pub struct SearchMeta {
    pub version: u32,
    /// Mean document length in tokens, for BM25 length normalization.
    pub avg_doc_len: f32,
    /// Documents, addressed by [`DocId`] (their index in this vec).
    pub docs: Vec<DocMeta>,
    /// Inverted-index shards, one per one-character term prefix.
    pub shards: Vec<ShardRef>,
}

/// Index into [`SearchMeta::docs`].
pub type DocId = u32;

#[derive(Debug, Clone, Facet)]
pub struct DocMeta {
    pub url: String,
    pub title: String,
    /// Document length in tokens (BM25 normalization).
    pub len: u32,
    /// Filename (under `/search/fragment/`) of this document's [`Fragment`].
    pub fragment: String,
}

#[derive(Debug, Clone, Facet)]
pub struct ShardRef {
    /// Lowercased first character of the stemmed terms in this shard.
    /// Empty string is the catch-all bucket for terms that don't start
    /// with an ASCII alphanumeric.
    pub prefix: String,
    /// Filename under `/search/index/`.
    pub file: String,
}

/// Postings for every term sharing one prefix. Path: `/search/index/<hash>`.
// s[impl format.terms-sorted]
#[derive(Debug, Clone, Facet)]
pub struct Shard {
    /// Sorted by [`TermPostings::term`] so the reader can binary-search and
    /// range-scan a prefix.
    pub terms: Vec<TermPostings>,
}

#[derive(Debug, Clone, Facet)]
pub struct TermPostings {
    /// The stemmed term.
    pub term: String,
    /// Documents containing it, sorted by [`Posting::doc`].
    pub postings: Vec<Posting>,
}

// s[impl format.postings]
#[derive(Debug, Clone, Facet)]
pub struct Posting {
    pub doc: DocId,
    /// Token positions of this term within the document's word list,
    /// ascending. Its length is the in-document term frequency.
    pub positions: Vec<u32>,
}

/// Per-document display data. Path: `/search/fragment/<hash>`.
// s[impl format.fragment]
#[derive(Debug, Clone, Facet)]
pub struct Fragment {
    pub url: String,
    pub title: String,
    /// Display tokens (original casing, not stemmed) for excerpt rendering.
    /// Positions in [`Posting::positions`] index into this vec.
    pub words: Vec<String>,
    /// Headings, for sub-result deep links.
    pub anchors: Vec<Anchor>,
}

#[derive(Debug, Clone, Facet)]
pub struct Anchor {
    /// Element `id` to deep-link to (`url#id`).
    pub id: String,
    pub text: String,
    /// Word position where this heading starts.
    pub position: u32,
}

/// Serialize any format struct. Compact, not self-describing — fine because
/// the same dodeca build produces and consumes it.
// s[impl format.encoding]
pub fn encode<T: Facet<'static>>(value: &T) -> Result<Vec<u8>, String> {
    facet_postcard::to_vec(value).map_err(|e| e.to_string())
}

/// Deserialize a format struct produced by [`encode`].
pub fn decode<T: Facet<'static>>(bytes: &[u8]) -> Result<T, String> {
    facet_postcard::from_slice(bytes).map_err(|e| e.to_string())
}

// ============================================================================
// Analysis — shared by indexer and query so they tokenize/stem identically.
// ============================================================================

/// A single analyzed token: the original surface form (for display/excerpts)
/// and its stemmed form (the indexed key).
pub struct Token {
    pub display: String,
    pub stem: String,
}

fn stemmer() -> rust_stemmers::Stemmer {
    rust_stemmers::Stemmer::create(rust_stemmers::Algorithm::English)
}

/// Tokenize text into ordered tokens. Word boundaries follow UAX#29; tokens
/// are lowercased before stemming. Identical logic must run at index time and
/// query time, which is why it lives here.
// s[impl analyze.tokenize]
// s[impl analyze.lowercase]
// s[impl analyze.stem]
// s[impl analyze.display-form]
// s[impl analyze.consistent]
pub fn analyze(text: &str) -> Vec<Token> {
    let stemmer = stemmer();
    text.unicode_words()
        .map(|w| {
            let lower = w.to_lowercase();
            let stem = stemmer.stem(&lower).into_owned();
            Token {
                display: w.to_string(),
                stem,
            }
        })
        .collect()
}

/// Stem-only analysis, for query terms (we don't need surface forms there).
pub fn analyze_stems(text: &str) -> Vec<String> {
    analyze(text).into_iter().map(|t| t.stem).collect()
}

/// The shard a stemmed term belongs to: its lowercased first ASCII
/// alphanumeric character, or the empty catch-all bucket.
// s[impl format.shard-prefix]
pub fn shard_prefix(stem: &str) -> String {
    match stem.chars().next() {
        Some(c) if c.is_ascii_alphanumeric() => c.to_ascii_lowercase().to_string(),
        _ => String::new(),
    }
}

// ============================================================================
// Query engine — pure, so it is unit-testable natively and reused by wasm.
// ============================================================================

/// One ranked result, before excerpting.
#[derive(Debug, Clone)]
pub struct Hit {
    pub doc: DocId,
    pub score: f32,
    /// Positions in the document where any query term matched (ascending),
    /// used to pick the excerpt window.
    pub match_positions: Vec<u32>,
}

const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;

/// A query slot is the set of indexed terms acceptable for one query word.
/// All but the last word are exact-stem; the last word also accepts terms
/// that *start with* its stem, giving as-you-type behavior cheaply (shard
/// terms are sorted, so this is a range scan, not a full scan).
fn query_slots(query: &str) -> Vec<QuerySlot> {
    let stems = analyze_stems(query);
    let n = stems.len();
    stems
        .into_iter()
        .enumerate()
        .map(|(i, stem)| QuerySlot {
            prefix_match: i + 1 == n,
            stem,
        })
        .collect()
}

struct QuerySlot {
    stem: String,
    /// If true, match any term beginning with `stem`, not just `stem` itself.
    prefix_match: bool,
}

/// Distinct shard prefixes a query needs, so the loader knows what to fetch.
// s[impl query.shard-selection]
pub fn shards_for_query(query: &str) -> Vec<String> {
    let mut prefixes: Vec<String> = query_slots(query)
        .iter()
        .map(|s| shard_prefix(&s.stem))
        .collect();
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

/// Look up every term in `shard` matching `slot`, returning their postings.
// s[impl query.prefix]
fn slot_postings<'a>(slot: &QuerySlot, shard: &'a Shard) -> Vec<&'a TermPostings> {
    if slot.prefix_match {
        // shard.terms is sorted: take the contiguous run of terms with the prefix.
        let start = shard
            .terms
            .partition_point(|t| t.term.as_str() < slot.stem.as_str());
        shard.terms[start..]
            .iter()
            .take_while(|t| t.term.starts_with(&slot.stem))
            .collect()
    } else {
        shard
            .terms
            .binary_search_by(|t| t.term.as_str().cmp(slot.stem.as_str()))
            .ok()
            .map(|i| vec![&shard.terms[i]])
            .unwrap_or_default()
    }
}

/// Rank documents for `query`. `shard_for` resolves a prefix (from
/// [`shards_for_query`]) to its loaded [`Shard`], or `None` if absent.
///
/// Semantics: AND across query words (a document must match every slot),
/// scored by summed BM25 (best term per slot). Returns hits sorted by
/// descending score, capped at `limit`.
// s[impl query.and]
// s[impl query.bm25]
pub fn rank<'a>(
    meta: &SearchMeta,
    query: &str,
    shard_for: impl Fn(&str) -> Option<&'a Shard>,
    limit: usize,
) -> Vec<Hit> {
    let slots = query_slots(query);
    if slots.is_empty() {
        return Vec::new();
    }
    let n_docs = meta.docs.len().max(1) as f32;
    let avg_dl = if meta.avg_doc_len > 0.0 {
        meta.avg_doc_len
    } else {
        1.0
    };

    // Per document: the best BM25 contribution for each slot (a prefix slot
    // can match several indexed terms — only the strongest counts) and the
    // union of matched positions, used to pick the excerpt window.
    use std::collections::HashMap;
    struct Acc {
        slot_best: Vec<f32>,
        positions: Vec<u32>,
    }
    let mut acc: HashMap<DocId, Acc> = HashMap::new();

    for (slot_idx, slot) in slots.iter().enumerate() {
        let Some(shard) = shard_for(&shard_prefix(&slot.stem)) else {
            // A required slot whose shard is missing can never be satisfied;
            // the AND filter below then drops every document.
            continue;
        };
        for tp in slot_postings(slot, shard) {
            let df = tp.postings.len() as f32;
            // BM25 IDF, always positive.
            let idf = ((n_docs - df + 0.5) / (df + 0.5) + 1.0).ln();
            for p in &tp.postings {
                let Some(dm) = meta.docs.get(p.doc as usize) else {
                    continue;
                };
                let tf = p.positions.len() as f32;
                let dl = dm.len.max(1) as f32;
                let denom = tf + BM25_K1 * (1.0 - BM25_B + BM25_B * dl / avg_dl);
                let term_score = idf * (tf * (BM25_K1 + 1.0)) / denom;
                let e = acc.entry(p.doc).or_insert_with(|| Acc {
                    slot_best: vec![0.0; slots.len()],
                    positions: Vec::new(),
                });
                let best = &mut e.slot_best[slot_idx];
                *best = best.max(term_score);
                e.positions.extend_from_slice(&p.positions);
            }
        }
    }

    let mut hits: Vec<Hit> = acc
        .into_iter()
        // AND: every slot must have contributed something.
        .filter(|(_, a)| a.slot_best.iter().all(|s| *s > 0.0))
        .map(|(doc, mut a)| {
            a.positions.sort_unstable();
            a.positions.dedup();
            Hit {
                doc,
                score: a.slot_best.iter().sum(),
                match_positions: a.positions,
            }
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.doc.cmp(&b.doc))
    });
    hits.truncate(limit);
    hits
}

/// A rendered search result.
#[derive(Debug, Clone, Facet)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    /// HTML excerpt with matched words wrapped in `<mark>`. Already escaped.
    pub excerpt: String,
    pub score: f32,
}

const EXCERPT_WORDS: usize = 30;

/// Build the displayable result for `hit` from its [`Fragment`]. Picks the
/// densest window of matched words, wraps matches in `<mark>`, deep-links to
/// the nearest preceding heading, and appends a text-fragment directive so a
/// supporting browser scrolls straight to the match.
// s[impl render.excerpt]
// s[impl render.mark]
// s[impl render.deeplink]
// s[impl render.text-fragment]
pub fn render(hit: &Hit, fragment: &Fragment) -> SearchResult {
    let words = &fragment.words;
    let matched: std::collections::HashSet<u32> = hit.match_positions.iter().copied().collect();

    // Slide an EXCERPT_WORDS window; keep the one covering the most matches.
    let (mut best_start, mut best_hits) = (0usize, -1i32);
    if !words.is_empty() {
        let last_start = words.len().saturating_sub(EXCERPT_WORDS);
        for start in 0..=last_start {
            let end = (start + EXCERPT_WORDS).min(words.len());
            let count = (start..end)
                .filter(|i| matched.contains(&(*i as u32)))
                .count() as i32;
            if count > best_hits {
                best_hits = count;
                best_start = start;
            }
            if start == last_start {
                break;
            }
        }
    }
    let end = (best_start + EXCERPT_WORDS).min(words.len());

    let mut excerpt = String::new();
    if best_start > 0 {
        excerpt.push('…');
    }
    for (i, word) in words.iter().enumerate().take(end).skip(best_start) {
        if i > best_start {
            excerpt.push(' ');
        }
        let escaped = escape_html(word);
        if matched.contains(&(i as u32)) {
            excerpt.push_str("<mark>");
            excerpt.push_str(&escaped);
            excerpt.push_str("</mark>");
        } else {
            excerpt.push_str(&escaped);
        }
    }
    if end < words.len() {
        excerpt.push('…');
    }

    // Deep-link to the heading the matched text falls under: anchor on the
    // first matched word in the excerpt window (or the window start if the
    // window holds no matches at all), then pick the nearest heading at or
    // before it.
    let anchor_pos = (best_start..end)
        .find(|i| matched.contains(&(*i as u32)))
        .unwrap_or(best_start);
    let anchor = fragment
        .anchors
        .iter()
        .filter(|a| (a.position as usize) <= anchor_pos)
        .max_by_key(|a| a.position);
    let mut url = match anchor {
        Some(a) if !a.id.is_empty() => format!("{}#{}", fragment.url, a.id),
        _ => fragment.url.clone(),
    };

    // Append a text-fragment directive for one matched word, disambiguated by
    // neighboring words when possible. A `start,end` range across non-adjacent
    // query terms lets the browser highlight every intervening node, including
    // site chrome that appears before `<main>`.
    let window_matches: Vec<usize> = (best_start..end)
        .filter(|i| matched.contains(&(*i as u32)))
        .collect();
    if let Some(directive) = text_fragment_directive(words, &window_matches) {
        if !url.contains('#') {
            url.push('#');
        }
        url.push_str(":~:");
        url.push_str(&directive);
    }

    SearchResult {
        url,
        title: fragment.title.clone(),
        excerpt,
        score: hit.score,
    }
}

fn text_fragment_directive(words: &[String], matches: &[usize]) -> Option<String> {
    let &pos = matches
        .iter()
        .max_by_key(|&&pos| usize::from(pos > 0) + usize::from(pos + 1 < words.len()))?;
    let mut directive = String::from("text=");
    if pos > 0 {
        directive.push_str(&percent_encode(&words[pos - 1]));
        directive.push_str("-,");
    }
    directive.push_str(&percent_encode(&words[pos]));
    if pos + 1 < words.len() {
        directive.push_str(",-");
        directive.push_str(&percent_encode(&words[pos + 1]));
    }
    Some(directive)
}

/// Percent-encode a word for use inside a `:~:text=` directive. Only
/// `A-Za-z0-9_.~` pass through; every other byte (including the directive
/// delimiters `-` and `,`, and all non-ASCII) is `%`-escaped.
fn percent_encode(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for byte in word.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}
