//! Site-wide annotation index derived from inline `<!-- note ... -->` comments.

use std::collections::HashMap;

use facet::Facet;
use picante::PicanteResult;

use crate::db::{Db, SourceRegistry};
use crate::queries::{build_tree, default_title_from_source_path};

#[derive(Debug, Clone, Default, Facet)]
pub struct AnnotationIndex {
    pub total: u32,
    pub open: u32,
    pub resolved: u32,
    pub threads: Vec<AnnotationThread>,
}

#[derive(Debug, Clone, Default, Facet)]
pub struct AnnotationThread {
    pub id: String,
    pub route: String,
    pub title: String,
    pub source_file: String,
    pub line: u32,
    pub quote: String,
    pub resolved: bool,
    pub kind: String,
    pub author: String,
    pub created: String,
    pub comments: Vec<AnnotationComment>,
}

#[derive(Debug, Clone, Default, Facet)]
pub struct AnnotationComment {
    pub author: String,
    pub kind: String,
    pub created: String,
    pub body: String,
    pub line: u32,
    pub nonce: String,
}

#[derive(Debug, Clone)]
struct RouteMeta {
    route: String,
    title: String,
}

pub async fn index<DB: Db>(db: &DB) -> PicanteResult<AnnotationIndex> {
    let sources = SourceRegistry::sources(db)?.unwrap_or_default();
    let route_meta = route_meta_by_source(db).await?;
    let mut by_id: HashMap<String, AnnotationThread> = HashMap::new();

    for source in sources.iter() {
        let path = source.path(db)?;
        let source_file = path.as_str().to_string();
        let content = source.content(db)?;
        let fallback = RouteMeta {
            route: path.to_route().as_str().to_string(),
            title: default_title_from_source_path(path.as_str()),
        };
        let meta = route_meta.get(&source_file).unwrap_or(&fallback);

        for block in note_blocks(content.as_str()) {
            let id = block
                .note
                .meta
                .id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{source_file}:{}", block.line));
            let comment = AnnotationComment {
                author: block.note.meta.author.clone().unwrap_or_default(),
                kind: block
                    .note
                    .meta
                    .kind
                    .clone()
                    .unwrap_or_else(|| "note".to_string()),
                created: block.note.meta.created.clone().unwrap_or_default(),
                body: block.note.body.clone(),
                line: block.line,
                nonce: block.note.meta.nonce.clone().unwrap_or_default(),
            };

            let thread = by_id.entry(id.clone()).or_insert_with(|| AnnotationThread {
                id,
                route: meta.route.clone(),
                title: meta.title.clone(),
                source_file: source_file.clone(),
                line: block.line,
                quote: block.note.meta.quote.clone().unwrap_or_default(),
                resolved: false,
                kind: comment.kind.clone(),
                author: comment.author.clone(),
                created: comment.created.clone(),
                comments: Vec::new(),
            });

            if thread.quote.is_empty() {
                thread.quote = block.note.meta.quote.clone().unwrap_or_default();
            }
            thread.resolved |= block.note.meta.resolved == Some(true);
            thread.comments.push(comment);
        }
    }

    let mut threads: Vec<_> = by_id.into_values().collect();
    threads.sort_by(|a, b| {
        a.route
            .cmp(&b.route)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.id.cmp(&b.id))
    });
    for thread in &mut threads {
        thread.comments.sort_by_key(|comment| comment.line);
    }
    let resolved = threads.iter().filter(|thread| thread.resolved).count() as u32;
    let total = threads.len() as u32;
    Ok(AnnotationIndex {
        total,
        open: total.saturating_sub(resolved),
        resolved,
        threads,
    })
}

pub async fn render_html(index: &AnnotationIndex) -> String {
    let threads = render_threads_html(index).await;
    format!(
        "<!doctype html><html><head><meta charset=utf-8>\
<meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>dodeca annotations</title>\
<style>\
:root{{--bg:#fff;--panel:#f8fafc;--text:#111827;--muted:#6b7280;--border:#d1d5db;--accent:#2563eb;--ok:#0f766e;--bad:#b91c1c}}\
@media(prefers-color-scheme:dark){{:root{{--bg:#10131a;--panel:#171b24;--text:#e5e7eb;--muted:#9ca3af;--border:#374151;--accent:#60a5fa;--ok:#34d399;--bad:#f87171}}}}\
*{{box-sizing:border-box}}body{{font:14px/1.5 system-ui,sans-serif;max-width:76rem;margin:0 auto;padding:24px;color:var(--text);background:var(--bg)}}\
a{{color:var(--accent);text-decoration:none}}a:hover{{text-decoration:underline}}\
header{{display:flex;align-items:flex-start;justify-content:space-between;gap:16px;margin-bottom:18px}}\
h1{{font-size:1.35rem;line-height:1.2;margin:0}}h2{{font-size:.95rem;margin:0 0 6px}}\
.meta,.muted{{color:var(--muted)}}.nav{{display:flex;gap:10px;flex-wrap:wrap}}\
.nav a,.pill{{border:1px solid var(--border);border-radius:999px;padding:4px 9px;background:var(--panel);color:var(--text)}}\
.counts{{display:flex;gap:8px;flex-wrap:wrap;margin:10px 0 18px}}\
.toolbar{{display:flex;gap:10px;align-items:center;flex-wrap:wrap;margin:0 0 14px}}\
input[type=search]{{flex:1;min-width:220px;border:1px solid var(--border);border-radius:6px;background:var(--bg);color:var(--text);padding:7px 9px;font:inherit}}\
label{{display:inline-flex;gap:6px;align-items:center;color:var(--muted)}}\
.thread{{border:1px solid var(--border);border-left:3px solid var(--accent);border-radius:8px;margin:10px 0;background:var(--bg);overflow:hidden}}\
.thread[data-resolved=true]{{opacity:.72}}.thread-head{{display:flex;gap:10px;justify-content:space-between;padding:10px 12px;background:var(--panel);border-bottom:1px solid var(--border)}}\
.thread-title{{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}}.thread-title strong{{font-size:.98rem}}\
.tag{{font-size:.72rem;text-transform:uppercase;font-weight:700;color:var(--accent)}}.resolved{{color:var(--ok)}}\
.thread-body{{padding:10px 12px}}.quote{{margin:0 0 9px;color:var(--muted);border-left:3px solid var(--accent);padding-left:8px}}\
.comment{{padding:8px 0;border-top:1px solid var(--border)}}.comment:first-of-type{{border-top:0;padding-top:0}}\
.comment-meta{{display:flex;gap:8px;flex-wrap:wrap;color:var(--muted);font-size:.78rem;margin-bottom:4px}}\
.comment-body>:first-child{{margin-top:0}}.comment-body>:last-child{{margin-bottom:0}}\
code{{background:var(--panel);border:1px solid var(--border);border-radius:4px;padding:0 4px}}\
.empty{{padding:22px;border:1px solid var(--border);border-radius:8px;background:var(--panel);color:var(--muted)}}\
@media(max-width:700px){{body{{padding:16px}}header,.thread-head{{display:block}}.nav{{margin-top:10px}}}}\
</style></head><body>\
<header><div><h1>dodeca annotations</h1><p class=meta>{} open · {} resolved · {} total</p></div>\
<nav class=nav><a href=\"/_dodeca/status\">status</a><a href=\"/_dodeca/annotations.json\">json</a><a href=\"/_dodeca/annotations.md\">markdown</a></nav></header>\
<section class=counts><span class=pill>{} open</span><span class=pill>{} resolved</span><span class=pill>{} total</span></section>\
<section class=toolbar><input id=q type=search placeholder=\"Filter annotations\"><label><input id=resolved type=checkbox> resolved</label></section>\
<main id=list>{threads}</main>\
<script>\
const q=document.querySelector('#q'),r=document.querySelector('#resolved'),items=[...document.querySelectorAll('.thread')];\
function f(){{const s=q.value.toLowerCase(),show=r.checked;for(const item of items){{const ok=(!s||item.dataset.search.includes(s))&&(show||item.dataset.resolved!=='true');item.hidden=!ok;}}}}\
q.addEventListener('input',f);r.addEventListener('change',f);f();\
</script></body></html>",
        index.open, index.resolved, index.total, index.open, index.resolved, index.total
    )
}

pub fn render_markdown(index: &AnnotationIndex) -> String {
    let mut out = format!(
        "# dodeca annotations\n\n{} open, {} resolved, {} total.\n\n",
        index.open, index.resolved, index.total
    );
    if index.threads.is_empty() {
        out.push_str("No annotations.\n");
        return out;
    }
    for thread in &index.threads {
        let state = if thread.resolved { "resolved" } else { "open" };
        out.push_str(&format!(
            "## [{}]({}) - {} ({})\n\n",
            thread.title, thread.route, thread.kind, state
        ));
        out.push_str(&format!(
            "- id: `{}`\n- source: `{}:{}`\n",
            thread.id, thread.source_file, thread.line
        ));
        if !thread.quote.is_empty() {
            out.push_str(&format!("- quote: `{}`\n", thread.quote));
        }
        out.push('\n');
        for comment in &thread.comments {
            out.push_str(&format!(
                "### {}{}{}\n\n{}\n\n",
                if comment.author.is_empty() {
                    "anon"
                } else {
                    &comment.author
                },
                if comment.created.is_empty() {
                    ""
                } else {
                    " - "
                },
                comment.created,
                comment.body
            ));
        }
    }
    out
}

async fn render_threads_html(index: &AnnotationIndex) -> String {
    if index.threads.is_empty() {
        return "<div class=empty>No annotations.</div>".to_string();
    }

    let mut out = String::new();
    for thread in &index.threads {
        let mut comments = String::new();
        for comment in &thread.comments {
            let author = if comment.author.is_empty() {
                "anon"
            } else {
                &comment.author
            };
            let body = render_comment_body(&comment.body).await;
            comments.push_str(&format!(
                "<section class=comment><div class=comment-meta><b>{}</b><span>{}</span><span>{}</span><span>line {}</span></div><div class=comment-body>{}</div></section>",
                html_escape(author),
                html_escape(&comment.kind),
                html_escape(&comment.created),
                comment.line,
                body
            ));
        }

        let status = if thread.resolved {
            "<span class=\"resolved\">resolved</span>"
        } else {
            "<span>open</span>"
        };
        let quote = if thread.quote.is_empty() {
            String::new()
        } else {
            format!(
                "<blockquote class=quote>{}</blockquote>",
                html_escape(&thread.quote)
            )
        };
        let search = thread_search_text(thread);
        out.push_str(&format!(
            "<article class=thread data-resolved=\"{}\" data-search=\"{}\">\
<div class=thread-head><div class=thread-title><strong><a href=\"{}\">{}</a></strong><span class=tag>{}</span>{}</div>\
<div class=muted><code>{}:{}</code> · <a href=\"{}\">edit</a></div></div>\
<div class=thread-body>{quote}{comments}</div></article>",
            thread.resolved,
            attr_escape(&search),
            attr_escape(&thread.route),
            html_escape(&thread.title),
            html_escape(&thread.kind),
            status,
            html_escape(&thread.source_file),
            thread.line,
            attr_escape(&edit_href(&thread.route)),
        ));
    }
    out
}

async fn render_comment_body(body: &str) -> String {
    let options = marq::RenderOptions::new();
    match marq::render(body, &options).await {
        Ok(doc) => doc.html,
        Err(_) => format!("<pre>{}</pre>", html_escape(body)),
    }
}

fn thread_search_text(thread: &AnnotationThread) -> String {
    let mut search = format!(
        "{} {} {} {} {} {}",
        thread.id, thread.route, thread.title, thread.source_file, thread.kind, thread.quote
    );
    for comment in &thread.comments {
        search.push(' ');
        search.push_str(&comment.author);
        search.push(' ');
        search.push_str(&comment.kind);
        search.push(' ');
        search.push_str(&comment.body);
    }
    search.to_lowercase()
}

fn edit_href(route: &str) -> String {
    format!("/_dodeca/edit/{}", route.trim_start_matches('/'))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn attr_escape(s: &str) -> String {
    html_escape(s).replace('"', "&quot;").replace('\'', "&#39;")
}

async fn route_meta_by_source<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, RouteMeta>> {
    let mut out = HashMap::new();
    let Ok(tree) = build_tree(db).await? else {
        return Ok(out);
    };

    for section in tree.sections.values() {
        if let Some(source_file) = section.source_map.source_path.as_ref() {
            out.insert(
                source_file.clone(),
                RouteMeta {
                    route: section.route.as_str().to_string(),
                    title: section.title.as_str().to_string(),
                },
            );
        }
    }
    for page in tree.pages.values() {
        if let Some(source_file) = page.source_map.source_path.as_ref() {
            out.insert(
                source_file.clone(),
                RouteMeta {
                    route: page.route.as_str().to_string(),
                    title: page.title.as_str().to_string(),
                },
            );
        }
    }

    Ok(out)
}

struct NoteBlock {
    note: marq::Note,
    line: u32,
}

fn note_blocks(content: &str) -> Vec<NoteBlock> {
    let mut blocks = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = content[from..].find("<!--") {
        let start = from + rel;
        let Some(close_rel) = content[start..].find("-->") else {
            break;
        };
        let end = start + close_rel + 3;
        if let Some(note) = marq::parse_note(&content[start..end]) {
            blocks.push(NoteBlock {
                note,
                line: content[..start].matches('\n').count() as u32 + 1,
            });
        }
        from = end;
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_blocks_finds_notes_and_lines() {
        let content =
            "# T\n\nPara.\n\n<!-- note\n+++\nid = \"a\"\n+++\nfirst\n-->\n\n<!-- nope -->\n";
        let blocks = note_blocks(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].line, 5);
        assert_eq!(blocks[0].note.meta.id.as_deref(), Some("a"));
    }
}
