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
