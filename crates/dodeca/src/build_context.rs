use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use ignore::WalkBuilder;

use crate::config::ResolvedSource;
use crate::db::{
    CodeFile, DataFile, Database, QueryStats, SassFile, SourceFile, StaticFile, TemplateFile,
};
use crate::template_paths::{logical_template_path, physical_template_path};
use crate::types::{
    CodeContent, CodePath, DataContent, DataPath, SassContent, SassPath, SassPathRef,
    SourceContent, SourcePath, SourcePathRef, StaticPath, TemplateContent, TemplatePath,
    TemplatePathRef,
};
use crate::vite;

/// Check if a file extension is a supported data file format.
pub fn is_data_file_extension(ext: &str) -> bool {
    let ext_lower = ext.to_lowercase();
    matches!(ext_lower.as_str(), "json" | "toml" | "yaml" | "yml")
}

/// The path-segment form of a mount: `/spec/build/` → `spec/build`, `/` → ``.
fn mount_segment(mount: &str) -> &str {
    mount.trim_matches('/')
}

/// Prefix a source-relative path with a mount segment to form a registry key.
/// The root mount `/` (empty segment) leaves the path unchanged, so a
/// single-source build produces exactly the same keys as before.
pub(crate) fn mounted_key(mount: &str, rel: &str) -> String {
    let seg = mount_segment(mount);
    if seg.is_empty() {
        rel.to_string()
    } else {
        format!("{seg}/{rel}")
    }
}

/// Reverse `mounted_key`: given a mounted source key, find the source that owns
/// it (longest matching mount segment — the root `/` is the fallback) and the
/// path relative to that source's content dir.
pub fn source_for_key(sources: &[ResolvedSource], key: &str) -> Option<(ResolvedSource, String)> {
    let mut best: Option<(&ResolvedSource, String)> = None;
    for source in sources {
        let seg = mount_segment(&source.mount);
        let rel = if seg.is_empty() {
            Some(key.to_string())
        } else {
            key.strip_prefix(seg)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(str::to_string)
        };
        if let Some(rel) = rel {
            let longer = best
                .as_ref()
                .is_none_or(|(b, _)| seg.len() > mount_segment(&b.mount).len());
            if longer {
                best = Some((source, rel));
            }
        }
    }
    best.map(|(source, rel)| (source.clone(), rel))
}

/// A source's `templates/` directory — its sibling of `content_dir`, mirroring
/// the primary's `BuildContext::templates_dir()`. This is where a mounted
/// source keeps its own chrome.
fn source_templates_dir(source: &ResolvedSource) -> Utf8PathBuf {
    source
        .content_dir
        .parent()
        .unwrap_or(&source.content_dir)
        .join("templates")
}

/// The normalized mount of the source that serves `route`: the source whose
/// `mount` is the longest prefix of the route. The root `/` always matches, so
/// this never fails. Routes are mount-prefixed (derived from mounted source
/// keys via `SourcePath::to_route`), so the prefix recovers the owning source.
///
/// Mounts are normalized with a trailing slash (`/wiki/`), but a route may not
/// have one — notably a mounted source's own root section serves at `/wiki`
/// (no trailing slash). We add a trailing slash to the route before matching so
/// `/wiki` still resolves to the `/wiki/` mount, while `/wikilike` does not.
pub fn source_for_route<'a>(route: &str, sources: &'a [ResolvedSource]) -> &'a str {
    let route_slashed = if route.ends_with('/') {
        route.to_string()
    } else {
        format!("{route}/")
    };
    sources
        .iter()
        .filter(|s| route_slashed.starts_with(s.mount.as_str()))
        .max_by_key(|s| s.mount.len())
        .map(|s| s.mount.as_str())
        .unwrap_or("/")
}

/// Narrow the full (mount-prefixed) template registry down to the templates
/// owned by the source serving `route`, re-keyed by their *bare* names.
///
/// The primary (mount `/`) source's templates are a shared base that every
/// source inherits; a mounted source's own templates override by bare name. So
/// a page mounted at `/wiki/` renders with `/wiki/`'s `page.html` if it has one,
/// otherwise the primary's — and `{% extends "base.html" %}` resolves the same
/// way. A mounted source can therefore ship zero templates (pure inherit) or
/// override only the few it customizes.
///
/// A single-source site (one source at `/`) returns the map unchanged, so the
/// common case stays byte-identical to before.
pub fn templates_for_route(
    all: HashMap<String, String>,
    route: &str,
    sources: &[ResolvedSource],
) -> HashMap<String, String> {
    if sources.len() <= 1 {
        return all;
    }
    let owner_mount = source_for_route(route, sources);
    // The primary (mount `/`) source's templates are the shared base every
    // source inherits; a mounted source's own templates override by bare name.
    // So picante/dibs can ship zero templates and still render with the shared
    // chrome, while styx overrides only the few it customizes (codemirror,
    // monaco, its homepage). All re-keyed by bare name; the owner wins.
    let mut out = HashMap::new();
    if owner_mount != "/" {
        for (key, content) in &all {
            if let Some((src, rel)) = source_for_key(sources, key) {
                if src.mount == "/" {
                    out.insert(rel, content.clone());
                }
            }
        }
    }
    for (key, content) in all {
        if let Some((src, rel)) = source_for_key(sources, &key) {
            if src.mount == owner_mount {
                out.insert(rel, content);
            }
        }
    }
    out
}

/// Load markdown source files from every source's content dir, with
/// mount-prefixed keys. Shared by `BuildContext` (build) and the serve path so
/// both produce byte-identical registry keys.
pub fn load_source_files(
    db: &Database,
    roots: &[ResolvedSource],
) -> Result<Vec<(SourcePath, SourceFile)>> {
    let mut out = Vec::new();
    for root in roots {
        let md_files: Vec<Utf8PathBuf> = WalkBuilder::new(&root.content_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();
        for path in md_files {
            let content = fs::read_to_string(&path)?;
            let last_modified = fs::metadata(&path)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let relative = path
                .strip_prefix(&root.content_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());
            let key = mounted_key(&root.mount, &relative);
            let source_path = SourcePath::new(key);
            let source = SourceFile::new(
                db,
                source_path.clone(),
                SourceContent::new(content),
                last_modified,
            )?;
            out.push((source_path, source));
        }
    }
    Ok(out)
}

/// Absolute paths of every file matched by any source's `impls`
/// `include`/`test_include` globs (minus `exclude`), found by walking the
/// project root once. Empty when no source declares `impls`. Shared by
/// [`load_code_files`] (which reads + ingests them) and the file watcher (which
/// needs the abs paths to recognize a changed code file).
pub fn code_file_abs_paths(roots: &[ResolvedSource], project_root: &Utf8Path) -> Vec<Utf8PathBuf> {
    use globset::{Glob, GlobSetBuilder};

    let mut inc = GlobSetBuilder::new();
    let mut exc = GlobSetBuilder::new();
    let mut any = false;
    for root in roots {
        for impl_ in &root.impls {
            // test_include files carry references too (verify-only enforcement
            // is a later coverage refinement), so scan them alongside include.
            for pat in impl_.include.iter().chain(&impl_.test_include) {
                if let Ok(g) = Glob::new(pat) {
                    inc.add(g);
                    any = true;
                }
            }
            for pat in &impl_.exclude {
                if let Ok(g) = Glob::new(pat) {
                    exc.add(g);
                }
            }
        }
    }
    if !any {
        return Vec::new();
    }
    let (Ok(include), Ok(exclude)) = (inc.build(), exc.build()) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in WalkBuilder::new(project_root)
        .build()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let Ok(abs) = Utf8PathBuf::from_path_buf(entry.into_path()) else {
            continue;
        };
        let Ok(rel) = abs.strip_prefix(project_root) else {
            continue;
        };
        if include.is_match(rel.as_std_path()) && !exclude.is_match(rel.as_std_path()) {
            out.push(abs);
        }
    }
    out
}

/// Read every code file matched by the sources' `impls` globs into `CodeFile`
/// inputs keyed by project-root-relative path. Returns empty when no source
/// declares `impls`.
pub fn load_code_files(
    db: &Database,
    roots: &[ResolvedSource],
    project_root: &Utf8Path,
) -> Result<Vec<(crate::types::CodePath, crate::db::CodeFile)>> {
    let mut out = Vec::new();
    for abs in code_file_abs_paths(roots, project_root) {
        let Ok(rel) = abs.strip_prefix(project_root) else {
            continue;
        };
        let content = fs::read_to_string(&abs)?;
        let last_modified = fs::metadata(&abs)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let code_path = CodePath::new(rel.as_str().to_string());
        let file = CodeFile::new(
            db,
            code_path.clone(),
            CodeContent::new(content),
            last_modified,
        )?;
        out.push((code_path, file));
    }
    Ok(out)
}

/// Load template files from every source's `templates/` dir (sibling of its
/// content dir), with mount-prefixed keys. The primary (mount `/`) keeps bare
/// keys (`page.html`); a mounted source gets prefixed keys (`wiki/page.html`).
/// Render-time filtering (`templates_for_route`) strips the prefix back to bare
/// names so a mounted source renders with its own chrome and its
/// `{% extends "base.html" %}` resolves within its own set.
///
/// Shared by `BuildContext` (build) and the `ddc serve` path so both produce
/// byte-identical registry keys — mirroring `load_source_files`.
pub fn load_template_files(
    db: &Database,
    roots: &[ResolvedSource],
) -> Result<Vec<(TemplatePath, TemplateFile)>> {
    let mut out = Vec::new();
    for root in roots {
        let templates_dir = source_templates_dir(root);
        if !templates_dir.exists() {
            continue;
        }
        let files: Vec<Utf8PathBuf> = WalkBuilder::new(&templates_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                Utf8Path::from_path(e.path())
                    .and_then(|path| path.strip_prefix(&templates_dir).ok())
                    .and_then(logical_template_path)
                    .is_some()
            })
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();
        for path in files {
            let content = fs::read_to_string(&path)?;
            let relative = path
                .strip_prefix(&templates_dir)
                .ok()
                .and_then(logical_template_path)
                .unwrap_or_else(|| path.to_string());
            let key = mounted_key(&root.mount, &relative);
            let template_path = TemplatePath::new(key);
            let template =
                TemplateFile::new(db, template_path.clone(), TemplateContent::new(content))?;
            out.push((template_path, template));
        }
    }
    Ok(out)
}

/// Load static files from every NON-primary source's `static/` dir (sibling of
/// its content dir), with mount-prefixed keys (`wiki/style.css`). The primary
/// (mount `/`) `static/` + `dist/` are loaded by the caller; this only adds the
/// mounted sources' assets, so they're served + cache-busted under their mount
/// and a mounted page's source-root-absolute asset refs (`/style.css`) alias to
/// them. Shared by `BuildContext` (build) and the `ddc serve` path so both load
/// the same set — mirroring `load_template_files` / `load_source_files`.
pub fn load_source_static_files(
    db: &Database,
    roots: &[ResolvedSource],
) -> Result<Vec<(StaticPath, StaticFile)>> {
    let mut out = Vec::new();
    for root in roots.iter().skip(1) {
        let parent = root.content_dir.parent().unwrap_or(&root.content_dir);

        // Load every file under `dir` with mount-prefixed keys.
        let mut load_dir = |dir: &Utf8Path| -> Result<()> {
            if !dir.exists() {
                return Ok(());
            }
            let files: Vec<Utf8PathBuf> = WalkBuilder::new(dir)
                .build()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type()
                        .map(|ft| ft.is_file() || (ft.is_symlink() && e.path().is_file()))
                        .unwrap_or(false)
                })
                .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
                .collect();
            for path in files {
                let content = fs::read(&path)?;
                let relative = path
                    .strip_prefix(dir)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|_| path.to_string());
                let key = mounted_key(&root.mount, &relative);
                let static_path = StaticPath::new(key);
                let static_file = StaticFile::new(db, static_path.clone(), content)?;
                out.push((static_path, static_file));
            }
            Ok(())
        };

        // `static/` assets, then the vite `dist/` output (built JS/CSS), each
        // under the source's mount so a mounted page's `/quiz.js` / `/style.css`
        // alias resolves and per-source vite bundles work (see vite_manifest_map).
        load_dir(&parent.join("static"))?;
        let dist = parent.join("dist");
        load_dir(&dist)?;
        // WalkBuilder skips dotfiles, so add the vite manifest explicitly.
        let manifest = dist.join(".vite/manifest.json");
        if manifest.exists() {
            let content = fs::read(&manifest)?;
            let key = mounted_key(&root.mount, ".vite/manifest.json");
            let static_path = StaticPath::new(key);
            let static_file = StaticFile::new(db, static_path.clone(), content)?;
            out.push((static_path, static_file));
        }
    }
    Ok(out)
}

/// Load Sass/SCSS from every NON-primary source's `sass/` dir (sibling of its
/// content dir), with mount-prefixed keys (`styx/main.scss`). The primary
/// (mount `/`) sass is loaded by `BuildContext::load_sass`; this adds the
/// mounted sources' sass so each source compiles its *own* bundle, emitted at
/// `<mount>/main.css` (see `source_css_outputs`). Mirrors
/// `load_source_static_files` / `load_template_files`.
pub fn load_source_sass_files(
    db: &Database,
    roots: &[ResolvedSource],
) -> Result<Vec<(SassPath, SassFile)>> {
    let mut out = Vec::new();
    for root in roots.iter().skip(1) {
        let source_sass = root
            .content_dir
            .parent()
            .unwrap_or(&root.content_dir)
            .join("sass");
        if !source_sass.exists() {
            continue;
        }
        let files: Vec<Utf8PathBuf> = WalkBuilder::new(&source_sass)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "scss" || ext == "sass")
                    .unwrap_or(false)
            })
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();
        for path in files {
            let content = fs::read_to_string(&path)?;
            let relative = path
                .strip_prefix(&source_sass)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());
            let key = mounted_key(&root.mount, &relative);
            let sass_path = SassPath::new(key);
            let sass_content = SassContent::new(content);
            let sass_file = SassFile::new(db, sass_path.clone(), sass_content)?;
            out.push((sass_path, sass_file));
        }
    }
    Ok(out)
}

/// The build context with picante database.
pub struct BuildContext {
    pub db: Arc<Database>,
    /// The primary content dir — the mount-`/` source. Templates, sass, static,
    /// data and the cache are derived from it (one shared chrome for the site).
    pub content_dir: Utf8PathBuf,
    /// Project root (the dir holding `.config/`). `impls` code globs resolve
    /// against this. Defaults to the content dir's parent; set explicitly from
    /// the resolved config's root where available.
    pub project_root: Utf8PathBuf,
    /// All content sources, each with its mount prefix. Markdown is loaded from
    /// every source with mount-prefixed keys; a single-source build has exactly
    /// one entry at mount `/`.
    pub source_roots: Vec<ResolvedSource>,
    pub output_dir: Utf8PathBuf,
    /// Source files keyed by source path.
    pub sources: BTreeMap<SourcePath, SourceFile>,
    /// Template files keyed by template path.
    pub templates: BTreeMap<TemplatePath, TemplateFile>,
    /// Sass/SCSS files keyed by sass path.
    pub sass_files: BTreeMap<SassPath, SassFile>,
    /// Static files keyed by static path.
    pub static_files: BTreeMap<StaticPath, StaticFile>,
    /// Data files keyed by data path.
    pub data_files: BTreeMap<DataPath, DataFile>,
    /// Code files (from source `impls` globs) keyed by project-root-relative
    /// path, scanned for requirement references.
    pub code_files: BTreeMap<CodePath, CodeFile>,
    /// Query statistics, if tracking is enabled.
    pub stats: Option<Arc<QueryStats>>,
}

impl BuildContext {
    pub fn new(content_dir: &Utf8Path, output_dir: &Utf8Path) -> Self {
        Self::with_stats(content_dir, output_dir, None)
    }

    pub fn with_stats(
        content_dir: &Utf8Path,
        output_dir: &Utf8Path,
        stats: Option<Arc<QueryStats>>,
    ) -> Self {
        let db = Arc::new(Database::new(stats.clone()));
        Self {
            db,
            content_dir: content_dir.to_owned(),
            project_root: content_dir.parent().unwrap_or(content_dir).to_owned(),
            source_roots: vec![ResolvedSource {
                name: String::new(),
                mount: "/".to_string(),
                content_dir: content_dir.to_owned(),
                checkout_dir: None,
                git: None,
                repo: None,
                impls: Vec::new(),
            }],
            output_dir: output_dir.to_owned(),
            sources: BTreeMap::new(),
            templates: BTreeMap::new(),
            sass_files: BTreeMap::new(),
            static_files: BTreeMap::new(),
            data_files: BTreeMap::new(),
            code_files: BTreeMap::new(),
            stats,
        }
    }

    /// Set the project root that `impls` code globs resolve against.
    pub fn set_project_root(&mut self, root: impl Into<Utf8PathBuf>) {
        self.project_root = root.into();
    }

    /// Get the database Arc for sharing with render contexts.
    pub fn db_arc(&self) -> Arc<Database> {
        self.db.clone()
    }

    /// Replace the content sources. The first source's content dir becomes the
    /// primary (`content_dir`), from which templates/sass/static/data/cache are
    /// derived. A one-element list at mount `/` is the single-source default.
    pub fn set_source_roots(&mut self, sources: Vec<ResolvedSource>) {
        if let Some(primary) = sources.first() {
            self.content_dir = primary.content_dir.clone();
        }
        self.source_roots = sources;
    }

    /// Get the templates directory, sibling to the content dir.
    pub fn templates_dir(&self) -> Utf8PathBuf {
        self.content_dir
            .parent()
            .unwrap_or(&self.content_dir)
            .join("templates")
    }

    /// Get the Sass directory, sibling to the content dir.
    pub fn sass_dir(&self) -> Utf8PathBuf {
        self.content_dir
            .parent()
            .unwrap_or(&self.content_dir)
            .join("sass")
    }

    /// Get the static directory, sibling to the content dir.
    pub fn static_dir(&self) -> Utf8PathBuf {
        self.content_dir
            .parent()
            .unwrap_or(&self.content_dir)
            .join("static")
    }

    /// Get the dist directory, sibling to the content dir, for generated/build output.
    pub fn dist_dir(&self) -> Utf8PathBuf {
        self.content_dir
            .parent()
            .unwrap_or(&self.content_dir)
            .join("dist")
    }

    /// Get the data directory, sibling to the content dir.
    pub fn data_dir(&self) -> Utf8PathBuf {
        self.content_dir
            .parent()
            .unwrap_or(&self.content_dir)
            .join("data")
    }

    /// Load all source files into the database, from every content source.
    ///
    /// Each source's markdown keys are prefixed by its mount (`spec/build/…`),
    /// so routes, wiki-link slugs, output paths and search all flow prefixed
    /// downstream. The root mount `/` leaves keys unchanged — a single-source
    /// build is byte-identical to before.
    pub fn load_sources(&mut self) -> Result<()> {
        // Clone the roots so we don't hold a borrow of `self` while inserting.
        let roots = self.source_roots.clone();
        for (path, source) in load_source_files(&self.db, &roots)? {
            self.sources.insert(path, source);
        }
        Ok(())
    }

    /// Load every code file matched by a source's `impls` globs into `CodeFile`
    /// inputs, keyed by project-root-relative path.
    pub fn load_code(&mut self) -> Result<()> {
        let roots = self.source_roots.clone();
        let project_root = self.project_root.clone();
        for (path, file) in load_code_files(&self.db, &roots, &project_root)? {
            self.code_files.insert(path, file);
        }
        Ok(())
    }

    /// Load all template files into the database, from every content source.
    ///
    /// Each source's templates are keyed by its mount (`wiki/page.html`); the
    /// primary (mount `/`) keeps bare keys, so a single-source build is
    /// byte-identical to before. See `load_template_files`.
    pub fn load_templates(&mut self) -> Result<()> {
        let roots = self.source_roots.clone();
        for (path, template) in load_template_files(&self.db, &roots)? {
            self.templates.insert(path, template);
        }
        Ok(())
    }

    /// Load all Sass/SCSS files into the database. The primary (mount `/`)
    /// source's `sass/` keeps bare keys (`main.scss`); each non-primary source's
    /// `sass/` is loaded with mount-prefixed keys (`styx/main.scss`) so every
    /// source compiles its own `<mount>/main.css` bundle (see
    /// `source_css_outputs`).
    pub fn load_sass(&mut self) -> Result<()> {
        let sass_dir = self.sass_dir();
        if sass_dir.exists() {
            let sass_files: Vec<Utf8PathBuf> = WalkBuilder::new(&sass_dir)
                .build()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "scss" || ext == "sass")
                        .unwrap_or(false)
                })
                .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
                .collect();

            for path in sass_files {
                let content = fs::read_to_string(&path)?;
                let relative = path
                    .strip_prefix(&sass_dir)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|_| path.to_string());

                let sass_path = SassPath::new(relative);
                let sass_content = SassContent::new(content);
                let sass_file = SassFile::new(&*self.db, sass_path.clone(), sass_content)?;
                self.sass_files.insert(sass_path, sass_file);
            }
        }

        // Per-source sass: each non-primary source's `sass/` dir, mount-prefixed.
        let roots = self.source_roots.clone();
        for (path, file) in load_source_sass_files(&self.db, &roots)? {
            self.sass_files.insert(path, file);
        }

        Ok(())
    }

    /// Load all static files into the database from static/ and dist/, with dist/ taking priority.
    pub fn load_static(&mut self) -> Result<()> {
        let static_dir = self.static_dir();
        let dist_dir = self.dist_dir();

        if static_dir.exists() {
            let static_files: Vec<Utf8PathBuf> = WalkBuilder::new(&static_dir)
                .build()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type()
                        .map(|ft| ft.is_file() || (ft.is_symlink() && e.path().is_file()))
                        .unwrap_or(false)
                })
                .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
                .collect();

            for path in static_files {
                let content = fs::read(&path)?;
                let relative = path
                    .strip_prefix(&static_dir)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|_| path.to_string());

                let static_path = StaticPath::new(relative);
                let static_file = StaticFile::new(&*self.db, static_path.clone(), content)?;
                self.static_files.insert(static_path, static_file);
            }
        }

        if dist_dir.exists() {
            let dist_files: Vec<Utf8PathBuf> = WalkBuilder::new(&dist_dir)
                .build()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type()
                        .map(|ft| ft.is_file() || (ft.is_symlink() && e.path().is_file()))
                        .unwrap_or(false)
                })
                .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
                .collect();

            for path in dist_files {
                let content = fs::read(&path)?;
                let relative = path
                    .strip_prefix(&dist_dir)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|_| path.to_string());

                tracing::trace!(path = %relative, "load_static: loading file from dist");

                let static_path = StaticPath::new(relative);
                let static_file = StaticFile::new(&*self.db, static_path.clone(), content)?;
                self.static_files.insert(static_path, static_file);
            }

            let manifest_path = dist_dir.join(".vite/manifest.json");
            if manifest_path.exists() {
                let content = fs::read(&manifest_path)?;
                tracing::debug!(bytes = content.len(), "loaded vite manifest");
                let static_path = StaticPath::new(".vite/manifest.json".to_string());
                let static_file = StaticFile::new(&*self.db, static_path.clone(), content)?;
                self.static_files.insert(static_path, static_file);
            }
        }

        let project_dir = self.content_dir.parent().unwrap_or(&self.content_dir);
        if vite::has_vite_config(project_dir.as_std_path()) {
            let has_manifest = self
                .static_files
                .contains_key(&StaticPath::new(".vite/manifest.json".to_string()));
            if !has_manifest {
                let dist_dir = self.dist_dir();
                let manifest_path = dist_dir.join(".vite/manifest.json");
                return Err(eyre!(
                    "Vite is configured but manifest not found.\n\n\
                    Expected manifest at: {}\n\n\
                    This usually means one of:\n\
                    1. Vite build hasn't run yet - try `pnpm run build` in {}\n\
                    2. vite.config.ts is missing `build.manifest: true`\n\
                    3. vite.config.ts has a different outDir than 'dist'\n\n\
                    Looked in:\n\
                    - {}\n",
                    manifest_path,
                    project_dir,
                    manifest_path,
                ));
            }
        }

        // Per-source static assets: each non-primary source's `static/` dir,
        // loaded with mount-prefixed keys so its assets land under its mount in
        // the (cache-busted) output and never collide with another source's
        // same-named asset. The primary (mount `/`) is already handled above via
        // `self.static_dir()`.
        let roots = self.source_roots.clone();
        for (path, file) in load_source_static_files(&self.db, &roots)? {
            self.static_files.insert(path, file);
        }

        Ok(())
    }

    /// Load all data files into the database.
    pub fn load_data(&mut self) -> Result<()> {
        let data_dir = self.data_dir();
        if !data_dir.exists() {
            return Ok(());
        }

        let data_files: Vec<Utf8PathBuf> = WalkBuilder::new(&data_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| is_data_file_extension(&ext.to_string_lossy()))
                    .unwrap_or(false)
            })
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();

        for path in data_files {
            let content = fs::read_to_string(&path)?;
            let relative = path
                .strip_prefix(&data_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let data_path = DataPath::new(relative);
            let data_content = DataContent::new(content);
            let data_file = DataFile::new(&*self.db, data_path.clone(), data_content)?;
            self.data_files.insert(data_path, data_file);
        }

        Ok(())
    }

    /// Update a single source file for incremental rebuilds.
    ///
    /// `relative_path` is the mounted key; we resolve it back to the owning
    /// source and its on-disk path before reading.
    pub fn update_source(&mut self, relative_path: &SourcePathRef) -> Result<bool> {
        let full_path = match source_for_key(&self.source_roots, relative_path.as_str()) {
            Some((root, rel)) => root.content_dir.join(rel),
            None => self.content_dir.join(relative_path.as_str()),
        };
        if !full_path.exists() {
            self.sources.remove(relative_path);
            return Ok(true);
        }

        let content = fs::read_to_string(&full_path)?;
        let source_content = SourceContent::new(content);
        let last_modified = fs::metadata(&full_path)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let source_path = SourcePath::new(relative_path.to_string());
        let source = SourceFile::new(
            &*self.db,
            source_path.clone(),
            source_content,
            last_modified,
        )
        .expect("failed to create source file");
        self.sources.insert(source_path, source);

        Ok(true)
    }

    /// Update a single template file for incremental rebuilds.
    ///
    /// `relative_path` is the registry key, which is mount-prefixed for
    /// non-primary sources (`wiki/page.html`); resolve it back to the owning
    /// source's `templates/` dir and source-relative name before reading.
    pub fn update_template(&mut self, relative_path: &TemplatePathRef) -> Result<bool> {
        let key = relative_path.as_str();
        let (templates_dir, rel) = match source_for_key(&self.source_roots, key) {
            Some((src, rel)) => (source_templates_dir(&src), rel),
            None => (self.templates_dir(), key.to_string()),
        };
        let full_path = physical_template_path(&templates_dir, &rel);
        if !full_path.exists() {
            self.templates.remove(relative_path);
            return Ok(true);
        }

        let content = fs::read_to_string(&full_path)?;
        let template_content = TemplateContent::new(content);

        let template_path = TemplatePath::new(relative_path.to_string());
        let template = TemplateFile::new(&*self.db, template_path.clone(), template_content)
            .expect("failed to create template file");
        self.templates.insert(template_path, template);

        Ok(true)
    }

    /// Update a single Sass file for incremental rebuilds.
    pub fn update_sass(&mut self, relative_path: &SassPathRef) -> Result<bool> {
        let sass_dir = self.sass_dir();
        let full_path = sass_dir.join(relative_path.as_str());
        if !full_path.exists() {
            self.sass_files.remove(relative_path);
            return Ok(true);
        }

        let content = fs::read_to_string(&full_path)?;
        let sass_content = SassContent::new(content);

        let sass_path = SassPath::new(relative_path.to_string());
        let sass_file = SassFile::new(&*self.db, sass_path.clone(), sass_content)
            .expect("failed to create sass file");
        self.sass_files.insert(sass_path, sass_file);

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(mount: &str, content_dir: &str) -> ResolvedSource {
        ResolvedSource {
            name: String::new(),
            mount: mount.to_string(),
            content_dir: Utf8PathBuf::from(content_dir),
            checkout_dir: None,
            git: None,
            repo: None,
            impls: Vec::new(),
        }
    }

    #[test]
    fn mounted_key_leaves_root_unchanged() {
        assert_eq!(mounted_key("/", "guide/intro.md"), "guide/intro.md");
        assert_eq!(mounted_key("", "x.md"), "x.md");
    }

    #[test]
    fn mounted_key_prefixes_non_root() {
        assert_eq!(mounted_key("/spec/build/", "exec.md"), "spec/build/exec.md");
        assert_eq!(mounted_key("spec/build", "exec.md"), "spec/build/exec.md");
    }

    #[test]
    fn source_for_key_reverses_root() {
        let sources = vec![src("/", "/proj/content")];
        let (s, rel) = source_for_key(&sources, "guide/intro.md").unwrap();
        assert_eq!(s.content_dir, Utf8PathBuf::from("/proj/content"));
        assert_eq!(rel, "guide/intro.md");
    }

    #[test]
    fn source_for_key_picks_longest_mount() {
        // Root and a nested source both "match" a nested key; the nested one
        // (longer segment) must win, so the file resolves under the right repo.
        let sources = vec![
            src("/", "/proj/content"),
            src("/spec/build", "/proj/../vixen/docs/content"),
        ];
        let (s, rel) = source_for_key(&sources, "spec/build/exec.md").unwrap();
        assert_eq!(
            s.content_dir,
            Utf8PathBuf::from("/proj/../vixen/docs/content")
        );
        assert_eq!(rel, "exec.md");

        // A key outside any nested mount falls to root.
        let (s, rel) = source_for_key(&sources, "identity.md").unwrap();
        assert_eq!(s.content_dir, Utf8PathBuf::from("/proj/content"));
        assert_eq!(rel, "identity.md");
    }

    #[test]
    fn source_for_route_picks_longest_mount() {
        let sources = vec![
            src("/", "/proj/kb/content"),
            src("/wiki/", "/proj/wiki/content"),
        ];
        // Root route → primary.
        assert_eq!(source_for_route("/hello/", &sources), "/");
        // The wiki home and its pages → the wiki mount (longest prefix wins).
        assert_eq!(source_for_route("/wiki/", &sources), "/wiki/");
        assert_eq!(source_for_route("/wiki/note/", &sources), "/wiki/");
        // The mounted source's own root section serves WITHOUT a trailing slash
        // (`/wiki`), and its pages may too (`/wiki/note`) — both must resolve to
        // the `/wiki/` mount, not fall back to the primary.
        assert_eq!(source_for_route("/wiki", &sources), "/wiki/");
        assert_eq!(source_for_route("/wiki/note", &sources), "/wiki/");
        // A lookalike that isn't actually under the mount stays on the primary
        // (the trailing slash on `/wiki/` is the boundary).
        assert_eq!(source_for_route("/wikilike/", &sources), "/");
        assert_eq!(source_for_route("/wikilike", &sources), "/");
    }

    #[test]
    fn templates_for_route_inherits_primary_with_overrides() {
        let sources = vec![
            src("/", "/proj/kb/content"),
            src("/wiki/", "/proj/wiki/content"),
        ];
        let mut all = HashMap::new();
        all.insert("page.html".to_string(), "KB-PAGE".to_string());
        all.insert("base.html".to_string(), "KB-BASE".to_string());
        all.insert("macros.html".to_string(), "KB-MACROS".to_string());
        // The wiki overrides page/base but ships no macros of its own.
        all.insert("wiki/page.html".to_string(), "WIKI-PAGE".to_string());
        all.insert("wiki/base.html".to_string(), "WIKI-BASE".to_string());

        // A primary-mounted route sees only the primary's templates, bare-keyed.
        let kb = templates_for_route(all.clone(), "/hello/", &sources);
        assert_eq!(kb.get("page.html").map(String::as_str), Some("KB-PAGE"));
        assert_eq!(kb.get("base.html").map(String::as_str), Some("KB-BASE"));
        assert_eq!(kb.get("macros.html").map(String::as_str), Some("KB-MACROS"));
        assert_eq!(kb.len(), 3);

        // A wiki-mounted route inherits the primary's templates as a base, with
        // its own overriding by bare name: page/base are the wiki's, macros is
        // inherited from the primary.
        let wiki = templates_for_route(all.clone(), "/wiki/note/", &sources);
        assert_eq!(wiki.get("page.html").map(String::as_str), Some("WIKI-PAGE"));
        assert_eq!(wiki.get("base.html").map(String::as_str), Some("WIKI-BASE"));
        assert_eq!(
            wiki.get("macros.html").map(String::as_str),
            Some("KB-MACROS")
        );
        assert_eq!(wiki.len(), 3);
    }

    #[test]
    fn templates_for_route_single_source_passthrough() {
        let sources = vec![src("/", "/proj/content")];
        let mut all = HashMap::new();
        all.insert("page.html".to_string(), "PAGE".to_string());
        all.insert("base.html".to_string(), "BASE".to_string());
        // One source at `/` ⇒ identical to the flat behaviour.
        assert_eq!(templates_for_route(all.clone(), "/hello/", &sources), all);
    }
}
