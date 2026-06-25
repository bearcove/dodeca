//! Content-graph analysis over an [`AuthoringProject`]: which page links to
//! which, resolved to routes. This is the LSP-type-free analysis layer (line/
//! column, not tower_lsp ranges), lifted out of `dodeca-authoring-lsp` so it can
//! back picante tracked queries. The editor crate re-exports these.

use std::collections::{HashMap, HashSet};

use camino::{Utf8Path, Utf8PathBuf};
use picante::PicanteResult;
use pulldown_cmark::{Event, Options, Parser, Tag};

use crate::authoring_model::{AuthoringPage, AuthoringProject, build_authoring_project_from_db};
pub use crate::authoring_model::{normalize_route, strip_query};
use crate::db::Db;

/// The memoized [`AuthoringProject`] for a db (overlaid snapshot or owned),
/// keyed on `content_dir`. This is the authoring analysis entry point: picante
/// memoizes it, so within an LSP revision repeated requests reuse it, and the
/// snapshot's inherited render cells keep unchanged pages free across edits.
/// The inner `Result` carries build failures (parse errors, missing files) as a
/// string, matching the `build_site` convention.
#[picante::tracked]
pub async fn authoring_project<DB: Db>(
    db: &DB,
    content_dir: Utf8PathBuf,
) -> PicanteResult<Result<AuthoringProject, String>> {
    Ok(build_authoring_project_from_db(db, &content_dir)
        .await
        .map_err(|e| e.to_string()))
}

/// The memoized content/route graph (which page links to which, resolved to
/// routes) over [`authoring_project`]. This is the "heavy query on every
/// keystroke" — now a tracked query that picante recomputes only when its
/// inputs change rather than a hand-rolled per-revision rebuild.
#[picante::tracked]
pub async fn content_graph<DB: Db>(
    db: &DB,
    content_dir: Utf8PathBuf,
) -> PicanteResult<Result<Vec<RouteGraphNode>, String>> {
    match authoring_project(db, content_dir).await? {
        Ok(project) => Ok(Ok(route_graph_for_project(&project))),
        Err(e) => Ok(Err(e)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct RouteGraphNode {
    pub route: String,
    pub source_file: String,
    pub title: String,
    pub incoming: Vec<RouteGraphEdge>,
    pub outgoing: Vec<RouteGraphEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct RouteGraphEdge {
    pub kind: RouteGraphEdgeKind,
    pub source_route: String,
    pub source_file: String,
    pub target_route: String,
    pub target: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub line_end: Option<u32>,
    pub column_end: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum RouteGraphEdgeKind {
    Markdown,
    RenderedHtml,
}

impl RouteGraphEdgeKind {
    pub fn label(self) -> &'static str {
        match self {
            RouteGraphEdgeKind::Markdown => "markdown",
            RouteGraphEdgeKind::RenderedHtml => "renderedHtml",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownReferenceKind {
    Link,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownReference {
    pub kind: MarkdownReferenceKind,
    pub target: String,
    pub byte_start: usize,
    pub byte_end: usize,
}

pub fn route_graph_for_project(project: &AuthoringProject) -> Vec<RouteGraphNode> {
    let mut outgoing_by_route: HashMap<String, Vec<RouteGraphEdge>> = HashMap::new();
    let mut incoming_by_route: HashMap<String, Vec<RouteGraphEdge>> = HashMap::new();
    let mut seen_edges: HashSet<(String, String, String)> = HashSet::new();

    for page in &project.pages {
        let Some(content) = project.source_contents.get(&page.source_file) else {
            continue;
        };
        for reference in markdown_references(content) {
            let Some(target_route) = reference_target_route(project, page, &reference) else {
                continue;
            };
            if !project.route_exists(&target_route) {
                continue;
            }
            let (line, column) = byte_to_line_column(content, reference.byte_start);
            let (line_end, column_end) = byte_to_line_column(content, reference.byte_end);
            let edge = RouteGraphEdge {
                kind: RouteGraphEdgeKind::Markdown,
                source_route: page.route.clone(),
                source_file: page.source_file.clone(),
                target_route: target_route.clone(),
                target: reference.target,
                line: Some(line),
                column: Some(column),
                line_end: Some(line_end),
                column_end: Some(column_end),
            };
            seen_edges.insert((
                edge.source_route.clone(),
                edge.target_route.clone(),
                edge.target.clone(),
            ));
            outgoing_by_route
                .entry(page.route.clone())
                .or_default()
                .push(edge.clone());
            incoming_by_route
                .entry(target_route)
                .or_default()
                .push(edge);
        }
    }

    for (source_route, hrefs) in &project.rendered_hrefs_by_route {
        let Some(source_page) = project.page_for_route(source_route) else {
            continue;
        };
        for href in hrefs {
            let Some(target_route) = rendered_href_target_route(project, source_page, &href.href)
            else {
                continue;
            };
            if !seen_edges.insert((
                source_route.clone(),
                target_route.clone(),
                href.href.clone(),
            )) {
                continue;
            }
            let edge = RouteGraphEdge {
                kind: RouteGraphEdgeKind::RenderedHtml,
                source_route: source_route.clone(),
                source_file: source_page.source_file.clone(),
                target_route: target_route.clone(),
                target: href.href.clone(),
                line: None,
                column: None,
                line_end: None,
                column_end: None,
            };
            outgoing_by_route
                .entry(source_route.clone())
                .or_default()
                .push(edge.clone());
            incoming_by_route
                .entry(target_route)
                .or_default()
                .push(edge);
        }
    }

    project
        .pages
        .iter()
        .map(|page| RouteGraphNode {
            route: page.route.clone(),
            source_file: page.source_file.clone(),
            title: page.title.clone(),
            incoming: incoming_by_route.remove(&page.route).unwrap_or_default(),
            outgoing: outgoing_by_route.remove(&page.route).unwrap_or_default(),
        })
        .collect()
}

pub fn rendered_href_target_route(
    project: &AuthoringProject,
    source_page: &AuthoringPage,
    href: &str,
) -> Option<String> {
    if is_special_target(href) {
        return None;
    }

    let (target_without_fragment, _) = split_fragment(href);
    if target_without_fragment.is_empty() || is_likely_static_file(target_without_fragment) {
        return None;
    }

    let target_route = route_for_link_target(project, source_page, target_without_fragment);
    project.route_exists(&target_route).then_some(target_route)
}

pub fn reference_target_route(
    project: &AuthoringProject,
    page: &AuthoringPage,
    reference: &MarkdownReference,
) -> Option<String> {
    let target = reference.target.as_str();
    if is_special_target(target) {
        return None;
    }

    let (target_without_fragment, _) = split_fragment(target);
    if let Some(source_target) = target_without_fragment.strip_prefix("@/") {
        return project.source_to_route.get(source_target).cloned();
    }

    if reference.kind == MarkdownReferenceKind::Image
        || is_likely_static_file(target_without_fragment)
    {
        return None;
    }

    Some(route_for_link_target(
        project,
        page,
        target_without_fragment,
    ))
}

pub fn route_for_link_target(
    project: &AuthoringProject,
    page: &AuthoringPage,
    target_without_fragment: &str,
) -> String {
    if target_without_fragment.is_empty() {
        return page.route.clone();
    }

    if let Some(source_target) =
        source_target_for_relative_markdown_link(page, target_without_fragment)
        && let Some(route) = project.source_to_route.get(&source_target)
    {
        return route.clone();
    }

    if target_without_fragment.starts_with('/') {
        normalize_route(target_without_fragment)
    } else {
        normalize_route(&format!(
            "{}{target_without_fragment}",
            ensure_trailing_slash(&page.link_base_route)
        ))
    }
}

pub fn source_target_for_relative_markdown_link(
    page: &AuthoringPage,
    target: &str,
) -> Option<String> {
    if !target.ends_with(".md") {
        return None;
    }
    let source_parent = Utf8Path::new(&page.source_file)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    Some(normalize_relative_path(&source_parent.join(target)))
}

pub fn normalize_relative_path(path: &Utf8Path) -> String {
    let mut parts = Vec::new();
    for segment in path.as_str().split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            segment => parts.push(segment),
        }
    }
    parts.join("/")
}

pub fn ensure_trailing_slash(route: &str) -> String {
    if route == "/" || route.ends_with('/') {
        route.to_string()
    } else {
        format!("{route}/")
    }
}

pub fn markdown_references(content: &str) -> Vec<MarkdownReference> {
    Parser::new_ext(content, Options::all())
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Start(Tag::Link { dest_url, .. }) => Some(MarkdownReference {
                kind: MarkdownReferenceKind::Link,
                target: dest_url.to_string(),
                byte_start: range.start,
                byte_end: range.end,
            }),
            Event::Start(Tag::Image { dest_url, .. }) => Some(MarkdownReference {
                kind: MarkdownReferenceKind::Image,
                target: dest_url.to_string(),
                byte_start: range.start,
                byte_end: range.end,
            }),
            _ => None,
        })
        .collect()
}

pub fn is_special_target(target: &str) -> bool {
    target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with("tel:")
        || target.starts_with("javascript:")
        || target.starts_with("data:")
        || target.starts_with("/__")
}

pub fn split_fragment(target: &str) -> (&str, Option<&str>) {
    let target = strip_query(target);
    match target.find('#') {
        Some(idx) => (&target[..idx], Some(&target[idx + 1..])),
        None => (target, None),
    }
}

pub fn is_likely_static_file(path: &str) -> bool {
    let extensions = [
        ".css", ".js", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".woff", ".woff2", ".ttf",
        ".eot", ".pdf", ".zip", ".tar", ".gz", ".webp", ".jxl", ".xml", ".txt", ".wasm",
    ];
    extensions.iter().any(|ext| path.ends_with(ext))
}

pub fn byte_to_line_column(content: &str, byte_offset: usize) -> (u32, u32) {
    let mut line = 1;
    let mut column = 1;
    for (idx, ch) in content.char_indices() {
        if idx >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}
