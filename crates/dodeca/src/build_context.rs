use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use ignore::WalkBuilder;

use crate::config::ResolvedSource;
use crate::db::{DataFile, Database, QueryStats, SassFile, SourceFile, StaticFile, TemplateFile};
use crate::template_paths::{logical_template_path, physical_template_path};
use crate::types::{
    DataContent, DataPath, SassContent, SassPath, SassPathRef, SourceContent, SourcePath,
    SourcePathRef, StaticPath, TemplateContent, TemplatePath, TemplatePathRef,
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
fn mounted_key(mount: &str, rel: &str) -> String {
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
fn source_for_key(sources: &[ResolvedSource], key: &str) -> Option<(ResolvedSource, String)> {
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

/// The build context with picante database.
pub struct BuildContext {
    pub db: Arc<Database>,
    /// The primary content dir — the mount-`/` source. Templates, sass, static,
    /// data and the cache are derived from it (one shared chrome for the site).
    pub content_dir: Utf8PathBuf,
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
            source_roots: vec![ResolvedSource {
                mount: "/".to_string(),
                content_dir: content_dir.to_owned(),
            }],
            output_dir: output_dir.to_owned(),
            sources: BTreeMap::new(),
            templates: BTreeMap::new(),
            sass_files: BTreeMap::new(),
            static_files: BTreeMap::new(),
            data_files: BTreeMap::new(),
            stats,
        }
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
        for root in &roots {
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
                let source_content = SourceContent::new(content);
                let source = SourceFile::new(
                    &*self.db,
                    source_path.clone(),
                    source_content,
                    last_modified,
                )?;
                self.sources.insert(source_path, source);
            }
        }

        Ok(())
    }

    /// Load all template files into the database.
    pub fn load_templates(&mut self) -> Result<()> {
        let templates_dir = self.templates_dir();
        if !templates_dir.exists() {
            return Ok(());
        }

        let template_files: Vec<Utf8PathBuf> = WalkBuilder::new(&templates_dir)
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

        for path in template_files {
            let content = fs::read_to_string(&path)?;
            let relative = path
                .strip_prefix(&templates_dir)
                .ok()
                .and_then(logical_template_path)
                .unwrap_or_else(|| path.to_string());

            let template_path = TemplatePath::new(relative);
            let template_content = TemplateContent::new(content);
            let template = TemplateFile::new(&*self.db, template_path.clone(), template_content)?;
            self.templates.insert(template_path, template);
        }

        Ok(())
    }

    /// Load all Sass/SCSS files into the database.
    pub fn load_sass(&mut self) -> Result<()> {
        let sass_dir = self.sass_dir();
        if !sass_dir.exists() {
            return Ok(());
        }

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
        for root in roots.iter().skip(1) {
            let source_static = root
                .content_dir
                .parent()
                .unwrap_or(&root.content_dir)
                .join("static");
            if !source_static.exists() {
                continue;
            }
            let files: Vec<Utf8PathBuf> = WalkBuilder::new(&source_static)
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
                    .strip_prefix(&source_static)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|_| path.to_string());
                let key = mounted_key(&root.mount, &relative);
                let static_path = StaticPath::new(key);
                let static_file = StaticFile::new(&*self.db, static_path.clone(), content)?;
                self.static_files.insert(static_path, static_file);
            }
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
    pub fn update_template(&mut self, relative_path: &TemplatePathRef) -> Result<bool> {
        let templates_dir = self.templates_dir();
        let full_path = physical_template_path(&templates_dir, relative_path.as_str());
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
            mount: mount.to_string(),
            content_dir: Utf8PathBuf::from(content_dir),
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
}
