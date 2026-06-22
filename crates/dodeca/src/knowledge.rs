//! Semantic knowledge API: embed page text and answer nearest-neighbor queries.
//!
//! This backs the dev-only well-known endpoint `/_dodeca/knowledge/search`, so an
//! agent can `curl` the knowledge base and get the most relevant pages for a
//! question. v1 embeds one vector per page (title + body text) via the embed
//! cell and ranks by cosine similarity; chunking by heading is a future refinement.

use facet::Facet;

use crate::db::SiteTree;

/// One search hit: a page and how closely it matches the query.
#[derive(Debug, Clone, Facet)]
pub struct KnowledgeHit {
    pub route: String,
    pub title: String,
    pub snippet: String,
    /// Cosine similarity in `[-1, 1]` (vectors are unit-normalized).
    pub score: f64,
}

/// Response body for `/_dodeca/knowledge/search` (serialized with facet-json).
#[derive(Debug, Clone, Facet)]
pub struct KnowledgeResponse {
    pub query: String,
    pub hits: Vec<KnowledgeHit>,
}

/// Embed `query` and the body of every page, then return the `k` pages whose
/// embedding is most similar. Returns an empty hit list if the query is blank or
/// the embedder fails (the caller still gets a well-formed response).
pub async fn search(tree: &SiteTree, query: &str, k: usize) -> KnowledgeResponse {
    let query = query.trim().to_string();
    let pages: Vec<&crate::db::Page> = tree.pages.values().collect();
    if query.is_empty() || pages.is_empty() {
        return KnowledgeResponse {
            query,
            hits: Vec::new(),
        };
    }

    // texts[0] is the query; the rest line up with `pages`.
    let mut texts = Vec::with_capacity(pages.len() + 1);
    texts.push(query.clone());
    for page in &pages {
        let body = plain_text(page.body_html.as_str());
        texts.push(format!("{}\n{}", page.title.as_str(), body));
    }

    let cell_embed_proto::EmbedResult::Success { vectors, .. } = crate::cells::embed(texts).await
    else {
        return KnowledgeResponse {
            query,
            hits: Vec::new(),
        };
    };
    let Some((query_vec, page_vecs)) = vectors.split_first() else {
        return KnowledgeResponse {
            query,
            hits: Vec::new(),
        };
    };

    let mut hits: Vec<KnowledgeHit> = pages
        .iter()
        .zip(page_vecs)
        .map(|(page, vec)| KnowledgeHit {
            route: page.route.as_str().to_string(),
            title: page.title.as_str().to_string(),
            snippet: snippet(page.body_html.as_str()),
            score: dot(query_vec, vec) as f64,
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(k);

    KnowledgeResponse { query, hits }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Strip HTML tags from rendered body HTML and collapse whitespace, for
/// embedding. Truncated generously — enough to characterize the page without
/// embedding a whole book.
fn plain_text(html: &str) -> String {
    const MAX: usize = 4000;
    let mut out = String::new();
    let mut in_tag = false;
    let mut prev_ws = false;
    for ch in html.chars() {
        match ch {
            // A tag boundary separates words (so "world</p><pre>x" → "world x",
            // not "worldx"); emit a collapsing space.
            '<' => {
                in_tag = true;
                if !prev_ws {
                    out.push(' ');
                    prev_ws = true;
                }
            }
            '>' => in_tag = false,
            _ if in_tag => {}
            c if c.is_whitespace() => {
                if !prev_ws {
                    out.push(' ');
                    prev_ws = true;
                }
            }
            c => {
                out.push(c);
                prev_ws = false;
                if out.len() >= MAX {
                    break;
                }
            }
        }
    }
    out.trim().to_string()
}

/// A short human-readable preview of the page for the search result.
fn snippet(html: &str) -> String {
    let text = plain_text(html);
    let mut s: String = text.chars().take(180).collect();
    if text.chars().count() > 180 {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_strips_tags_and_collapses_ws() {
        let html = "<p>Hello   <strong>there</strong>\n  world</p><pre>x</pre>";
        assert_eq!(plain_text(html), "Hello there world x");
    }

    #[test]
    fn snippet_is_bounded() {
        let html = format!("<p>{}</p>", "word ".repeat(100));
        assert!(snippet(&html).chars().count() <= 181);
        assert!(snippet(&html).ends_with('…'));
    }
}
