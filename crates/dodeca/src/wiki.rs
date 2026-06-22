//! Wiki auto-linking: scope, alias index, and matcher.
//!
//! A *wiki* is a section subtree: a section declares `wiki = true` in its
//! `_index.md` `[extra]`, and every descendant inherits it. Within a wiki, a
//! bare mention of another wiki page's title (or one of its `aliases`) is turned
//! into a link **at render time** — the source markdown is never modified.
//!
//! This module owns the three pure pieces: [`route_in_wiki`] (scope),
//! [`AliasIndex::build`] (the target set: title/alias → route), and
//! [`AliasIndex::find_links`] (the matching policy). The HTML transform that
//! applies them to a page's body lives in the render pipeline.

use std::collections::HashSet;

use aho_corasick::{AhoCorasick, MatchKind};
use hotmeal::{
    Document, LocalName, NodeId, NodeKind, QualName, StrTendril, ns, parse_body_fragment,
};

use crate::db::{Page, Section, SiteTree};
use crate::types::{HtmlBody, Route};

/// Elements whose text must never be auto-linked: existing links, code, and
/// headings. Auto-linking descends into everything else.
const SKIP_TAGS: &[&str] = &["a", "code", "pre", "kbd", "script", "style"];

fn is_skip_tag(tag: &str) -> bool {
    SKIP_TAGS.contains(&tag) || matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

/// Whether `route` falls inside a wiki: true if the section at `route` or any
/// ancestor section sets `wiki = true` in its `[extra]` frontmatter.
pub fn route_in_wiki(route: &Route, tree: &SiteTree) -> bool {
    let mut current = route.clone();
    loop {
        if let Some(section) = tree.sections.get(&current)
            && section_wiki_flag(section)
        {
            return true;
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return false,
        }
    }
}

fn section_wiki_flag(section: &Section) -> bool {
    section
        .extra
        .as_object()
        .and_then(|o| o.get("wiki"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Extra link triggers for a page beyond its title, from `aliases = ["…", …]`
/// in `[extra]`. Non-string entries are ignored.
fn page_aliases(page: &Page) -> Vec<String> {
    let Some(array) = page
        .extra
        .as_object()
        .and_then(|o| o.get("aliases"))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|v| v.as_string().map(|s| s.as_str().trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// One located auto-link: the byte range within the scanned text and the route
/// it should link to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMatch {
    pub start: usize,
    pub end: usize,
    pub route: Route,
}

/// A built automaton over every wiki page's title and aliases, mapping each
/// matched phrase back to its route.
pub struct AliasIndex {
    ac: AhoCorasick,
    /// Route for each pattern, indexed by the automaton's `PatternID`.
    routes: Vec<Route>,
}

impl AliasIndex {
    /// Build the index from the wiki pages in `tree`. Returns `None` if there
    /// are no wiki pages (so the caller can skip the transform entirely).
    pub fn build(tree: &SiteTree) -> Option<Self> {
        let mut entries: Vec<(String, Route)> = Vec::new();
        for page in tree.pages.values() {
            if !route_in_wiki(&page.route, tree) {
                continue;
            }
            let title = page.title.as_str().trim();
            if !title.is_empty() {
                entries.push((title.to_string(), page.route.clone()));
            }
            for alias in page_aliases(page) {
                entries.push((alias, page.route.clone()));
            }
        }
        Self::from_entries(entries)
    }

    /// Core constructor over `(phrase, route)` pairs. Longer phrases win when
    /// they overlap (leftmost-longest), so "not dead ends" beats "dead".
    pub fn from_entries(entries: Vec<(String, Route)>) -> Option<Self> {
        if entries.is_empty() {
            return None;
        }
        let (patterns, routes): (Vec<String>, Vec<Route>) = entries.into_iter().unzip();
        let ac = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .match_kind(MatchKind::LeftmostLongest)
            .build(&patterns)
            .ok()?;
        Some(Self { ac, routes })
    }

    /// Locate auto-links within one run of plain text. Honors the policy:
    /// whole-word boundaries; the eligibility rule (see [`is_eligible`]) that
    /// keeps common single words from linking unless capitalized; never link
    /// `self_route`; and only the first mention of each target across the page
    /// (`seen` is threaded by the caller across every text run in document
    /// order). An ineligible occurrence is skipped without consuming a target's
    /// one slot, so a later eligible mention can still link.
    pub fn find_links(
        &self,
        text: &str,
        self_route: &Route,
        seen: &mut HashSet<Route>,
    ) -> Vec<LinkMatch> {
        let mut out = Vec::new();
        for m in self.ac.find_iter(text) {
            let (start, end) = (m.start(), m.end());
            if !on_word_boundaries(text, start, end) {
                continue;
            }
            if !is_eligible(&text[start..end], at_sentence_start(text, start)) {
                continue;
            }
            let route = &self.routes[m.pattern().as_usize()];
            if route == self_route || seen.contains(route) {
                continue;
            }
            seen.insert(route.clone());
            out.push(LinkMatch {
                start,
                end,
                route: route.clone(),
            });
        }
        out
    }
}

/// Whether a matched title occurrence in prose is distinctive enough to link:
/// a multi-word phrase always is; a single word is if it isn't a common English
/// word ([`crate::dictionary`]); a common single word only when it appears
/// Capitalized *mid-sentence* — the capital hints "this is a page", but at a
/// sentence start the capital is just grammar, so we fall back to skipping it.
fn is_eligible(matched: &str, at_sentence_start: bool) -> bool {
    if matched.chars().any(char::is_whitespace) {
        return true;
    }
    if !crate::dictionary::is_common_word(matched) {
        return true;
    }
    let capitalized = matched.chars().next().is_some_and(char::is_uppercase);
    capitalized && !at_sentence_start
}

/// Whether the match at byte `start` begins a sentence: it sits at the start of
/// the text run, or the previous non-space character is a sentence terminator.
fn at_sentence_start(text: &str, start: usize) -> bool {
    match text[..start].trim_end().chars().next_back() {
        None => true,
        Some(c) => matches!(c, '.' | '!' | '?'),
    }
}

/// Rewrite the body HTML of every wiki page and section in `tree` to auto-link
/// bare mentions of other wiki pages. No-op when the site has no wiki pages.
/// Called once the tree is fully assembled, since the alias index needs every
/// title; the markdown source is never touched (this only rewrites `body_html`).
pub fn apply_auto_links(tree: &mut SiteTree) {
    let Some(index) = AliasIndex::build(tree) else {
        return;
    };
    // Collect the wiki routes up front (immutable), then rewrite by route so we
    // never hold an index/tree borrow across a mutable page borrow.
    let pages: Vec<Route> = tree
        .pages
        .keys()
        .filter(|r| route_in_wiki(r, tree))
        .cloned()
        .collect();
    let sections: Vec<Route> = tree
        .sections
        .keys()
        .filter(|r| route_in_wiki(r, tree))
        .cloned()
        .collect();

    for route in pages {
        let page = &tree.pages[&route];
        let linked = auto_link_body(page.body_html.as_str(), &index, &page.route);
        if let Some(page) = tree.pages.get_mut(&route) {
            page.body_html = HtmlBody::new(linked);
        }
    }
    for route in sections {
        let section = &tree.sections[&route];
        let linked = auto_link_body(section.body_html.as_str(), &index, &section.route);
        if let Some(section) = tree.sections.get_mut(&route) {
            section.body_html = HtmlBody::new(linked);
        }
    }
}

/// Auto-link a page's rendered body HTML: turn the first bare mention of each
/// other wiki page into an `<a class="wiki-link" data-autolink>`, leaving the
/// markdown source untouched. Skips text inside links, code, and headings, and
/// never links the page to itself. Returns the body unchanged when nothing
/// matches (so untouched pages don't pay a re-serialize).
pub fn auto_link_body(html: &str, index: &AliasIndex, self_route: &Route) -> String {
    let tendril = StrTendril::from(html);
    let mut doc = parse_body_fragment(&tendril);
    // A body fragment parses with its nodes directly under `root`, not a `<body>`.
    let root = doc.root;

    // Phase 1: collect the text nodes to rewrite (in document order, threading
    // `seen` so only the first mention of each target links). Mutating the arena
    // while walking it would invalidate the walk, so we gather first.
    let mut seen: HashSet<Route> = HashSet::new();
    let mut targets: Vec<(NodeId, Vec<LinkMatch>, String)> = Vec::new();
    collect_targets(
        &doc,
        root,
        false,
        index,
        self_route,
        &mut seen,
        &mut targets,
    );
    if targets.is_empty() {
        return html.to_string();
    }

    // Phase 2: splice each text node into [text?, <a>…</a>, text?, …].
    for (text_node, matches, text) in targets {
        let mut cursor = 0usize;
        for m in matches {
            if m.start > cursor {
                let lead = doc.create_text(text[cursor..m.start].to_string());
                doc.insert_before(text_node, lead);
            }
            let anchor = doc.create_element(LocalName::from("a"));
            doc.set_attr(anchor, attr("href"), m.route.as_str().to_string());
            doc.set_attr(anchor, attr("class"), "wiki-link");
            doc.set_attr(anchor, attr("data-autolink"), "");
            let label = doc.create_text(text[m.start..m.end].to_string());
            doc.append_child(anchor, label);
            doc.insert_before(text_node, anchor);
            cursor = m.end;
        }
        if cursor < text.len() {
            let tail = doc.create_text(text[cursor..].to_string());
            doc.insert_before(text_node, tail);
        }
        doc.remove(text_node);
    }

    doc.serialize_inner_html(root)
}

/// An unnamespaced attribute QualName (HTML attributes carry no namespace).
fn attr(local: &str) -> QualName {
    QualName::new(None, ns!(), LocalName::from(local))
}

/// Walk `id`'s subtree in document order, recording text nodes (and their
/// matches) that should be auto-linked. `skip` is true inside link/code/heading
/// elements, whose text is left alone.
fn collect_targets(
    doc: &Document<'_>,
    id: NodeId,
    skip: bool,
    index: &AliasIndex,
    self_route: &Route,
    seen: &mut HashSet<Route>,
    out: &mut Vec<(NodeId, Vec<LinkMatch>, String)>,
) {
    match &doc.get(id).kind {
        NodeKind::Text(text) if !skip => {
            let text = text.as_ref().to_string();
            let matches = index.find_links(&text, self_route, seen);
            if !matches.is_empty() {
                out.push((id, matches, text));
            }
        }
        NodeKind::Element(elem) => {
            let child_skip = skip || is_skip_tag(elem.tag.as_ref());
            for child in doc.children(id).collect::<Vec<_>>() {
                collect_targets(doc, child, child_skip, index, self_route, seen, out);
            }
        }
        _ => {
            for child in doc.children(id).collect::<Vec<_>>() {
                collect_targets(doc, child, skip, index, self_route, seen, out);
            }
        }
    }
}

/// Whether `[start, end)` in `text` is flanked by non-word characters (so
/// "leads" matches the word but not the tail of "leaders"). A word char is an
/// alphanumeric; everything else (and the string edges) is a boundary.
fn on_word_boundaries(text: &str, start: usize, end: usize) -> bool {
    let before_ok = text[..start]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric());
    let after_ok = text[end..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric());
    before_ok && after_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(pairs: &[(&str, &str)]) -> AliasIndex {
        AliasIndex::from_entries(
            pairs
                .iter()
                .map(|(p, r)| (p.to_string(), Route::from(*r)))
                .collect(),
        )
        .expect("non-empty")
    }

    fn links(index: &AliasIndex, text: &str, self_route: &str) -> Vec<(usize, usize, String)> {
        let mut seen = HashSet::new();
        index
            .find_links(text, &Route::from(self_route), &mut seen)
            .into_iter()
            .map(|m| (m.start, m.end, m.route.as_str().to_string()))
            .collect()
    }

    // `bearcove`/`dodeca` are coined here (not in CMUdict), so they're eligible
    // regardless of case — used to isolate matching behaviour from the
    // dictionary/capitalization policy, which the later tests cover directly.

    #[test]
    fn matches_whole_words_only() {
        let index = idx(&[("dodeca", "/dodeca")]);
        // Embedded in "dodecahedron" — not a whole word, so no match.
        assert_eq!(links(&index, "the dodecahedron spins", "/x"), vec![]);
        assert_eq!(
            links(&index, "follow the dodeca here", "/x"),
            vec![(11, 17, "/dodeca".into())]
        );
    }

    #[test]
    fn matching_is_case_insensitive_and_keeps_source_offsets() {
        let index = idx(&[("bearcove", "/bearcove")]);
        // Pattern is lowercase; the capitalized occurrence still matches.
        assert_eq!(
            links(&index, "the Bearcove tool", "/x"),
            vec![(4, 12, "/bearcove".into())]
        );
    }

    #[test]
    fn longest_phrase_wins() {
        let index = idx(&[("dodeca", "/dodeca"), ("dodeca engine", "/engine")]);
        assert_eq!(
            links(&index, "the dodeca engine runs", "/x"),
            vec![(4, 17, "/engine".into())]
        );
    }

    #[test]
    fn first_occurrence_per_target_only() {
        let index = idx(&[("bearcove", "/bearcove")]);
        let got = links(&index, "bearcove and bearcove and bearcove", "/x");
        assert_eq!(got, vec![(0, 8, "/bearcove".into())]);
    }

    #[test]
    fn never_links_to_self() {
        let index = idx(&[("bearcove", "/bearcove")]);
        assert_eq!(links(&index, "the bearcove page", "/bearcove"), vec![]);
    }

    // ── dictionary + capitalization eligibility ──────────────────────────────

    #[test]
    fn common_single_word_lowercase_is_skipped() {
        // "leads" is a CMUdict word; a bare lowercase mention is not distinctive.
        let index = idx(&[("leads", "/leads")]);
        assert_eq!(links(&index, "follow the leads here", "/x"), vec![]);
    }

    #[test]
    fn common_word_capitalized_midsentence_links() {
        // The capital mid-sentence hints "this is a page".
        let index = idx(&[("leads", "/leads")]);
        assert_eq!(
            links(&index, "we track Leads here", "/x"),
            vec![(9, 14, "/leads".into())]
        );
    }

    #[test]
    fn common_word_capitalized_at_sentence_start_is_skipped() {
        // At a sentence start the capital is just grammar — ambiguous, so skip.
        let index = idx(&[("ledger", "/ledger")]);
        assert_eq!(links(&index, "Ledger is append-only.", "/x"), vec![]);
        // ...but a second, mid-sentence capital still links.
        assert_eq!(
            links(&index, "Ledger is the Ledger.", "/x"),
            vec![(14, 20, "/ledger".into())]
        );
    }

    #[test]
    fn novel_single_word_links_even_lowercase() {
        let index = idx(&[("bearcove", "/bearcove")]);
        assert_eq!(
            links(&index, "built by bearcove here", "/x"),
            vec![(9, 17, "/bearcove".into())]
        );
    }

    #[test]
    fn multiword_phrase_links_even_with_common_words() {
        let index = idx(&[("verified facts", "/ledger")]);
        assert_eq!(
            links(&index, "only verified facts matter", "/x"),
            vec![(5, 19, "/ledger".into())]
        );
    }

    fn linked(html: &str, pairs: &[(&str, &str)], self_route: &str) -> String {
        auto_link_body(html, &idx(pairs), &Route::from(self_route))
    }

    #[test]
    fn auto_links_a_bare_mention_in_prose() {
        let out = linked(
            "<p>Built on bearcove tooling.</p>",
            &[("bearcove", "/orgs/bearcove")],
            "/x",
        );
        assert_eq!(
            out,
            r#"<p>Built on <a href="/orgs/bearcove" class="wiki-link" data-autolink="">bearcove</a> tooling.</p>"#
        );
    }

    #[test]
    fn leaves_code_links_and_headings_alone() {
        let pairs = &[("bearcove", "/bearcove")][..];
        assert_eq!(
            linked("<p>run <code>bearcove</code> now</p>", pairs, "/x"),
            "<p>run <code>bearcove</code> now</p>"
        );
        // Explicit link wins — we don't touch text already inside an <a>.
        assert_eq!(
            linked(r#"<p>see <a href="/b">bearcove</a></p>"#, pairs, "/x"),
            r#"<p>see <a href="/b">bearcove</a></p>"#
        );
        assert_eq!(
            linked("<h2>bearcove</h2>", pairs, "/x"),
            "<h2>bearcove</h2>"
        );
    }

    #[test]
    fn first_mention_across_separate_elements() {
        let out = linked(
            "<p>about bearcove here</p><p>more bearcove there</p>",
            &[("bearcove", "/bearcove")],
            "/x",
        );
        assert_eq!(
            out,
            r#"<p>about <a href="/bearcove" class="wiki-link" data-autolink="">bearcove</a> here</p><p>more bearcove there</p>"#
        );
    }

    #[test]
    fn untouched_body_is_returned_verbatim() {
        let html = "<p>nothing to link here</p>";
        assert_eq!(linked(html, &[("bearcove", "/bearcove")], "/x"), html);
    }
}
