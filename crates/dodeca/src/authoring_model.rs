use std::collections::{HashMap, HashSet, VecDeque};

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use gingembre::ast::{Node, Span};
use gingembre::parser::Parser as TemplateParser;
use gingembre::semantic::TemplateSemanticIndex;
use hotmeal::{Document, NodeId, NodeKind, StrTendril, parse};
use ignore::WalkBuilder;

use crate::BuildContext;
use crate::db::{
    DataFile, DataRegistry, Database, MarkdownRenderSettings, SassFile, SassRegistry, SourceFile,
    SourceRegistry, StaticFile, StaticRegistry, TemplateFile, TemplateRegistry,
};
use crate::queries::{build_tree, load_all_templates, source_to_route_map};
use crate::render::{Renderable, render_authoring_html};
use crate::template_host::TEMPLATE_FUNCTION_NAMES;
use crate::template_paths::{logical_template_path, physical_template_path};
use crate::types::{
    DataContent, DataPath, Route, SassContent, SassPath, SourceContent, SourcePath, StaticPath,
    TemplateContent, TemplatePath,
};

#[derive(Debug, Clone)]
pub struct AuthoringDocumentOverlay {
    pub path: AuthoringInputPath,
    pub content: String,
}

pub struct AuthoringWorkspace {
    ctx: BuildContext,
    overlay_inputs: HashSet<AuthoringInputPath>,
}

#[derive(Clone)]
pub struct AuthoringWorkspaceInputs {
    db: std::sync::Arc<Database>,
    content_dir: Utf8PathBuf,
    sources: std::collections::BTreeMap<SourcePath, SourceFile>,
    templates: std::collections::BTreeMap<TemplatePath, TemplateFile>,
    static_files: std::collections::BTreeMap<StaticPath, StaticFile>,
    data_files: std::collections::BTreeMap<DataPath, DataFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AuthoringInputPath {
    Source(String),
    Template(String),
    Sass(String),
    Static(String),
    Dist(String),
    Data(String),
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringHeading {
    pub id: String,
    pub title: String,
    pub level: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringPageKind {
    Page,
    Section,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedHref {
    pub href: String,
    pub origin: Option<RenderedHrefOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedHrefOrigin {
    pub path: AuthoringInputPath,
    pub byte_start: usize,
    pub byte_end: usize,
}

pub async fn load_authoring_project(
    sources: &[crate::config::ResolvedSource],
    overlays: &[AuthoringDocumentOverlay],
) -> Result<AuthoringProject> {
    let mut workspace = AuthoringWorkspace::new(sources)?;
    workspace.apply_overlays(overlays)?;
    workspace.inputs().project().await
}

impl AuthoringWorkspace {
    /// Build an authoring workspace over all `sources` (multi-source aware —
    /// same mount-prefixed keys as `BuildContext`/serve). The first source's
    /// content dir is the primary, from which templates/sass/static/data are
    /// derived. Markdown is loaded from every source.
    pub fn new(sources: &[crate::config::ResolvedSource]) -> Result<Self> {
        let primary = sources
            .first()
            .map(|s| s.content_dir.clone())
            .ok_or_else(|| eyre!("authoring workspace needs at least one source"))?;
        let output_dir = primary.parent().unwrap_or(&primary).join("public");
        let mut ctx = BuildContext::new(&primary, &output_dir);
        ctx.set_source_roots(sources.to_vec());
        MarkdownRenderSettings::set(&*ctx.db, false)?;

        ctx.load_sources()?;
        ctx.load_templates()?;
        ctx.load_data()?;
        load_authoring_static_paths(&mut ctx)?;
        set_registries(&ctx)?;

        Ok(Self {
            ctx,
            overlay_inputs: HashSet::new(),
        })
    }

    pub fn content_dir(&self) -> &Utf8Path {
        &self.ctx.content_dir
    }

    pub fn input_path_for_absolute_path(
        &self,
        path: &Utf8Path,
    ) -> Result<Option<AuthoringInputPath>> {
        input_path_for_absolute_path(&self.ctx, path)
    }

    pub fn apply_file_change(
        &mut self,
        input_path: &AuthoringInputPath,
        content: Option<&str>,
    ) -> Result<()> {
        match input_path {
            AuthoringInputPath::Source(path) => {
                self.update_source(path, content)?;
                self.set_sources()?;
            }
            AuthoringInputPath::Template(path) => {
                self.update_template(path, content)?;
                self.set_templates()?;
            }
            AuthoringInputPath::Sass(path) => {
                self.update_sass(path, content)?;
                self.set_sass()?;
            }
            AuthoringInputPath::Static(path) | AuthoringInputPath::Dist(path) => {
                self.update_static(path, content.map(str::as_bytes))?;
                self.set_static_files()?;
            }
            AuthoringInputPath::Data(path) => {
                self.update_data(path, content)?;
                self.set_data_files()?;
            }
        }
        Ok(())
    }

    pub fn apply_overlays(&mut self, overlays: &[AuthoringDocumentOverlay]) -> Result<()> {
        let incoming = overlays
            .iter()
            .map(|overlay| overlay.path.clone())
            .collect::<HashSet<_>>();
        let removed = self
            .overlay_inputs
            .difference(&incoming)
            .cloned()
            .collect::<Vec<_>>();

        let mut changed = RegistryChanges::default();
        for input_path in removed {
            self.restore_input_from_disk(&input_path, &mut changed)?;
        }

        for overlay in overlays {
            self.apply_overlay(overlay, &mut changed)?;
        }

        self.overlay_inputs = incoming;
        self.set_changed_registries(changed)?;
        Ok(())
    }

    pub fn inputs(&self) -> AuthoringWorkspaceInputs {
        AuthoringWorkspaceInputs {
            db: self.ctx.db.clone(),
            content_dir: self.ctx.content_dir.clone(),
            sources: self.ctx.sources.clone(),
            templates: self.ctx.templates.clone(),
            static_files: self.ctx.static_files.clone(),
            data_files: self.ctx.data_files.clone(),
        }
    }

    fn apply_overlay(
        &mut self,
        overlay: &AuthoringDocumentOverlay,
        changed: &mut RegistryChanges,
    ) -> Result<()> {
        match &overlay.path {
            AuthoringInputPath::Source(path) => {
                self.update_source(path, Some(&overlay.content))?;
                changed.sources = true;
            }
            AuthoringInputPath::Template(path) => {
                self.update_template(path, Some(&overlay.content))?;
                changed.templates = true;
            }
            AuthoringInputPath::Sass(path) => {
                self.update_sass(path, Some(&overlay.content))?;
                changed.sass = true;
            }
            AuthoringInputPath::Static(path) | AuthoringInputPath::Dist(path) => {
                self.update_static(path, Some(overlay.content.as_bytes()))?;
                changed.static_files = true;
            }
            AuthoringInputPath::Data(path) => {
                self.update_data(path, Some(&overlay.content))?;
                changed.data = true;
            }
        }
        Ok(())
    }

    fn restore_input_from_disk(
        &mut self,
        input_path: &AuthoringInputPath,
        changed: &mut RegistryChanges,
    ) -> Result<()> {
        match input_path {
            AuthoringInputPath::Source(path) => {
                self.update_source(path, None)?;
                changed.sources = true;
            }
            AuthoringInputPath::Template(path) => {
                self.update_template(path, None)?;
                changed.templates = true;
            }
            AuthoringInputPath::Sass(path) => {
                self.update_sass(path, None)?;
                changed.sass = true;
            }
            AuthoringInputPath::Static(path) | AuthoringInputPath::Dist(path) => {
                self.update_static(path, None)?;
                changed.static_files = true;
            }
            AuthoringInputPath::Data(path) => {
                self.update_data(path, None)?;
                changed.data = true;
            }
        }
        Ok(())
    }

    fn update_source(&mut self, source_file: &str, content: Option<&str>) -> Result<()> {
        let source_path = SourcePath::new(source_file.to_string());
        // `source_file` is a mount-prefixed key; reverse it to the on-disk path
        // under the owning source's content dir.
        let (root_dir, rel) =
            crate::build_context::source_for_key(&self.ctx.source_roots, source_file)
                .map(|(src, rel)| (src.content_dir, rel))
                .unwrap_or_else(|| (self.ctx.content_dir.clone(), source_file.to_string()));
        let on_disk = root_dir.join(&rel);
        let content = match content {
            Some(content) => content.to_string(),
            None if on_disk.exists() => std::fs::read_to_string(&on_disk)?,
            None => {
                self.ctx.sources.remove(&source_path);
                return Ok(());
            }
        };
        let source = SourceFile::new(
            &*self.ctx.db,
            source_path.clone(),
            SourceContent::new(content),
            source_last_modified(&root_dir, &rel),
        )?;
        self.ctx.sources.insert(source_path, source);
        Ok(())
    }

    fn update_template(&mut self, template_file: &str, content: Option<&str>) -> Result<()> {
        let path = physical_template_path(&self.ctx.templates_dir(), template_file);
        let template_path = TemplatePath::new(template_file.to_string());
        let content = match content {
            Some(content) => content.to_string(),
            None if path.exists() => std::fs::read_to_string(&path)?,
            None => {
                self.ctx.templates.remove(&template_path);
                return Ok(());
            }
        };
        let template = TemplateFile::new(
            &*self.ctx.db,
            template_path.clone(),
            TemplateContent::new(content),
        )?;
        self.ctx.templates.insert(template_path, template);
        Ok(())
    }

    fn update_sass(&mut self, sass_file: &str, content: Option<&str>) -> Result<()> {
        let path = self.ctx.sass_dir().join(sass_file);
        let sass_path = SassPath::new(sass_file.to_string());
        let content = match content {
            Some(content) => content.to_string(),
            None if path.exists() => std::fs::read_to_string(&path)?,
            None => {
                self.ctx.sass_files.remove(&sass_path);
                return Ok(());
            }
        };
        let sass = SassFile::new(&*self.ctx.db, sass_path.clone(), SassContent::new(content))?;
        self.ctx.sass_files.insert(sass_path, sass);
        Ok(())
    }

    fn update_static(&mut self, static_file: &str, content: Option<&[u8]>) -> Result<()> {
        let static_path = StaticPath::new(static_file.to_string());
        let content = match content {
            Some(content) => content.to_vec(),
            None => match static_content_from_disk(&self.ctx, static_file)? {
                Some(content) => content,
                None => {
                    self.ctx.static_files.remove(&static_path);
                    return Ok(());
                }
            },
        };
        let file = StaticFile::new(&*self.ctx.db, static_path.clone(), content)?;
        self.ctx.static_files.insert(static_path, file);
        Ok(())
    }

    fn update_data(&mut self, data_file: &str, content: Option<&str>) -> Result<()> {
        let path = self.ctx.data_dir().join(data_file);
        let data_path = DataPath::new(data_file.to_string());
        let content = match content {
            Some(content) => content.to_string(),
            None if path.exists() => std::fs::read_to_string(&path)?,
            None => {
                self.ctx.data_files.remove(&data_path);
                return Ok(());
            }
        };
        let data = DataFile::new(&*self.ctx.db, data_path.clone(), DataContent::new(content))?;
        self.ctx.data_files.insert(data_path, data);
        Ok(())
    }

    fn set_changed_registries(&self, changed: RegistryChanges) -> Result<()> {
        if changed.sources {
            self.set_sources()?;
        }
        if changed.templates {
            self.set_templates()?;
        }
        if changed.sass {
            self.set_sass()?;
        }
        if changed.static_files {
            self.set_static_files()?;
        }
        if changed.data {
            self.set_data_files()?;
        }
        Ok(())
    }

    fn set_sources(&self) -> Result<()> {
        SourceRegistry::set(&*self.ctx.db, self.ctx.sources.values().copied().collect())?;
        Ok(())
    }

    fn set_templates(&self) -> Result<()> {
        TemplateRegistry::set(
            &*self.ctx.db,
            self.ctx.templates.values().copied().collect(),
        )?;
        Ok(())
    }

    fn set_sass(&self) -> Result<()> {
        SassRegistry::set(
            &*self.ctx.db,
            self.ctx.sass_files.values().copied().collect(),
        )?;
        Ok(())
    }

    fn set_static_files(&self) -> Result<()> {
        StaticRegistry::set(
            &*self.ctx.db,
            self.ctx.static_files.values().copied().collect(),
        )?;
        Ok(())
    }

    fn set_data_files(&self) -> Result<()> {
        DataRegistry::set(
            &*self.ctx.db,
            self.ctx.data_files.values().copied().collect(),
        )?;
        Ok(())
    }
}

#[derive(Default)]
struct RegistryChanges {
    sources: bool,
    templates: bool,
    sass: bool,
    static_files: bool,
    data: bool,
}

fn static_content_from_disk(ctx: &BuildContext, static_file: &str) -> Result<Option<Vec<u8>>> {
    let dist_path = ctx.dist_dir().join(static_file);
    if dist_path.exists() {
        return Ok(Some(std::fs::read(dist_path)?));
    }

    let static_path = ctx.static_dir().join(static_file);
    if static_path.exists() {
        return Ok(Some(std::fs::read(static_path)?));
    }

    Ok(None)
}

fn input_path_for_absolute_path(
    ctx: &BuildContext,
    path: &Utf8Path,
) -> Result<Option<AuthoringInputPath>> {
    // Content is per-source: attribute the file to the owning source (longest
    // content-dir prefix) and key it mount-prefixed, exactly as `load_sources`.
    if path.extension() == Some("md") {
        let mut best: Option<(&crate::config::ResolvedSource, Utf8PathBuf)> = None;
        for root in &ctx.source_roots {
            if let Ok(relative) = path.strip_prefix(&root.content_dir) {
                let longer = best.as_ref().is_none_or(|(b, _)| {
                    root.content_dir.as_str().len() > b.content_dir.as_str().len()
                });
                if longer {
                    best = Some((root, relative.to_owned()));
                }
            }
        }
        if let Some((root, relative)) = best {
            let key = crate::build_context::mounted_key(&root.mount, relative.as_str());
            return Ok(Some(AuthoringInputPath::Source(key)));
        }
    }

    if let Ok(relative) = path.strip_prefix(ctx.templates_dir()) {
        if let Some(path) = logical_template_path(relative) {
            return Ok(Some(AuthoringInputPath::Template(path)));
        }
    }

    if let Ok(relative) = path.strip_prefix(ctx.sass_dir()) {
        if matches!(path.extension(), Some("scss" | "sass")) {
            return Ok(Some(AuthoringInputPath::Sass(relative.to_string())));
        }
    }

    if let Ok(relative) = path.strip_prefix(ctx.static_dir()) {
        return Ok(Some(AuthoringInputPath::Static(relative.to_string())));
    }

    if let Ok(relative) = path.strip_prefix(ctx.dist_dir()) {
        return Ok(Some(AuthoringInputPath::Dist(relative.to_string())));
    }

    if let Ok(relative) = path.strip_prefix(ctx.data_dir()) {
        if path.extension().is_some_and(crate::is_data_file_extension) {
            return Ok(Some(AuthoringInputPath::Data(relative.to_string())));
        }
    }

    Ok(None)
}

impl AuthoringWorkspaceInputs {
    pub async fn project(self) -> Result<AuthoringProject> {
        build_authoring_project_from_inputs(self).await
    }
}

/// Builds an `AuthoringProject` from the host's live db (snapshot + overlays),
/// so the in-process LSP reuses the server's already-built + memoized state
/// instead of re-loading the workspace from disk. The binary injects the impl
/// (it has the `SiteServer`); the LSP Backend calls it via this trait — same
/// inversion of control as the LSP runner, breaking the crate cycle.
pub trait AuthoringProjectProvider: Send + Sync {
    /// `overlays` are open documents as `(absolute_path, content)`.
    fn project<'a>(
        &'a self,
        overlays: Vec<(String, String)>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AuthoringProject>> + Send + 'a>>;
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

async fn build_authoring_project_from_inputs(
    inputs: AuthoringWorkspaceInputs,
) -> Result<AuthoringProject> {
    build_authoring_project_on_db(ProjectBuildInputs {
        db: &*inputs.db,
        content_dir: &inputs.content_dir,
        sources: &inputs.sources,
        templates: &inputs.templates,
        static_files: &inputs.static_files,
        data_files: &inputs.data_files,
    })
    .await
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
        let parsed_template = TemplateParser::new(&path, &content)
            .parse_recovered()
            .template;
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

fn source_last_modified(content_dir: &Utf8Path, source_file: &str) -> i64 {
    std::fs::metadata(content_dir.join(source_file))
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn set_registries(ctx: &BuildContext) -> Result<()> {
    SourceRegistry::set(&*ctx.db, ctx.sources.values().copied().collect())?;
    TemplateRegistry::set(&*ctx.db, ctx.templates.values().copied().collect())?;
    SassRegistry::set(&*ctx.db, ctx.sass_files.values().copied().collect())?;
    StaticRegistry::set(&*ctx.db, ctx.static_files.values().copied().collect())?;
    DataRegistry::set(&*ctx.db, ctx.data_files.values().copied().collect())?;
    Ok(())
}

fn load_authoring_static_paths(ctx: &mut BuildContext) -> Result<()> {
    let mut files = std::collections::BTreeMap::new();
    load_static_dir(&ctx.static_dir(), &mut files)?;
    load_static_dir(&ctx.dist_dir(), &mut files)?;

    for (relative, content) in files {
        let static_path = StaticPath::new(relative);
        let static_file = crate::db::StaticFile::new(&*ctx.db, static_path.clone(), content)?;
        ctx.static_files.insert(static_path, static_file);
    }
    Ok(())
}

fn load_static_dir(
    dir: &Utf8Path,
    files: &mut std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in WalkBuilder::new(dir).build().filter_map(|entry| entry.ok()) {
        let path = Utf8PathBuf::from_path_buf(entry.into_path())
            .map_err(|path| eyre!("static path is not UTF-8: {}", path.display()))?;
        if !path.is_file() {
            continue;
        }
        let relative = path.strip_prefix(dir)?.to_string();
        files.insert(relative, std::fs::read(path)?);
    }
    Ok(())
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
        let template = TemplateParser::new(template_file, content)
            .parse_recovered()
            .template;
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

fn normalize_route(path: &str) -> String {
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

fn strip_query(target: &str) -> &str {
    target.split('?').next().unwrap_or(target)
}
