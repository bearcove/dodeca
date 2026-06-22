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

use crate::db::{Page, Section, SiteTree};
use crate::types::Route;

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
    /// whole-word boundaries, never link `self_route`, and only the first
    /// mention of each target across the page (`seen` is threaded by the caller
    /// across every text run in document order).
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

    #[test]
    fn matches_whole_words_only() {
        let index = idx(&[("leads", "/leads")]);
        // "leaders" must not match the embedded "leads".
        assert_eq!(links(&index, "the leaders met", "/x"), vec![]);
        assert_eq!(
            links(&index, "follow the leads here", "/x"),
            vec![(11, 16, "/leads".into())]
        );
    }

    #[test]
    fn is_case_insensitive_but_keeps_source_offsets() {
        let index = idx(&[("Methodology", "/methodology")]);
        let got = links(&index, "Our methodology is simple", "/x");
        assert_eq!(got, vec![(4, 15, "/methodology".into())]);
    }

    #[test]
    fn longest_phrase_wins() {
        let index = idx(&[("dead", "/dead"), ("not dead ends", "/nde")]);
        assert_eq!(
            links(&index, "these are not dead ends really", "/x"),
            vec![(10, 23, "/nde".into())]
        );
    }

    #[test]
    fn first_occurrence_per_target_only() {
        let index = idx(&[("leads", "/leads")]);
        let got = links(&index, "leads and more leads and leads", "/leads-x");
        assert_eq!(got, vec![(0, 5, "/leads".into())]);
    }

    #[test]
    fn never_links_to_self() {
        let index = idx(&[("Ledger", "/ledger")]);
        assert_eq!(
            links(&index, "This ledger is append-only", "/ledger"),
            vec![]
        );
    }
}
