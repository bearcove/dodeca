use std::collections::{HashMap, HashSet, VecDeque};

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use gingembre::ast::{Node, Span};
use gingembre::semantic::TemplateSemanticIndex;
use hotmeal::{Document, NodeId, NodeKind, StrTendril, parse};

use crate::db::{DataFile, SourceFile, StaticFile, TemplateFile};
use crate::queries::{build_tree, load_all_templates, source_to_route_map};
use crate::render::{Renderable, render_authoring_html};
use crate::template_host::TEMPLATE_FUNCTION_NAMES;
use crate::template_paths::physical_template_path;
use crate::types::{DataPath, Route, SourcePath, StaticPath, TemplatePath};

/// A dead-link / authoring diagnostic, with a one-based line/column span (the
/// editor crate converts to a tower_lsp range at the boundary). Lives here so
/// the diagnostics analysis can flow through picante tracked queries.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct AuthoringDiagnostic {
    pub source_file: String,
    pub route: String,
    pub kind: AuthoringDiagnosticKind,
    pub target: String,
    pub resolved_route: Option<String>,
    pub message: String,
    pub line: u32,
    pub column: u32,
    pub line_end: u32,
    pub column_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum AuthoringDiagnosticKind {
    Route,
    Anchor,
    Source,
    StaticAsset,
    Frontmatter,
    MissingTemplate,
    MissingBlock,
    UnknownMacro,
    UnknownFilter,
    UnknownTest,
    DuplicateTitle,
    DuplicateRoute,
    OrphanPage,
    NoInboundLinks,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
#[repr(u8)]
pub enum AuthoringInputPath {
    Source(String),
    Template(String),
    Sass(String),
    Static(String),
    Dist(String),
    Data(String),
}

#[derive(Debug, Clone, facet::Facet)]
pub struct AuthoringProject {
    pub pages: Vec<AuthoringPage>,
    pub known_routes: HashSet<String>,
    pub headings_by_route: HashMap<String, HashSet<String>>,
    pub source_to_route: HashMap<String, String>,
    pub route_to_source: HashMap<String, String>,
    pub source_contents: HashMap<String, String>,
    pub template_paths: HashMap<String, Utf8PathBuf>,
    pub template_contents: HashMap<String, String>,
    pub template_semantics: HashMap<String, TemplateSemanticIndex>,
    pub static_paths: HashMap<String, Utf8PathBuf>,
    pub data_paths: HashMap<String, Utf8PathBuf>,
    pub data_keys: Vec<String>,
    pub rendered_hrefs_by_route: HashMap<String, Vec<RenderedHref>>,
}

const TEMPLATE_CONTEXT_ROOTS: &[&str] =
    &["config", "page", "section", "current_path", "root", "data"];

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct AuthoringPage {
    pub kind: AuthoringPageKind,
    pub route: String,
    pub source_file: String,
    pub title: String,
    pub description: Option<String>,
    pub template: String,
    pub output_path: String,
    pub headings: Vec<AuthoringHeading>,
    pub heading_ids: Vec<String>,
    pub link_base_route: String,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct AuthoringHeading {
    pub id: String,
    pub title: String,
    pub level: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum AuthoringPageKind {
    Page,
    Section,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct RenderedHref {
    pub href: String,
    pub origin: Option<RenderedHrefOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct RenderedHrefOrigin {
    pub path: AuthoringInputPath,
    pub byte_start: usize,
    pub byte_end: usize,
}

/// Builds an `AuthoringProject` from the host's live db (snapshot + overlays),
/// so the in-process LSP reuses the server's already-built + memoized state
/// instead of re-loading the workspace from disk. The binary injects the impl
/// (it has the `SiteServer`); the LSP Backend calls it via this trait — same
/// inversion of control as the LSP runner, breaking the crate cycle.
pub trait AuthoringProjectProvider: Send + Sync {
    /// Hand back an overlaid [`AuthoringSnapshot`] — an isolated db snapshot with
    /// `overlays` (open documents as `(absolute_path, content)`) applied. The LSP
    /// runs the authoring tracked queries + the project builder against it.
    fn snapshot<'a>(
        &'a self,
        overlays: Vec<(String, String)>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AuthoringSnapshot>> + Send + 'a>>;
}

/// Map an absolute on-disk path to its mount-prefixed source key (inverse of
/// `build_context::source_for_key`'s path-building), using the source list.
fn source_key_for_path(sources: &[crate::config::ResolvedSource], abs: &str) -> Option<String> {
    let abs = Utf8Path::new(abs);
    for source in sources {
        if let Ok(rel) = abs.strip_prefix(&source.content_dir) {
            return Some(crate::build_context::mounted_key(
                &source.mount,
                rel.as_str(),
            ));
        }
    }
    None
}

/// An overlaid snapshot of a project db — dodeca's VFS handle. Holds the real
/// db (so `TASK_DB`-scoped cell RPC reaches the live hub), an isolated
/// [`DatabaseSnapshot`] with the editor's open buffers overlaid as `SourceFile`
/// inputs, and the workspace's primary content dir. The authoring tracked
/// queries + the project builder run against `snapshot`; picante memoizes within
/// it, and the snapshot's inherited render cells make unchanged pages free.
pub struct AuthoringSnapshot {
    db: std::sync::Arc<crate::db::Database>,
    snapshot: crate::db::DatabaseSnapshot,
    content_dir: Utf8PathBuf,
}

/// Build an overlaid [`AuthoringSnapshot`] from a live db: snapshot it (isolated),
/// then overlay the editor's open documents (`(abs_path, buffer)`) as `SourceFile`
/// inputs — this is dodeca's VFS. The snapshot is private to the caller, so
/// queries see the unsaved buffers without touching the real db.
///
/// Shared by the in-process (browser-editor) LSP and the standalone `ddc lsp`.
pub async fn overlay_snapshot(
    db: &std::sync::Arc<crate::db::Database>,
    sources_list: &[crate::config::ResolvedSource],
    overlays: Vec<(String, String)>,
) -> Result<AuthoringSnapshot> {
    use crate::db::{DatabaseSnapshot, SourceFile, SourceRegistry};

    let content_dir = sources_list
        .first()
        .map(|s| s.content_dir.clone())
        .ok_or_else(|| eyre::eyre!("no workspace content dir"))?;

    let snapshot = DatabaseSnapshot::from_database(db).await;

    // Overlay open documents onto the snapshot's isolated copy.
    let mut sources = SourceRegistry::sources(&snapshot)
        .ok()
        .flatten()
        .unwrap_or_default();
    for (path, content) in &overlays {
        let Some(key) = source_key_for_path(sources_list, path) else {
            continue;
        };
        let file = SourceFile::new(
            &snapshot,
            crate::types::SourcePath::new(key.clone()),
            crate::types::SourceContent::new(content.clone()),
            0,
        )
        .map_err(|e| eyre::eyre!("overlay source: {e:?}"))?;
        match sources.iter().position(|s| {
            s.path(&snapshot)
                .ok()
                .map(|p| p.as_str() == key)
                .unwrap_or(false)
        }) {
            Some(i) => sources[i] = file,
            None => sources.push(file),
        }
    }
    SourceRegistry::set(&snapshot, sources).map_err(|e| eyre::eyre!("set overlays: {e:?}"))?;

    Ok(AuthoringSnapshot {
        db: db.clone(),
        snapshot,
        content_dir,
    })
}

impl AuthoringSnapshot {
    /// The overlaid snapshot db — run authoring tracked queries against this.
    pub fn db(&self) -> &crate::db::DatabaseSnapshot {
        &self.snapshot
    }

    pub fn content_dir(&self) -> &Utf8Path {
        &self.content_dir
    }

    /// Run `fut` with `TASK_DB` scoped to the real db, so render queries reach
    /// the live cell hub even though they execute against the snapshot.
    pub async fn scoped<T>(&self, fut: impl std::future::Future<Output = T> + Send) -> T {
        crate::db::TASK_DB.scope(self.db.clone(), fut).await
    }

    /// Build the [`AuthoringProject`] from this overlaid snapshot.
    /// The memoized [`AuthoringProject`] for this snapshot (via the
    /// `authoring_project` tracked query, so repeated requests in the same
    /// revision hit picante's cache rather than rebuilding).
    pub async fn project(&self) -> Result<AuthoringProject> {
        let content_dir = self.content_dir.clone();
        self.scoped(async move {
            match crate::authoring_graph::authoring_project(&self.snapshot, content_dir).await {
                Ok(result) => result.map_err(|e| eyre::eyre!(e)),
                Err(e) => Err(eyre::eyre!("authoring_project query: {e:?}")),
            }
        })
        .await
    }

    /// The memoized content/route graph for this snapshot (via the
    /// `content_graph` tracked query).
    pub async fn content_graph(&self) -> Result<Vec<crate::authoring_graph::RouteGraphNode>> {
        let content_dir = self.content_dir.clone();
        self.scoped(async move {
            match crate::authoring_graph::content_graph(&self.snapshot, content_dir).await {
                Ok(result) => result.map_err(|e| eyre::eyre!(e)),
                Err(e) => Err(eyre::eyre!("content_graph query: {e:?}")),
            }
        })
        .await
    }

    /// The memoized template authoring index for this snapshot.
    pub async fn template_index(
        &self,
    ) -> Result<crate::authoring_templates::TemplateAuthoringIndex> {
        let content_dir = self.content_dir.clone();
        self.scoped(async move {
            match crate::authoring_templates::template_authoring_index(&self.snapshot, content_dir)
                .await
            {
                Ok(result) => result.map_err(|e| eyre::eyre!(e)),
                Err(e) => Err(eyre::eyre!("template_authoring_index query: {e:?}")),
            }
        })
        .await
    }

    /// The memoized per-source frontmatter document targets for this snapshot.
    pub async fn frontmatter_targets(
        &self,
    ) -> Result<HashMap<String, Vec<crate::authoring_templates::FrontmatterDocumentTarget>>> {
        let content_dir = self.content_dir.clone();
        self.scoped(async move {
            match crate::authoring_templates::source_frontmatter_targets(
                &self.snapshot,
                content_dir,
            )
            .await
            {
                Ok(result) => result.map_err(|e| eyre::eyre!(e)),
                Err(e) => Err(eyre::eyre!("source_frontmatter_targets query: {e:?}")),
            }
        })
        .await
    }
}

/// Build an [`AuthoringProject`] from any `DB: Db` (the overlaid snapshot or the
/// owned db), pulling the source/template/static/data maps from the registries.
/// `TASK_DB` must already be scoped by the caller so render queries reach the
/// cell hub. Backs the `authoring_project` tracked query.
pub async fn build_authoring_project_from_db<DB: crate::db::Db>(
    db: &DB,
    content_dir: &Utf8Path,
) -> Result<AuthoringProject> {
    use crate::db::{DataRegistry, SourceRegistry, StaticRegistry, TemplateRegistry};

    let sources = SourceRegistry::sources(db)
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| Some(((*f.path(db).ok()?).clone(), f)))
        .collect();
    let templates = TemplateRegistry::templates(db)
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| Some(((*f.path(db).ok()?).clone(), f)))
        .collect();
    let static_files = StaticRegistry::files(db)
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| Some(((*f.path(db).ok()?).clone(), f)))
        .collect();
    let data_files = DataRegistry::files(db)
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| Some(((*f.path(db).ok()?).clone(), f)))
        .collect();
    build_authoring_project_on_db(ProjectBuildInputs {
        db,
        content_dir,
        sources: &sources,
        templates: &templates,
        static_files: &static_files,
        data_files: &data_files,
    })
    .await
}

/// An [`AuthoringProjectProvider`] over a standalone db (not a `SiteServer`) —
/// for the `ddc lsp` editor integration. Owns the loaded db + its sources; VFS
/// comes from [`overlay_snapshot`].
pub struct DbAuthoringProvider {
    pub db: std::sync::Arc<crate::db::Database>,
    pub sources: Vec<crate::config::ResolvedSource>,
}

impl AuthoringProjectProvider for DbAuthoringProvider {
    fn snapshot<'a>(
        &'a self,
        overlays: Vec<(String, String)>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AuthoringSnapshot>> + Send + 'a>>
    {
        Box::pin(async move { overlay_snapshot(&self.db, &self.sources, overlays).await })
    }
}

/// Borrowed inputs for building an `AuthoringProject` over any `DB: Db` — the
/// owned `Database` (disk workspace) or a `DatabaseSnapshot` (host db, for the
/// in-process LSP sharing the server's already-built + memoized state).
pub struct ProjectBuildInputs<'a, DB> {
    pub db: &'a DB,
    pub content_dir: &'a Utf8Path,
    pub sources: &'a std::collections::BTreeMap<SourcePath, SourceFile>,
    pub templates: &'a std::collections::BTreeMap<TemplatePath, TemplateFile>,
    pub static_files: &'a std::collections::BTreeMap<StaticPath, StaticFile>,
    pub data_files: &'a std::collections::BTreeMap<DataPath, DataFile>,
}

/// Build an `AuthoringProject` from any database — the disk workspace or a host
/// db snapshot. All queries (`build_tree`, `source_to_route_map`, content
/// lookups, rendering) are `DB`-generic; a snapshot reuses the host's memoized
/// renders, so only the edited file's dependents recompute.
pub async fn build_authoring_project_on_db<DB: crate::db::Db>(
    inputs: ProjectBuildInputs<'_, DB>,
) -> Result<AuthoringProject> {
    let site_tree = build_tree(inputs.db)
        .await?
        .map_err(|errors| eyre!("failed to parse source files for authoring model: {errors:?}"))?;
    let source_to_route = source_to_route_map(inputs.db).await?;
    let route_to_source = source_to_route
        .iter()
        .map(|(source, route)| (route.clone(), source.clone()))
        .collect::<HashMap<_, _>>();

    let mut source_contents = HashMap::new();
    for (source_path, source) in inputs.sources {
        source_contents.insert(
            source_path.to_string(),
            source.content(inputs.db)?.as_str().to_string(),
        );
    }

    let mut pages = Vec::new();
    for (source_file, route) in &source_to_route {
        let source_path = SourcePath::new(source_file.clone());
        let route_key = Route::new(route.clone());
        if source_path.is_section_index() {
            if let Some(section) = site_tree.sections.get(&route_key) {
                pages.push(AuthoringPage {
                    kind: AuthoringPageKind::Section,
                    route: route.clone(),
                    source_file: source_file.clone(),
                    title: section.title.as_str().to_string(),
                    description: section.description.clone(),
                    template: section_template_name(section.route.as_str(), &section.template),
                    output_path: route_output_path(section.route.as_str()),
                    headings: section
                        .headings
                        .iter()
                        .map(|heading| AuthoringHeading {
                            id: heading.id.clone(),
                            title: heading.title.clone(),
                            level: heading.level,
                        })
                        .collect(),
                    heading_ids: section
                        .headings
                        .iter()
                        .map(|heading| heading.id.clone())
                        .collect(),
                    link_base_route: section.route.as_str().to_string(),
                });
            }
        } else if let Some(page) = site_tree.pages.get(&route_key) {
            pages.push(AuthoringPage {
                kind: AuthoringPageKind::Page,
                route: route.clone(),
                source_file: source_file.clone(),
                title: page.title.as_str().to_string(),
                description: None,
                template: page
                    .template
                    .clone()
                    .unwrap_or_else(|| "page.html".to_string()),
                output_path: route_output_path(page.route.as_str()),
                headings: page
                    .headings
                    .iter()
                    .map(|heading| AuthoringHeading {
                        id: heading.id.clone(),
                        title: heading.title.clone(),
                        level: heading.level,
                    })
                    .collect(),
                heading_ids: page
                    .headings
                    .iter()
                    .map(|heading| heading.id.clone())
                    .collect(),
                link_base_route: page.section_route.as_str().to_string(),
            });
        }
    }

    pages.sort_by(|a, b| {
        a.route
            .cmp(&b.route)
            .then_with(|| a.source_file.cmp(&b.source_file))
    });

    let known_routes = pages.iter().map(|page| page.route.clone()).collect();
    let headings_by_route = pages
        .iter()
        .map(|page| {
            (
                page.route.clone(),
                page.heading_ids.iter().cloned().collect::<HashSet<_>>(),
            )
        })
        .collect();
    let project_dir = inputs
        .content_dir
        .parent()
        .unwrap_or(inputs.content_dir)
        .to_path_buf();
    let template_paths = inputs
        .templates
        .keys()
        .map(|path| {
            (
                path.as_str().to_string(),
                physical_template_path(&project_dir.join("templates"), path.as_str()),
            )
        })
        .collect();
    let mut template_contents = HashMap::new();
    let mut template_semantics = HashMap::new();
    for (template_path, template) in inputs.templates {
        let path = template_path.as_str().to_string();
        let content = template.content(inputs.db)?.as_str().to_string();
        let parsed_template = gingembre::parse_template_recovered(&content);
        template_semantics.insert(
            path.clone(),
            TemplateSemanticIndex::build(
                &parsed_template,
                TEMPLATE_CONTEXT_ROOTS,
                TEMPLATE_FUNCTION_NAMES,
            ),
        );
        template_contents.insert(path, content);
    }
    let static_template_href_origins = static_template_href_origins(&template_contents);
    let mut rendered_hrefs_by_route = HashMap::new();
    let render_templates = load_all_templates(inputs.db).await?;
    for page in &pages {
        let route = Route::new(page.route.clone());
        // Narrow to the source serving this route so a mounted source previews
        // with its own chrome in the authoring UI too.
        let templates =
            crate::queries::templates_for_route(render_templates.clone(), route.as_str());
        let rendered = match page.kind {
            AuthoringPageKind::Section => {
                if let Some(section) = site_tree.sections.get(&route) {
                    render_authoring_html(Renderable::Section(section), &site_tree, templates).await
                } else {
                    None
                }
            }
            AuthoringPageKind::Page => {
                if let Some(page) = site_tree.pages.get(&route) {
                    render_authoring_html(Renderable::Page(page), &site_tree, templates).await
                } else {
                    None
                }
            }
        };
        if let Some(html) = rendered {
            rendered_hrefs_by_route.insert(
                page.route.clone(),
                rendered_html_hrefs(&html, &static_template_href_origins),
            );
        }
    }
    let static_paths = inputs
        .static_files
        .keys()
        .filter_map(|path| {
            static_source_path(inputs.content_dir, path.as_str())
                .map(|source_path| (path.as_str().to_string(), source_path))
        })
        .collect();
    let mut data_keys = inputs
        .data_files
        .keys()
        .map(|path| data_completion_key(path.as_str()))
        .collect::<Vec<_>>();
    data_keys.sort();
    data_keys.dedup();
    let data_paths = inputs
        .data_files
        .keys()
        .map(|path| {
            (
                path.as_str().to_string(),
                project_dir.join("data").join(path.as_str()),
            )
        })
        .collect();

    Ok(AuthoringProject {
        pages,
        known_routes,
        headings_by_route,
        source_to_route,
        route_to_source,
        source_contents,
        template_paths,
        template_contents,
        template_semantics,
        static_paths,
        data_paths,
        data_keys,
        rendered_hrefs_by_route,
    })
}

impl AuthoringProject {
    pub fn page_for_source_file(&self, source_file: &str) -> Option<&AuthoringPage> {
        self.pages
            .iter()
            .find(|page| page.source_file == source_file)
    }

    pub fn page_for_route(&self, target_route: &str) -> Option<&AuthoringPage> {
        let source_file = self.source_file_for_route(target_route)?;
        self.page_for_source_file(source_file)
    }

    pub fn source_file_for_route(&self, target_route: &str) -> Option<&str> {
        self.route_to_source
            .get(target_route)
            .or_else(|| self.route_to_source.get(target_route.trim_end_matches('/')))
            .or_else(|| {
                let with_slash = format!("{}/", target_route.trim_end_matches('/'));
                self.route_to_source.get(&with_slash)
            })
            .map(|source_file| source_file.as_str())
    }

    pub fn route_exists(&self, target_route: &str) -> bool {
        self.known_routes.contains(target_route)
            || {
                let without_slash = target_route.trim_end_matches('/');
                !without_slash.is_empty()
                    && without_slash != target_route
                    && self.known_routes.contains(without_slash)
            }
            || {
                let with_slash = format!("{}/", target_route.trim_end_matches('/'));
                self.known_routes.contains(&with_slash)
            }
    }

    pub fn heading_exists(&self, target_route: &str, heading_id: &str) -> Option<bool> {
        self.heading_ids_for_route(target_route)
            .map(|ids| ids.contains(heading_id))
    }

    pub fn heading_for_route(
        &self,
        target_route: &str,
        heading_id: &str,
    ) -> Option<&AuthoringHeading> {
        self.page_for_route(target_route)?
            .headings
            .iter()
            .find(|heading| heading.id == heading_id)
    }

    fn heading_ids_for_route(&self, target_route: &str) -> Option<&HashSet<String>> {
        self.headings_by_route
            .get(target_route)
            .or_else(|| {
                self.headings_by_route
                    .get(target_route.trim_end_matches('/'))
            })
            .or_else(|| {
                let with_slash = format!("{}/", target_route.trim_end_matches('/'));
                self.headings_by_route.get(&with_slash)
            })
    }

    pub fn routes_refer_to_same_page(&self, left_route: &str, right_route: &str) -> bool {
        match (
            self.source_file_for_route(left_route),
            self.source_file_for_route(right_route),
        ) {
            (Some(left_source), Some(right_source)) => left_source == right_source,
            _ => normalize_route(left_route) == normalize_route(right_route),
        }
    }

    pub fn static_target_exists(&self, source_file: &str, target: &str) -> bool {
        self.static_target_path(source_file, target).is_some()
    }

    pub fn static_target_path(&self, source_file: &str, target: &str) -> Option<&str> {
        let target = strip_query(target);
        if target.is_empty() {
            return None;
        }

        if target.starts_with('/') {
            let path = target.trim_start_matches('/');
            return self.static_paths.get(path).map(|path| path.as_str());
        }

        let source_parent = Utf8Path::new(source_file)
            .parent()
            .unwrap_or_else(|| Utf8Path::new(""));
        let content_relative = source_parent.join(target).to_string();
        self.static_paths
            .get(&content_relative)
            .or_else(|| self.static_paths.get(target))
            .map(|path| path.as_str())
    }
}

fn static_source_path(content_dir: &Utf8Path, relative: &str) -> Option<Utf8PathBuf> {
    let project_dir = content_dir.parent().unwrap_or(content_dir);
    let static_path = project_dir.join("static").join(relative);
    if static_path.exists() {
        return Some(static_path);
    }
    let dist_path = project_dir.join("dist").join(relative);
    dist_path.exists().then_some(dist_path)
}

fn data_completion_key(path: &str) -> String {
    Utf8Path::new(path)
        .file_stem()
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

fn rendered_html_hrefs(
    html: &str,
    static_template_href_origins: &HashMap<String, Vec<RenderedHrefOrigin>>,
) -> Vec<RenderedHref> {
    let input = StrTendril::from(html);
    let doc = parse(&input);
    let mut hrefs = Vec::new();
    collect_rendered_html_hrefs(&doc, doc.root, &mut hrefs);
    let mut origins = static_template_href_origins
        .iter()
        .map(|(href, origins)| {
            (
                href.clone(),
                origins.iter().cloned().collect::<VecDeque<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();
    hrefs
        .into_iter()
        .map(|href| {
            let origin = origins
                .get_mut(&href)
                .and_then(VecDeque::pop_front)
                .or_else(|| {
                    static_template_href_origins
                        .get(&href)
                        .and_then(|origins| origins.first().cloned())
                });
            RenderedHref { href, origin }
        })
        .collect()
}

fn collect_rendered_html_hrefs(doc: &Document<'_>, node_id: NodeId, hrefs: &mut Vec<String>) {
    if let NodeKind::Element(element) = &doc.get(node_id).kind
        && element.tag.as_ref() == "a"
    {
        hrefs.extend(
            element
                .attrs
                .iter()
                .filter(|(name, _)| name.local.as_ref() == "href")
                .map(|(_, value)| value.as_ref().to_string()),
        );
    }

    for child_id in node_id.children(&doc.arena) {
        collect_rendered_html_hrefs(doc, child_id, hrefs);
    }
}

fn static_template_href_origins(
    template_contents: &HashMap<String, String>,
) -> HashMap<String, Vec<RenderedHrefOrigin>> {
    let mut origins = HashMap::new();
    for (template_file, content) in template_contents {
        let template = gingembre::parse_template_recovered(content);
        collect_template_href_origins(&template.body, template_file, &mut origins);
    }
    origins
}

fn collect_template_href_origins(
    nodes: &[Node],
    template_file: &str,
    origins: &mut HashMap<String, Vec<RenderedHrefOrigin>>,
) {
    for node in nodes {
        match node {
            Node::Text(text) => {
                for href in literal_anchor_hrefs(&text.text, text.span) {
                    origins
                        .entry(href.href)
                        .or_default()
                        .push(RenderedHrefOrigin {
                            path: AuthoringInputPath::Template(template_file.to_string()),
                            byte_start: href.byte_start,
                            byte_end: href.byte_end,
                        });
                }
            }
            Node::If(node) => {
                collect_template_href_origins(&node.then_body, template_file, origins);
                for branch in &node.elif_branches {
                    collect_template_href_origins(&branch.body, template_file, origins);
                }
                if let Some(body) = &node.else_body {
                    collect_template_href_origins(body, template_file, origins);
                }
            }
            Node::For(node) => {
                collect_template_href_origins(&node.body, template_file, origins);
                if let Some(body) = &node.else_body {
                    collect_template_href_origins(body, template_file, origins);
                }
            }
            Node::Block(node) => collect_template_href_origins(&node.body, template_file, origins),
            Node::Macro(node) => collect_template_href_origins(&node.body, template_file, origins),
            Node::Print(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Import(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiteralHref {
    href: String,
    byte_start: usize,
    byte_end: usize,
}

fn literal_anchor_hrefs(text: &str, span: Span) -> Vec<LiteralHref> {
    let base_offset = span.offset();
    let bytes = text.as_bytes();
    let mut hrefs = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(tag_start) = bytes[cursor..].iter().position(|byte| *byte == b'<') else {
            break;
        };
        let tag_start = cursor + tag_start;
        let mut name_start = tag_start + 1;
        if matches!(bytes.get(name_start), Some(b'/') | Some(b'!') | Some(b'?')) {
            cursor = name_start + 1;
            continue;
        }
        while bytes.get(name_start).is_some_and(u8::is_ascii_whitespace) {
            name_start += 1;
        }
        let name_end = scan_html_name(bytes, name_start);
        if !text[name_start..name_end].eq_ignore_ascii_case("a") {
            cursor = name_end.max(tag_start + 1);
            continue;
        }
        let Some(tag_end) = scan_html_tag_end(bytes, name_end) else {
            break;
        };
        collect_href_attrs(text, bytes, name_end, tag_end, base_offset, &mut hrefs);
        cursor = tag_end + 1;
    }
    hrefs
}

fn collect_href_attrs(
    text: &str,
    bytes: &[u8],
    mut cursor: usize,
    tag_end: usize,
    base_offset: usize,
    hrefs: &mut Vec<LiteralHref>,
) {
    while cursor < tag_end {
        while cursor < tag_end
            && (bytes[cursor].is_ascii_whitespace() || matches!(bytes[cursor], b'/'))
        {
            cursor += 1;
        }
        let name_start = cursor;
        let name_end = scan_html_name(bytes, name_start);
        if name_end == name_start {
            cursor += 1;
            continue;
        }
        cursor = name_end;
        while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag_end || bytes[cursor] != b'=' {
            continue;
        }
        cursor += 1;
        while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let Some((value_start, value_end, next_cursor)) =
            scan_html_attr_value(bytes, cursor, tag_end)
        else {
            break;
        };
        if text[name_start..name_end].eq_ignore_ascii_case("href") {
            hrefs.push(LiteralHref {
                href: text[value_start..value_end].to_string(),
                byte_start: base_offset + value_start,
                byte_end: base_offset + value_end,
            });
        }
        cursor = next_cursor;
    }
}

fn scan_html_name(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':' | b'_'))
    {
        cursor += 1;
    }
    cursor
}

fn scan_html_tag_end(bytes: &[u8], mut cursor: usize) -> Option<usize> {
    let mut quote = None;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        match quote {
            Some(active) if byte == active => quote = None,
            Some(_) => {}
            None if matches!(byte, b'\'' | b'"') => quote = Some(byte),
            None if byte == b'>' => return Some(cursor),
            None => {}
        }
        cursor += 1;
    }
    None
}

fn scan_html_attr_value(
    bytes: &[u8],
    cursor: usize,
    tag_end: usize,
) -> Option<(usize, usize, usize)> {
    let quote = *bytes.get(cursor)?;
    if matches!(quote, b'\'' | b'"') {
        let value_start = cursor + 1;
        let relative_end = bytes[value_start..tag_end]
            .iter()
            .position(|byte| *byte == quote)?;
        let value_end = value_start + relative_end;
        return Some((value_start, value_end, value_end + 1));
    }
    let value_start = cursor;
    let mut value_end = cursor;
    while value_end < tag_end
        && !bytes[value_end].is_ascii_whitespace()
        && !matches!(bytes[value_end], b'>')
    {
        value_end += 1;
    }
    (value_end > value_start).then_some((value_start, value_end, value_end))
}

fn section_template_name(route: &str, template: &Option<String>) -> String {
    template.clone().unwrap_or_else(|| {
        if route == "/" {
            "index.html".to_string()
        } else {
            "section.html".to_string()
        }
    })
}

fn route_output_path(route: &str) -> String {
    let route = route.trim_matches('/');
    if route.is_empty() {
        "index.html".to_string()
    } else {
        format!("{route}/index.html")
    }
}

pub fn normalize_route(path: &str) -> String {
    let mut parts = Vec::new();

    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }

    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

pub fn strip_query(target: &str) -> &str {
    target.split('?').next().unwrap_or(target)
}
