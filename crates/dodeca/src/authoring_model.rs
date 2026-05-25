use std::collections::{HashMap, HashSet};

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use ignore::WalkBuilder;

use crate::BuildContext;
use crate::db::{
    DataRegistry, Database, MarkdownRenderSettings, SassRegistry, SourceFile, SourceRegistry,
    StaticFile, StaticRegistry, TemplateRegistry,
};
use crate::queries::{build_tree, source_to_route_map};
use crate::types::{Route, SourceContent, SourcePath, StaticPath};

#[derive(Debug, Clone)]
pub(crate) struct AuthoringDocumentOverlay {
    pub source_file: String,
    pub content: String,
}

pub(crate) struct AuthoringWorkspace {
    ctx: BuildContext,
    overlay_sources: HashSet<String>,
}

#[derive(Clone)]
pub(crate) struct AuthoringWorkspaceSnapshot {
    db: std::sync::Arc<Database>,
    content_dir: Utf8PathBuf,
    sources: std::collections::BTreeMap<SourcePath, SourceFile>,
    static_files: std::collections::BTreeMap<StaticPath, StaticFile>,
}

#[derive(Debug, Clone)]
pub(crate) struct AuthoringProject {
    pub pages: Vec<AuthoringPage>,
    pub known_routes: HashSet<String>,
    pub headings_by_route: HashMap<String, HashSet<String>>,
    pub source_to_route: HashMap<String, String>,
    pub route_to_source: HashMap<String, String>,
    pub source_contents: HashMap<String, String>,
    pub static_paths: HashMap<String, Utf8PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthoringPage {
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
pub(crate) struct AuthoringHeading {
    pub id: String,
    pub title: String,
    pub level: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthoringPageKind {
    Page,
    Section,
}

#[cfg(test)]
pub(crate) async fn load_authoring_project(
    content_dir: &Utf8Path,
    overlays: &[AuthoringDocumentOverlay],
) -> Result<AuthoringProject> {
    let mut workspace = AuthoringWorkspace::new(content_dir)?;
    workspace.apply_overlays(overlays)?;
    workspace.snapshot().project().await
}

impl AuthoringWorkspace {
    pub(crate) fn new(content_dir: &Utf8Path) -> Result<Self> {
        let output_dir = content_dir.parent().unwrap_or(content_dir).join("public");
        let mut ctx = BuildContext::new(content_dir, &output_dir);
        MarkdownRenderSettings::set(&*ctx.db, false)?;

        ctx.load_sources()?;
        load_authoring_static_paths(&mut ctx)?;
        set_registries(&ctx)?;

        Ok(Self {
            ctx,
            overlay_sources: HashSet::new(),
        })
    }

    pub(crate) fn content_dir(&self) -> &Utf8Path {
        &self.ctx.content_dir
    }

    pub(crate) fn apply_overlays(&mut self, overlays: &[AuthoringDocumentOverlay]) -> Result<()> {
        let incoming = overlays
            .iter()
            .map(|overlay| overlay.source_file.clone())
            .collect::<HashSet<_>>();
        let removed = self
            .overlay_sources
            .difference(&incoming)
            .cloned()
            .collect::<Vec<_>>();

        for source_file in removed {
            self.restore_source_from_disk(&source_file)?;
        }

        for overlay in overlays {
            let source_path = SourcePath::new(overlay.source_file.clone());
            let source = SourceFile::new(
                &*self.ctx.db,
                source_path.clone(),
                SourceContent::new(overlay.content.clone()),
                source_last_modified(&self.ctx.content_dir, &overlay.source_file),
            )?;
            self.ctx.sources.insert(source_path, source);
        }

        self.overlay_sources = incoming;
        SourceRegistry::set(&*self.ctx.db, self.ctx.sources.values().copied().collect())?;
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> AuthoringWorkspaceSnapshot {
        AuthoringWorkspaceSnapshot {
            db: self.ctx.db.clone(),
            content_dir: self.ctx.content_dir.clone(),
            sources: self.ctx.sources.clone(),
            static_files: self.ctx.static_files.clone(),
        }
    }

    fn restore_source_from_disk(&mut self, source_file: &str) -> Result<()> {
        let path = self.ctx.content_dir.join(source_file);
        let source_path = SourcePath::new(source_file.to_string());
        if !path.exists() {
            self.ctx.sources.remove(&source_path);
            return Ok(());
        }

        let source = SourceFile::new(
            &*self.ctx.db,
            source_path.clone(),
            SourceContent::new(std::fs::read_to_string(&path)?),
            source_last_modified(&self.ctx.content_dir, source_file),
        )?;
        self.ctx.sources.insert(source_path, source);
        Ok(())
    }
}

impl AuthoringWorkspaceSnapshot {
    pub(crate) async fn project(self) -> Result<AuthoringProject> {
        build_authoring_project_from_snapshot(self).await
    }
}

async fn build_authoring_project_from_snapshot(
    snapshot: AuthoringWorkspaceSnapshot,
) -> Result<AuthoringProject> {
    let site_tree = build_tree(&*snapshot.db)
        .await?
        .map_err(|errors| eyre!("failed to parse source files for authoring model: {errors:?}"))?;
    let source_to_route = source_to_route_map(&*snapshot.db).await?;
    let route_to_source = source_to_route
        .iter()
        .map(|(source, route)| (route.clone(), source.clone()))
        .collect::<HashMap<_, _>>();

    let mut source_contents = HashMap::new();
    for (source_path, source) in &snapshot.sources {
        source_contents.insert(
            source_path.to_string(),
            source.content(&*snapshot.db)?.as_str().to_string(),
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
    let static_paths = snapshot
        .static_files
        .keys()
        .filter_map(|path| {
            static_source_path(&snapshot.content_dir, path.as_str())
                .map(|source_path| (path.as_str().to_string(), source_path))
        })
        .collect();

    Ok(AuthoringProject {
        pages,
        known_routes,
        headings_by_route,
        source_to_route,
        route_to_source,
        source_contents,
        static_paths,
    })
}

impl AuthoringProject {
    pub(crate) fn page_for_source_file(&self, source_file: &str) -> Option<&AuthoringPage> {
        self.pages
            .iter()
            .find(|page| page.source_file == source_file)
    }

    pub(crate) fn page_for_route(&self, target_route: &str) -> Option<&AuthoringPage> {
        let source_file = self.source_file_for_route(target_route)?;
        self.page_for_source_file(source_file)
    }

    pub(crate) fn source_file_for_route(&self, target_route: &str) -> Option<&str> {
        self.route_to_source
            .get(target_route)
            .or_else(|| self.route_to_source.get(target_route.trim_end_matches('/')))
            .or_else(|| {
                let with_slash = format!("{}/", target_route.trim_end_matches('/'));
                self.route_to_source.get(&with_slash)
            })
            .map(|source_file| source_file.as_str())
    }

    pub(crate) fn route_exists(&self, target_route: &str) -> bool {
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

    pub(crate) fn heading_exists(&self, target_route: &str, heading_id: &str) -> Option<bool> {
        self.heading_ids_for_route(target_route)
            .map(|ids| ids.contains(heading_id))
    }

    pub(crate) fn heading_for_route(
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

    pub(crate) fn routes_refer_to_same_page(&self, left_route: &str, right_route: &str) -> bool {
        match (
            self.source_file_for_route(left_route),
            self.source_file_for_route(right_route),
        ) {
            (Some(left_source), Some(right_source)) => left_source == right_source,
            _ => normalize_route(left_route) == normalize_route(right_route),
        }
    }

    pub(crate) fn static_target_exists(&self, source_file: &str, target: &str) -> bool {
        self.static_target_path(source_file, target).is_some()
    }

    pub(crate) fn static_target_path(&self, source_file: &str, target: &str) -> Option<&str> {
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
