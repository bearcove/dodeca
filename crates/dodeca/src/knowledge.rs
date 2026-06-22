//! Semantic knowledge API: chunk pages by heading, embed them (cached per
//! source in picante), and answer nearest-neighbor queries.
//!
//! Backs the dev-only well-known endpoints `/_dodeca/knowledge/search` and
//! `/_dodeca/knowledge/related`, so an agent can `curl` the knowledge base and
//! get the most relevant *sections* for a question. Pages are split into
//! heading-level chunks (each with its anchor), embedded via the embed cell, and
//! ranked by cosine similarity. [`page_chunks_embedded`] is a tracked query, so
//! a chunk is only re-embedded when its source file changes.

use std::collections::HashSet;

use facet::Facet;
use hotmeal::{NodeId, NodeKind, StrTendril, parse_body_fragment};
use picante::PicanteResult;

use crate::db::{Db, SourceFile};
use crate::queries::parse_file;

/// A unit-normalized embedding vector. Its `Eq`/`PartialEq` compare the raw bits
/// (`f32::to_bits`) — the right *cache* semantics, and what lets it flow through
/// picante's by-value memoization (plain `Vec<f32>` can't be `Eq`).
#[derive(Debug, Clone, Facet)]
pub struct Embedding {
    pub values: Vec<f32>,
}

impl PartialEq for Embedding {
    fn eq(&self, other: &Self) -> bool {
        self.values.len() == other.values.len()
            && std::iter::zip(&self.values, &other.values).all(|(a, b)| a.to_bits() == b.to_bits())
    }
}
impl Eq for Embedding {}

impl Embedding {
    fn dot(&self, other: &[f32]) -> f32 {
        std::iter::zip(&self.values, other)
            .map(|(a, b)| a * b)
            .sum()
    }
}

/// One embedded, heading-delimited section of a page — the unit we cache, rank,
/// and return.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct EmbeddedChunk {
    pub route: String,
    /// Heading id to deep-link to (`route#anchor`); empty for the page intro.
    pub anchor: String,
    pub title: String,
    /// Nearest heading (the page title for the intro chunk).
    pub heading: String,
    /// Short display preview of the section.
    pub snippet: String,
    pub embedding: Embedding,
}

/// A node in a page's connection graph.
#[derive(Debug, Clone, Facet)]
pub struct GraphNode {
    pub route: String,
    pub title: String,
}

/// A page's neighborhood: explicit links out, pages linking in, and semantically
/// related pages. Backs `/_dodeca/knowledge/graph`.
#[derive(Debug, Clone, Facet)]
pub struct GraphResponse {
    pub center: GraphNode,
    /// Pages this one links to.
    pub outbound: Vec<GraphNode>,
    /// Pages that link to this one.
    pub backlinks: Vec<GraphNode>,
    /// Semantically nearest pages (vector similarity).
    pub related: Vec<KnowledgeHit>,
}

/// One search hit (the JSON shape returned to the agent).
#[derive(Debug, Clone, Facet)]
pub struct KnowledgeHit {
    pub route: String,
    pub anchor: String,
    pub title: String,
    pub heading: String,
    pub snippet: String,
    /// Cosine similarity in `[-1, 1]` (vectors are unit-normalized).
    pub score: f64,
}

/// Response body for `/_dodeca/knowledge/*` (serialized with facet-json).
#[derive(Debug, Clone, Facet)]
pub struct KnowledgeResponse {
    pub query: String,
    pub hits: Vec<KnowledgeHit>,
}

/// Chunk one source's page by heading and embed each chunk. Tracked: re-runs
/// only when this source file changes, so unchanged pages keep their vectors.
#[picante::tracked]
pub async fn page_chunks_embedded<DB: Db>(
    db: &DB,
    source: SourceFile,
) -> PicanteResult<Vec<EmbeddedChunk>> {
    let parsed = match parse_file(db, source).await? {
        Ok(parsed) => parsed,
        Err(_) => return Ok(Vec::new()),
    };

    let mut raw = Vec::new();
    chunk_page(
        parsed.route.as_str(),
        parsed.title.as_str(),
        parsed.body_html.as_str(),
        &mut raw,
    );
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let texts: Vec<String> = raw.iter().map(RawChunk::embed_text).collect();
    let vectors = match crate::cells::embed(texts).await {
        cell_embed_proto::EmbedResult::Success { vectors, .. } => vectors,
        cell_embed_proto::EmbedResult::Error { .. } => return Ok(Vec::new()),
    };

    Ok(std::iter::zip(raw, vectors)
        .map(|(c, values)| EmbeddedChunk {
            route: c.route,
            anchor: c.anchor,
            title: c.title,
            heading: c.heading,
            snippet: snippet(&c.text),
            embedding: Embedding { values },
        })
        .collect())
}

/// Gather every source's cached chunks into one flat list.
async fn all_chunks<DB: Db>(db: &DB) -> PicanteResult<Vec<EmbeddedChunk>> {
    let sources = crate::db::SourceRegistry::sources(db)?.unwrap_or_default();
    let mut all = Vec::new();
    for source in sources.iter() {
        all.extend(page_chunks_embedded(db, *source).await?);
    }
    Ok(all)
}

/// Embed `query` and return the `k` most similar chunks across the whole site.
pub async fn search<DB: Db>(db: &DB, query: &str, k: usize) -> KnowledgeResponse {
    let query = query.trim().to_string();
    let chunks = all_chunks(db).await.unwrap_or_default();
    if query.is_empty() || chunks.is_empty() {
        return KnowledgeResponse {
            query,
            hits: Vec::new(),
        };
    }
    let Some(query_vec) = embed_query(&query).await else {
        return KnowledgeResponse {
            query,
            hits: Vec::new(),
        };
    };
    KnowledgeResponse {
        query,
        hits: top_k(&chunks, &query_vec, k, None),
    }
}

/// Pages most related to `route`: the chunks from *other* pages nearest to the
/// mean of `route`'s own chunk vectors.
pub async fn related<DB: Db>(db: &DB, route: &str, k: usize) -> KnowledgeResponse {
    let query = format!("related:{route}");
    let chunks = all_chunks(db).await.unwrap_or_default();
    let dim = chunks
        .iter()
        .find(|c| c.route == route)
        .map_or(0, |c| c.embedding.values.len());
    if dim == 0 {
        return KnowledgeResponse {
            query,
            hits: Vec::new(),
        };
    }
    let mut centroid = vec![0f32; dim];
    let mut n = 0usize;
    for c in chunks.iter().filter(|c| c.route == route) {
        for (acc, v) in std::iter::zip(&mut centroid, &c.embedding.values) {
            *acc += v;
        }
        n += 1;
    }
    for acc in &mut centroid {
        *acc /= n as f32;
    }
    KnowledgeResponse {
        query,
        hits: top_k(&chunks, &centroid, k, Some(route)),
    }
}

/// A page's connection graph: explicit links out, pages linking in (from the
/// site-wide rendered links), and semantically related pages. Returns `None`
/// when `route` isn't a real page/section.
pub async fn graph<DB: Db>(db: &DB, route: &str, k: usize) -> Option<GraphResponse> {
    let tree = match crate::queries::build_tree(db).await {
        Ok(Ok(tree)) => tree,
        _ => return None,
    };
    let center = {
        let trimmed = route.trim_end_matches('/');
        if trimmed.is_empty() {
            "/".to_string()
        } else {
            trimmed.to_string()
        }
    };
    let routes: HashSet<String> = tree
        .pages
        .keys()
        .chain(tree.sections.keys())
        .map(|r| r.as_str().to_string())
        .collect();
    if !routes.contains(&center) {
        return None;
    }
    let title_of = |r: &str| -> String {
        let route = crate::types::Route::from(r);
        tree.pages
            .get(&route)
            .map(|p| p.title.as_str().to_string())
            .or_else(|| {
                tree.sections
                    .get(&route)
                    .map(|s| s.title.as_str().to_string())
            })
            .unwrap_or_else(|| r.to_string())
    };

    // Walk every page's links from its cached body HTML (the rendered body still
    // carries auto-link / plain `/route` hrefs, plus `data-wiki-target` for
    // wikilinks which we resolve context-first via the #8 resolver). This avoids
    // a full re-render and works straight off the site tree.
    let resolver = crate::wikilink::Resolver::new(
        routes.iter().cloned(),
        tree.sections.keys().map(|r| r.as_str().to_string()),
        crate::config::global_config()
            .map(|c| {
                c.sources
                    .iter()
                    .map(|s| (s.name.clone(), s.mount.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    let bodies: Vec<(String, String)> = tree
        .pages
        .values()
        .map(|p| {
            (
                p.route.as_str().to_string(),
                p.body_html.as_str().to_string(),
            )
        })
        .chain(tree.sections.values().map(|s| {
            (
                s.route.as_str().to_string(),
                s.body_html.as_str().to_string(),
            )
        }))
        .collect();

    let mut outbound: Vec<String> = Vec::new();
    let mut backlinks: Vec<String> = Vec::new();
    for (source, body) in &bodies {
        let mut targets: Vec<String> = Vec::new();
        for href in scan_attr(body, "href") {
            if let Some(t) = internal_route(&href, &routes) {
                targets.push(t);
            }
        }
        for raw in scan_attr(body, "data-wiki-target") {
            if let crate::wikilink::Resolution::Resolved(t) = resolver.resolve(source, &raw) {
                targets.push(t);
            }
        }
        for target in targets {
            if &target == source {
                continue;
            }
            if source == &center {
                outbound.push(target);
            } else if target == center {
                backlinks.push(source.clone());
            }
        }
    }
    outbound.sort();
    outbound.dedup();
    backlinks.sort();
    backlinks.dedup();

    let related = related(db, &center, k).await.hits;
    Some(GraphResponse {
        center: GraphNode {
            route: center.clone(),
            title: title_of(&center),
        },
        outbound: outbound
            .iter()
            .map(|r| GraphNode {
                route: r.clone(),
                title: title_of(r),
            })
            .collect(),
        backlinks: backlinks
            .iter()
            .map(|r| GraphNode {
                route: r.clone(),
                title: title_of(r),
            })
            .collect(),
        related,
    })
}

/// A self-contained interactive SVG view of a page's connection graph. Fetches
/// `?format=json` for the given route and renders a radial node-link diagram;
/// clicking a node re-centers the graph, the ⇱ link opens the page.
pub fn graph_view_html(route: &str) -> String {
    let route_json = format!("\"{}\"", route.replace('\\', "\\\\").replace('"', "\\\""));
    format!(
        r##"<!doctype html><html><head><meta charset="utf-8">
<title>Knowledge graph</title>
<style>
  html,body{{margin:0;height:100%;background:#11111b;color:#cdd6f4;font:14px system-ui,sans-serif}}
  #t{{position:fixed;top:10px;left:14px;font-size:13px;opacity:.85}}
  svg{{width:100vw;height:100vh;display:block}}
  .lnk{{stroke:#45475a;stroke-width:1}}
  .nd circle{{cursor:pointer}}
  .nd text{{fill:#cdd6f4;font-size:12px;pointer-events:none}}
  .center circle{{fill:#f9e2af}}
  .out circle{{fill:#89b4fa}} .back circle{{fill:#a6e3a1}} .rel circle{{fill:#f38ba8}}
  a.visit{{fill:#89b4fa;text-decoration:none}}
  #legend{{position:fixed;bottom:12px;left:14px;font-size:12px;opacity:.85;line-height:1.6}}
  #legend b{{font-weight:600}}
  .sw{{display:inline-block;width:10px;height:10px;border-radius:50%;margin-right:5px;vertical-align:middle}}
</style></head><body>
<div id="t">…</div><svg id="g"></svg>
<div id="legend">
  <div><span class="sw" style="background:#f9e2af"></span>this page</div>
  <div><span class="sw" style="background:#89b4fa"></span>links out</div>
  <div><span class="sw" style="background:#a6e3a1"></span>links in (backlinks)</div>
  <div><span class="sw" style="background:#f38ba8"></span>related (semantic)</div>
</div>
<script>
const ROUTE={route_json};
const SVG="http://www.w3.org/2000/svg";
function el(n,a){{const e=document.createElementNS(SVG,n);for(const k in a)e.setAttribute(k,a[k]);return e;}}
async function draw(route){{
  const r=await fetch(`/_dodeca/knowledge/graph?route=${{encodeURIComponent(route)}}&format=json`).then(r=>r.json());
  if(!r.center){{document.getElementById('t').textContent='no graph for '+route;return;}}
  document.getElementById('t').innerHTML=`<b>${{r.center.title}}</b> <span style="opacity:.6">${{r.center.route}}</span>`;
  const g=document.getElementById('g');g.innerHTML='';
  const W=g.clientWidth,H=g.clientHeight,cx=W/2,cy=H/2;
  const groups=[['out',r.outbound],['back',r.backlinks],['rel',(r.related||[]).map(h=>({{route:h.route,title:h.title}}))]];
  const seen=new Set([r.center.route]);
  const nodes=[];
  for(const [cls,arr] of groups) for(const n of arr){{ if(seen.has(n.route))continue; seen.add(n.route); nodes.push({{cls,...n}}); }}
  const R=Math.min(W,H)/2-90;
  nodes.forEach((n,i)=>{{const a=-Math.PI/2+2*Math.PI*i/Math.max(nodes.length,1);n.x=cx+R*Math.cos(a);n.y=cy+R*Math.sin(a);}});
  for(const n of nodes) g.appendChild(el('line',{{class:'lnk',x1:cx,y1:cy,x2:n.x,y2:n.y}}));
  function node(n,cls,big){{
    const grp=el('g',{{class:'nd '+cls}});
    const c=el('circle',{{cx:n.x,cy:n.y,r:big?13:8}});
    c.addEventListener('click',()=>draw(n.route));
    grp.appendChild(c);
    const t=el('text',{{x:n.x+(n.x<cx?-12:12),y:n.y+4,'text-anchor':n.x<cx?'end':'start'}});
    t.textContent=(n.title||n.route).slice(0,32);
    grp.appendChild(t);
    g.appendChild(grp);
  }}
  for(const n of nodes) node(n,n.cls,false);
  r.center.x=cx;r.center.y=cy;
  node(r.center,'center',true);
}}
draw(ROUTE);
</script></body></html>"##
    )
}

/// Extract every value of the `attr="…"` attribute in `html` (HTML-unescaped) —
/// a cheap scan over the cached body, enough to read links and wiki targets.
fn scan_attr(html: &str, attr: &str) -> Vec<String> {
    let needle = format!("{attr}=\"");
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(&needle) {
        rest = &rest[i + needle.len()..];
        let Some(end) = rest.find('"') else { break };
        out.push(
            rest[..end]
                .replace("&amp;", "&")
                .replace("&quot;", "\"")
                .replace("&lt;", "<")
                .replace("&gt;", ">"),
        );
        rest = &rest[end + 1..];
    }
    out
}

/// Normalize an internal href to a known route, or `None` for external /
/// fragment-only / unknown links.
fn internal_route(href: &str, routes: &HashSet<String>) -> Option<String> {
    if !href.starts_with('/') || href.starts_with("//") {
        return None;
    }
    let path = href.split(['?', '#']).next().unwrap_or(href);
    let trimmed = path.trim_end_matches('/');
    let candidate = if trimmed.is_empty() { "/" } else { trimmed };
    routes.contains(candidate).then(|| candidate.to_string())
}

/// Rank `chunks` against `query_vec` and take the top `k`, optionally skipping a
/// route.
fn top_k(
    chunks: &[EmbeddedChunk],
    query_vec: &[f32],
    k: usize,
    exclude_route: Option<&str>,
) -> Vec<KnowledgeHit> {
    let mut scored: Vec<(f32, &EmbeddedChunk)> = chunks
        .iter()
        .filter(|c| exclude_route != Some(c.route.as_str()))
        .map(|c| (c.embedding.dot(query_vec), c))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.truncate(k);
    scored
        .into_iter()
        .map(|(score, c)| KnowledgeHit {
            route: c.route.clone(),
            anchor: c.anchor.clone(),
            title: c.title.clone(),
            heading: c.heading.clone(),
            snippet: c.snippet.clone(),
            score: score as f64,
        })
        .collect()
}

async fn embed_query(query: &str) -> Option<Vec<f32>> {
    match crate::cells::embed(vec![query.to_string()]).await {
        cell_embed_proto::EmbedResult::Success { mut vectors, .. } => vectors.drain(..).next(),
        cell_embed_proto::EmbedResult::Error { .. } => None,
    }
}

/// A page section before embedding.
struct RawChunk {
    route: String,
    anchor: String,
    title: String,
    heading: String,
    text: String,
}

impl RawChunk {
    /// Text fed to the embedder: page + heading context, then the body.
    fn embed_text(&self) -> String {
        format!("{} / {}\n{}", self.title, self.heading, self.text)
    }
}

/// Split a page's rendered body into heading-delimited chunks (appended to
/// `out`). Content before the first heading is the page "intro" chunk; each
/// heading starts a new chunk anchored at its id.
fn chunk_page(route: &str, title: &str, body_html: &str, out: &mut Vec<RawChunk>) {
    let tendril = StrTendril::from(body_html);
    let doc = parse_body_fragment(&tendril);

    let mut current = RawChunk {
        route: route.to_string(),
        anchor: String::new(),
        title: title.to_string(),
        heading: title.to_string(),
        text: String::new(),
    };
    let flush = |out: &mut Vec<RawChunk>, c: &RawChunk| {
        if !c.text.trim().is_empty() {
            out.push(RawChunk {
                route: c.route.clone(),
                anchor: c.anchor.clone(),
                title: c.title.clone(),
                heading: c.heading.clone(),
                text: c.text.trim().to_string(),
            });
        }
    };

    for child in doc.children(doc.root).collect::<Vec<_>>() {
        if let NodeKind::Element(el) = &doc.get(child).kind {
            if matches!(el.tag.as_ref(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                flush(out, &current);
                let anchor = el
                    .attrs
                    .iter()
                    .find(|(n, _)| n.local.as_ref() == "id")
                    .map(|(_, v)| v.as_ref().to_string())
                    .unwrap_or_default();
                let mut heading = String::new();
                collect_text(&doc, child, &mut heading);
                current = RawChunk {
                    route: route.to_string(),
                    anchor,
                    title: title.to_string(),
                    heading: heading.trim().to_string(),
                    text: String::new(),
                };
                continue;
            }
        }
        collect_text(&doc, child, &mut current.text);
    }
    flush(out, &current);
}

/// Append the visible text of `id`'s subtree to `out`, collapsing whitespace.
fn collect_text(doc: &hotmeal::Document<'_>, id: NodeId, out: &mut String) {
    match &doc.get(id).kind {
        NodeKind::Text(t) => {
            for ch in t.as_ref().chars() {
                if ch.is_whitespace() {
                    if !out.ends_with(' ') {
                        out.push(' ');
                    }
                } else {
                    out.push(ch);
                }
            }
        }
        _ => {
            for child in doc.children(id).collect::<Vec<_>>() {
                collect_text(doc, child, out);
            }
        }
    }
}

/// A short human-readable preview for a search result.
fn snippet(text: &str) -> String {
    let text = text.trim();
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
    fn embedding_eq_is_bitwise() {
        let a = Embedding {
            values: vec![0.1, 0.2],
        };
        let b = Embedding {
            values: vec![0.1, 0.2],
        };
        let c = Embedding {
            values: vec![0.1, 0.3],
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn chunks_split_at_headings_with_anchors() {
        let html = "<p>Intro text here.</p>\
                    <h2 id=\"setup\">Setup</h2><p>Install the thing.</p>\
                    <h2 id=\"usage\">Usage</h2><p>Run the thing.</p>";
        let mut out = Vec::new();
        chunk_page("/guide", "Guide", html, &mut out);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].anchor, "");
        assert!(out[0].text.contains("Intro text"));
        assert_eq!(out[1].anchor, "setup");
        assert_eq!(out[1].heading, "Setup");
        assert!(out[1].text.contains("Install the thing"));
        assert_eq!(out[2].anchor, "usage");
        assert!(out[2].text.contains("Run the thing"));
    }

    #[test]
    fn snippet_is_bounded() {
        let long = "word ".repeat(100);
        assert!(snippet(&long).chars().count() <= 181);
        assert!(snippet(&long).ends_with('…'));
    }
}
