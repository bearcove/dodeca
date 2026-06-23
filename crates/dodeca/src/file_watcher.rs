//! File watcher for live reload in serve mode
//!
//! Handles:
//! - File creation, modification, and deletion
//! - File moves/renames (treating as delete + create)
//! - New directory detection (adds to watcher)

use camino::{Utf8Path, Utf8PathBuf};
use notify::{
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, ModifyKind, RenameMode},
};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Type alias for the watcher handle
pub type WatcherHandle = Arc<Mutex<RecommendedWatcher>>;
/// Type alias for the watcher event receiver
pub type WatcherReceiver = std::sync::mpsc::Receiver<notify::Result<notify::Event>>;

/// Configuration for the file watcher.
///
/// `content_dir`/`static_dir`/… describe the **primary** source (mount `/`).
/// `sources`, when non-empty, lists every mounted source: content and static
/// are watched per-source and their keys are mount-prefixed (mirroring
/// `BuildContext`), while templates/sass/dist/data stay primary-derived (one
/// shared chrome). Empty `sources` = legacy single-source behavior.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    pub content_dir: Utf8PathBuf,
    pub templates_dir: Utf8PathBuf,
    pub sass_dir: Utf8PathBuf,
    pub static_dir: Utf8PathBuf,
    pub dist_dir: Utf8PathBuf,
    pub data_dir: Utf8PathBuf,
    /// All mounted content sources (empty ⇒ single-source/legacy).
    pub sources: Vec<crate::config::ResolvedSource>,
    /// Absolute path to the project config file (`.config/dodeca.styx`), if any.
    /// A change to it triggers a full config re-resolution + registry reload.
    pub config_file: Option<Utf8PathBuf>,
    /// Absolute paths of files pulled in by `include` shortcodes. They live
    /// outside every content/asset tree, so they're tracked here explicitly so a
    /// change to one categorizes as [`PathCategory::Include`] and re-renders the
    /// pages that embed it. Grows as includes are discovered at render time.
    pub included_files: std::collections::HashSet<Utf8PathBuf>,
}

/// Among `sources`, the one whose `dir_of` is the longest path-prefix of `path`,
/// with the path relative to that dir. Used to attribute a changed file to its
/// owning source for content and static trees.
fn longest_source_match<'a>(
    sources: &'a [crate::config::ResolvedSource],
    path: &Utf8Path,
    dir_of: impl Fn(&crate::config::ResolvedSource) -> Option<Utf8PathBuf>,
) -> Option<(&'a crate::config::ResolvedSource, Utf8PathBuf)> {
    let mut best: Option<(&crate::config::ResolvedSource, Utf8PathBuf, usize)> = None;
    for source in sources {
        let Some(dir) = dir_of(source) else { continue };
        if let Ok(rel) = path.strip_prefix(&dir) {
            let len = dir.as_str().len();
            if best.as_ref().is_none_or(|(_, _, bl)| len > *bl) {
                best = Some((source, rel.to_owned(), len));
            }
        }
    }
    best.map(|(source, rel, _)| (source, rel))
}

/// The static dir of a source — sibling of its content dir.
fn source_static_dir(source: &crate::config::ResolvedSource) -> Option<Utf8PathBuf> {
    source.content_dir.parent().map(|p| p.join("static"))
}

/// The templates dir of a source — sibling of its content dir.
fn source_templates_dir(source: &crate::config::ResolvedSource) -> Option<Utf8PathBuf> {
    source.content_dir.parent().map(|p| p.join("templates"))
}

/// The sass dir of a source — sibling of its content dir.
fn source_sass_dir(source: &crate::config::ResolvedSource) -> Option<Utf8PathBuf> {
    source.content_dir.parent().map(|p| p.join("sass"))
}

/// Processed file event ready for the server to handle
#[derive(Debug, Clone)]
pub enum FileEvent {
    /// File was created or modified - reload its content
    Changed(Utf8PathBuf),
    /// File was deleted - remove from registry
    Removed(Utf8PathBuf),
    /// New directory was created - already added to watcher
    DirectoryCreated(Utf8PathBuf),
}

/// Recursively scan a directory and return file change events.
/// Used to catch files created before the watcher was fully set up.
pub fn scan_directory_recursive(dir: &Path, config: &WatcherConfig) -> Vec<FileEvent> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_file()) {
            if should_watch_path(&path, config) {
                if let Ok(utf8) = Utf8PathBuf::from_path_buf(path) {
                    out.push(FileEvent::Changed(utf8));
                }
            }
        } else if entry.file_type().is_ok_and(|t| t.is_dir()) {
            out.extend(scan_directory_recursive(&path, config));
        }
    }

    out
}

/// Categorizes a path by which directory it belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCategory {
    Content,
    Template,
    Sass,
    Static,
    /// Dist directory (generated/build output) - takes priority over Static
    Dist,
    Data,
    /// The project config file (`.config/dodeca.styx`). A change re-resolves the
    /// config and reloads every source's registries in place.
    Config,
    /// A file pulled in by an `include` shortcode. A change re-reads it into the
    /// include registry, re-rendering the pages that embed it.
    Include,
    Unknown,
}

impl WatcherConfig {
    /// The source-relative `(mount, rel)` for a content-tree path: the owning
    /// source's mount and the path under its content dir. Falls back to the
    /// primary `content_dir` (mount `/`) when `sources` is empty.
    fn content_match(&self, path: &Utf8Path) -> Option<(String, Utf8PathBuf)> {
        if self.sources.is_empty() {
            return path
                .strip_prefix(&self.content_dir)
                .ok()
                .map(|rel| ("/".to_string(), rel.to_owned()));
        }
        longest_source_match(&self.sources, path, |s| Some(s.content_dir.clone()))
            .map(|(s, rel)| (s.mount.clone(), rel))
    }

    /// The source-relative `(mount, rel)` for a static-tree path.
    fn static_match(&self, path: &Utf8Path) -> Option<(String, Utf8PathBuf)> {
        if self.sources.is_empty() {
            return path
                .strip_prefix(&self.static_dir)
                .ok()
                .map(|rel| ("/".to_string(), rel.to_owned()));
        }
        longest_source_match(&self.sources, path, source_static_dir)
            .map(|(s, rel)| (s.mount.clone(), rel))
    }

    /// The source-relative `(mount, rel)` for a template-tree path — primary or
    /// any mounted source's own `templates/` dir.
    fn template_match(&self, path: &Utf8Path) -> Option<(String, Utf8PathBuf)> {
        if self.sources.is_empty() {
            return path
                .strip_prefix(&self.templates_dir)
                .ok()
                .map(|rel| ("/".to_string(), rel.to_owned()));
        }
        longest_source_match(&self.sources, path, source_templates_dir)
            .map(|(s, rel)| (s.mount.clone(), rel))
    }

    /// The source-relative `(mount, rel)` for a sass-tree path — primary or any
    /// mounted source's own `sass/` dir.
    fn sass_match(&self, path: &Utf8Path) -> Option<(String, Utf8PathBuf)> {
        if self.sources.is_empty() {
            return path
                .strip_prefix(&self.sass_dir)
                .ok()
                .map(|rel| ("/".to_string(), rel.to_owned()));
        }
        longest_source_match(&self.sources, path, source_sass_dir)
            .map(|(s, rel)| (s.mount.clone(), rel))
    }

    /// Categorize a path by which watched directory it belongs to.
    pub fn categorize(&self, path: &Utf8Path) -> PathCategory {
        if self.config_file.as_deref() == Some(path) {
            PathCategory::Config
        } else if self.included_files.contains(path) {
            PathCategory::Include
        } else if self.content_match(path).is_some() {
            PathCategory::Content
        } else if self.template_match(path).is_some() {
            PathCategory::Template
        } else if self.sass_match(path).is_some() {
            PathCategory::Sass
        } else if path.starts_with(&self.dist_dir) {
            PathCategory::Dist
        } else if self.static_match(path).is_some() {
            PathCategory::Static
        } else if path.starts_with(&self.data_dir) {
            PathCategory::Data
        } else {
            PathCategory::Unknown
        }
    }

    /// Get the registry key for a file within its category. Content and static
    /// keys are mount-prefixed (`spec/build/…`) so they match `BuildContext`.
    pub fn relative_path(&self, path: &Utf8Path) -> Option<Utf8PathBuf> {
        match self.categorize(path) {
            PathCategory::Content => self
                .content_match(path)
                .map(|(mount, rel)| crate::build_context::mounted_key(&mount, rel.as_str()).into()),
            PathCategory::Static => self
                .static_match(path)
                .map(|(mount, rel)| crate::build_context::mounted_key(&mount, rel.as_str()).into()),
            PathCategory::Template => self
                .template_match(path)
                .map(|(mount, rel)| crate::build_context::mounted_key(&mount, rel.as_str()).into()),
            PathCategory::Sass => self
                .sass_match(path)
                .map(|(mount, rel)| crate::build_context::mounted_key(&mount, rel.as_str()).into()),
            PathCategory::Dist => path.strip_prefix(&self.dist_dir).ok().map(|p| p.to_owned()),
            PathCategory::Data => path.strip_prefix(&self.data_dir).ok().map(|p| p.to_owned()),
            PathCategory::Config | PathCategory::Include | PathCategory::Unknown => None,
        }
    }

    /// Every directory the watcher should recursively watch: the primary
    /// templates/sass/static/dist/data, plus every source's content, static,
    /// templates, sass and dist dirs (so edits to a mounted source's own chrome
    /// hot-reload). De-duplicated; non-existent dirs are fine (best-effort).
    pub fn all_watch_dirs(&self) -> Vec<Utf8PathBuf> {
        let mut dirs = vec![
            self.content_dir.clone(),
            self.templates_dir.clone(),
            self.sass_dir.clone(),
            self.static_dir.clone(),
            self.dist_dir.clone(),
            self.data_dir.clone(),
        ];
        for source in &self.sources {
            dirs.push(source.content_dir.clone());
            for dir in [
                source_static_dir(source),
                source_templates_dir(source),
                source_sass_dir(source),
                source.content_dir.parent().map(|p| p.join("dist")),
            ]
            .into_iter()
            .flatten()
            {
                dirs.push(dir);
            }
        }
        // Watch the config file's directory (`.config/`) so edits to
        // `dodeca.styx` fire — it lives outside every content/asset tree.
        if let Some(cfg) = &self.config_file
            && let Some(parent) = cfg.parent()
        {
            dirs.push(parent.to_owned());
        }
        dirs.sort();
        dirs.dedup();
        dirs
    }
}

/// Check if a path should be watched based on extension or location
fn should_watch_path(path: &Path, config: &WatcherConfig) -> bool {
    let path_str = path.to_string_lossy();

    // Skip temp files
    if path_str.contains(".tmp.") || path_str.ends_with("~") || path_str.contains(".swp") {
        return false;
    }

    // Check if it's in a watched directory (static/data watch all files)
    let utf8_path = match Utf8Path::from_path(path) {
        Some(p) => p,
        None => return false,
    };

    match config.categorize(utf8_path) {
        PathCategory::Config | PathCategory::Include => true,
        PathCategory::Static | PathCategory::Dist | PathCategory::Data => true,
        PathCategory::Content | PathCategory::Template | PathCategory::Sass => {
            // For these, check extension
            path.extension()
                .map(|e| {
                    let e = e.to_string_lossy();
                    matches!(
                        e.as_ref(),
                        "md" | "scss" | "css" | "html" | "json" | "toml" | "yaml" | "yml"
                    )
                })
                .unwrap_or(false)
        }
        PathCategory::Unknown => false,
    }
}

/// Process a notify event into our FileEvent type
/// Returns a list of events to process
pub fn process_notify_event(
    event: notify::Event,
    config: &WatcherConfig,
    watcher: &Arc<Mutex<RecommendedWatcher>>,
) -> Vec<FileEvent> {
    tracing::debug!(
        event_kind = ?event.kind,
        paths = ?event.paths,
        "file_watcher: received notify event"
    );

    let mut events = Vec::new();

    match event.kind {
        // File/directory created
        EventKind::Create(create_kind) => {
            for path in &event.paths {
                // Check if it's a directory
                if matches!(create_kind, CreateKind::Folder) || path.is_dir() {
                    // Add new directory to watcher
                    if let Ok(mut w) = watcher.lock() {
                        if let Err(e) = w.watch(path, RecursiveMode::Recursive) {
                            tracing::warn!("Failed to watch new directory {:?}: {}", path, e);
                        } else {
                            tracing::debug!("Now watching new directory: {:?}", path);
                            if let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone()) {
                                events.push(FileEvent::DirectoryCreated(utf8));
                            }
                        }
                    }
                } else if should_watch_path(path, config)
                    && let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone())
                {
                    tracing::debug!(path = %utf8, "file_watcher: file created");
                    events.push(FileEvent::Changed(utf8));
                }
            }
        }

        // File modified
        EventKind::Modify(ModifyKind::Data(_)) | EventKind::Modify(ModifyKind::Any) => {
            for path in &event.paths {
                if should_watch_path(path, config)
                    && let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone())
                {
                    tracing::debug!(path = %utf8, "file_watcher: file modified");
                    events.push(FileEvent::Changed(utf8));
                }
            }
        }

        // File renamed/moved
        EventKind::Modify(ModifyKind::Name(rename_mode)) => {
            match rename_mode {
                RenameMode::From => {
                    // Old path - treat as deletion
                    for path in &event.paths {
                        if should_watch_path(path, config)
                            && let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone())
                        {
                            tracing::debug!(path = %utf8, "file_watcher: file renamed from (removed)");
                            events.push(FileEvent::Removed(utf8));
                        }
                    }
                }
                RenameMode::To => {
                    // New path - treat as creation
                    for path in &event.paths {
                        if path.is_dir() {
                            // New directory from rename
                            if let Ok(mut w) = watcher.lock() {
                                let _ = w.watch(path, RecursiveMode::Recursive);
                                if let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone()) {
                                    events.push(FileEvent::DirectoryCreated(utf8));
                                }
                            }
                        } else if should_watch_path(path, config)
                            && let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone())
                        {
                            events.push(FileEvent::Changed(utf8));
                        }
                    }
                }
                RenameMode::Any => {
                    // FSEvents (macOS) sends Any for both old and new paths
                    // Check if file exists to determine if it's a creation or deletion
                    for path in &event.paths {
                        if path.exists() {
                            // File exists at this path - it's the destination (new location)
                            if path.is_dir() {
                                if let Ok(mut w) = watcher.lock() {
                                    let _ = w.watch(path, RecursiveMode::Recursive);
                                    if let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone()) {
                                        events.push(FileEvent::DirectoryCreated(utf8));
                                    }
                                }
                            } else if should_watch_path(path, config)
                                && let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone())
                            {
                                events.push(FileEvent::Changed(utf8));
                            }
                        } else {
                            // File doesn't exist at this path - it's the source (old location)
                            if should_watch_path(path, config)
                                && let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone())
                            {
                                events.push(FileEvent::Removed(utf8));
                            }
                        }
                    }
                }
                // Both paths provided - first is From, second is To
                RenameMode::Both if event.paths.len() >= 2 => {
                    let from_path = &event.paths[0];
                    let to_path = &event.paths[1];

                    if should_watch_path(from_path, config)
                        && let Ok(utf8) = Utf8PathBuf::from_path_buf(from_path.clone())
                    {
                        events.push(FileEvent::Removed(utf8));
                    }

                    if to_path.is_dir() {
                        if let Ok(mut w) = watcher.lock() {
                            let _ = w.watch(to_path, RecursiveMode::Recursive);
                            if let Ok(utf8) = Utf8PathBuf::from_path_buf(to_path.clone()) {
                                events.push(FileEvent::DirectoryCreated(utf8));
                            }
                        }
                    } else if should_watch_path(to_path, config)
                        && let Ok(utf8) = Utf8PathBuf::from_path_buf(to_path.clone())
                    {
                        events.push(FileEvent::Changed(utf8));
                    }
                }
                _ => {}
            }
        }

        // File/directory removed
        EventKind::Remove(_) => {
            for path in &event.paths {
                if should_watch_path(path, config)
                    && let Ok(utf8) = Utf8PathBuf::from_path_buf(path.clone())
                {
                    tracing::debug!(path = %utf8, "file_watcher: file removed");
                    events.push(FileEvent::Removed(utf8));
                }
            }
        }

        _ => {
            tracing::debug!(event_kind = ?event.kind, "file_watcher: ignoring event kind");
        }
    }

    if !events.is_empty() {
        tracing::debug!(count = events.len(), events = ?events, "file_watcher: emitting events");
    }

    events
}

/// Create and configure the file watcher
pub fn create_watcher(config: &WatcherConfig) -> eyre::Result<(WatcherHandle, WatcherReceiver)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;

    let watcher = Arc::new(Mutex::new(watcher));

    // Watch every directory across all sources (content + static per source,
    // plus the primary templates/sass/dist/data). Best-effort: a missing dir
    // (e.g. an un-checked-out source) is simply skipped.
    {
        let mut w = watcher.lock().unwrap();
        for dir in config.all_watch_dirs() {
            if dir.exists() {
                w.watch(dir.as_std_path(), RecursiveMode::Recursive)?;
            }
        }
    }

    Ok((watcher, rx))
}

/// Add directories to an existing watcher (best-effort, recursive). Re-watching
/// an already-watched dir is a no-op. Used by config hot-reload to start
/// watching a newly-added source's content/chrome dirs.
pub fn watch_dirs(watcher: &WatcherHandle, dirs: &[Utf8PathBuf]) {
    let Ok(mut w) = watcher.lock() else { return };
    for dir in dirs {
        if dir.exists()
            && let Err(e) = w.watch(dir.as_std_path(), RecursiveMode::Recursive)
        {
            tracing::debug!(dir = %dir, error = %e, "watch_dirs: failed to watch");
        }
    }
}

/// Watch the *parent directory* (non-recursively) of each given file. Used for
/// files pulled in by `include` shortcodes: the macOS notify backend (FSEvents)
/// is directory-based, so watching the parent — not the file itself — is what
/// actually delivers change events. Non-recursive keeps it from descending into
/// sibling `target/`, `node_modules/`, etc. `categorize` (via `included_files`)
/// then narrows the resulting events to the specific included files.
pub fn watch_include_files(watcher: &WatcherHandle, files: &[Utf8PathBuf]) {
    let Ok(mut w) = watcher.lock() else { return };
    let mut seen = std::collections::HashSet::new();
    for file in files {
        let Some(parent) = file.parent() else {
            continue;
        };
        if !seen.insert(parent.to_owned()) || !parent.exists() {
            continue;
        }
        match w.watch(parent.as_std_path(), RecursiveMode::NonRecursive) {
            Ok(()) => tracing::debug!(dir = %parent, "watch_include_files: watching parent"),
            Err(e) => tracing::warn!(dir = %parent, error = %e, "watch_include_files: failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::RemoveKind;
    use std::path::PathBuf;

    fn test_config(base: &Utf8Path) -> WatcherConfig {
        WatcherConfig {
            content_dir: base.join("content"),
            templates_dir: base.join("templates"),
            sass_dir: base.join("sass"),
            static_dir: base.join("static"),
            dist_dir: base.join("dist"),
            data_dir: base.join("data"),
            sources: vec![],
            config_file: Some(base.join(".config/dodeca.styx")),
            included_files: Default::default(),
        }
    }

    fn src(name: &str, mount: &str, content_dir: &str) -> crate::config::ResolvedSource {
        crate::config::ResolvedSource {
            name: name.to_string(),
            mount: mount.to_string(),
            content_dir: Utf8PathBuf::from(content_dir),
            checkout_dir: None,
            git: None,
        }
    }

    /// A two-source config: primary `kb` at `/proj/content`, `build` mounted at
    /// `/spec/build` from a sibling checkout `/proj/spec/content`.
    fn multi_source_config() -> WatcherConfig {
        WatcherConfig {
            content_dir: Utf8PathBuf::from("/proj/content"),
            templates_dir: Utf8PathBuf::from("/proj/templates"),
            sass_dir: Utf8PathBuf::from("/proj/sass"),
            static_dir: Utf8PathBuf::from("/proj/static"),
            dist_dir: Utf8PathBuf::from("/proj/dist"),
            data_dir: Utf8PathBuf::from("/proj/data"),
            sources: vec![
                src("kb", "/", "/proj/content"),
                src("build", "/spec/build", "/proj/spec/content"),
            ],
            config_file: Some(Utf8PathBuf::from("/proj/.config/dodeca.styx")),
            included_files: Default::default(),
        }
    }

    #[test]
    fn config_file_is_categorized_and_watched() {
        let c = multi_source_config();
        // The config file itself categorizes as Config (not Content/Unknown).
        assert_eq!(
            c.categorize(Utf8Path::new("/proj/.config/dodeca.styx")),
            PathCategory::Config
        );
        // It maps to no registry key (a config change triggers a full reload).
        assert_eq!(
            c.relative_path(Utf8Path::new("/proj/.config/dodeca.styx")),
            None
        );
        // Its directory is in the watch set so notify fires on edits.
        assert!(
            c.all_watch_dirs()
                .contains(&Utf8PathBuf::from("/proj/.config"))
        );
    }

    #[test]
    fn multi_source_content_keys_are_mount_prefixed() {
        let c = multi_source_config();
        // Primary (mount /): unchanged key.
        assert_eq!(
            c.categorize(Utf8Path::new("/proj/content/guide/a.md")),
            PathCategory::Content
        );
        assert_eq!(
            c.relative_path(Utf8Path::new("/proj/content/guide/a.md"))
                .unwrap(),
            Utf8PathBuf::from("guide/a.md")
        );
        // Mounted source: key prefixed by its mount.
        assert_eq!(
            c.categorize(Utf8Path::new("/proj/spec/content/exec.md")),
            PathCategory::Content
        );
        assert_eq!(
            c.relative_path(Utf8Path::new("/proj/spec/content/exec.md"))
                .unwrap(),
            Utf8PathBuf::from("spec/build/exec.md")
        );
    }

    #[test]
    fn multi_source_static_keys_are_mount_prefixed() {
        let c = multi_source_config();
        assert_eq!(
            c.categorize(Utf8Path::new("/proj/spec/static/img/logo.png")),
            PathCategory::Static
        );
        assert_eq!(
            c.relative_path(Utf8Path::new("/proj/spec/static/img/logo.png"))
                .unwrap(),
            Utf8PathBuf::from("spec/build/img/logo.png")
        );
    }

    #[test]
    fn multi_source_templates_stay_primary() {
        let c = multi_source_config();
        assert_eq!(
            c.categorize(Utf8Path::new("/proj/templates/base.html")),
            PathCategory::Template
        );
        assert_eq!(
            c.relative_path(Utf8Path::new("/proj/templates/base.html"))
                .unwrap(),
            Utf8PathBuf::from("base.html")
        );
    }

    #[test]
    fn all_watch_dirs_covers_every_source_checkout() {
        let dirs = multi_source_config().all_watch_dirs();
        assert!(dirs.contains(&Utf8PathBuf::from("/proj/spec/content")));
        assert!(dirs.contains(&Utf8PathBuf::from("/proj/spec/static")));
        assert!(dirs.contains(&Utf8PathBuf::from("/proj/templates")));
    }

    #[test]
    fn test_categorize_paths() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        assert_eq!(
            config.categorize(Utf8Path::new("/project/content/page.md")),
            PathCategory::Content
        );
        assert_eq!(
            config.categorize(Utf8Path::new("/project/templates/base.html")),
            PathCategory::Template
        );
        assert_eq!(
            config.categorize(Utf8Path::new("/project/sass/main.scss")),
            PathCategory::Sass
        );
        assert_eq!(
            config.categorize(Utf8Path::new("/project/static/image.png")),
            PathCategory::Static
        );
        assert_eq!(
            config.categorize(Utf8Path::new("/project/dist/main.js")),
            PathCategory::Dist
        );
        assert_eq!(
            config.categorize(Utf8Path::new("/project/data/config.toml")),
            PathCategory::Data
        );
        assert_eq!(
            config.categorize(Utf8Path::new("/other/file.txt")),
            PathCategory::Unknown
        );
    }

    #[test]
    fn test_relative_path() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        assert_eq!(
            config.relative_path(Utf8Path::new("/project/content/docs/page.md")),
            Some(Utf8PathBuf::from("docs/page.md"))
        );
        assert_eq!(
            config.relative_path(Utf8Path::new("/project/static/fonts/Inter.woff2")),
            Some(Utf8PathBuf::from("fonts/Inter.woff2"))
        );
        assert_eq!(
            config.relative_path(Utf8Path::new("/project/dist/assets/main.js")),
            Some(Utf8PathBuf::from("assets/main.js"))
        );
        assert_eq!(config.relative_path(Utf8Path::new("/other/file.txt")), None);
    }

    #[test]
    fn test_should_watch_content_files() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        // Content files with known extensions should be watched
        assert!(should_watch_path(
            Path::new("/project/content/page.md"),
            &config
        ));
        assert!(should_watch_path(
            Path::new("/project/templates/base.html"),
            &config
        ));
        assert!(should_watch_path(
            Path::new("/project/sass/main.scss"),
            &config
        ));

        // Unknown extensions in content/templates/sass should not be watched
        assert!(!should_watch_path(
            Path::new("/project/content/notes.txt"),
            &config
        ));
    }

    #[test]
    fn test_should_watch_static_files() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        // All static files should be watched regardless of extension
        assert!(should_watch_path(
            Path::new("/project/static/image.png"),
            &config
        ));
        assert!(should_watch_path(
            Path::new("/project/static/fonts/Inter.woff2"),
            &config
        ));
        assert!(should_watch_path(
            Path::new("/project/static/random.xyz"),
            &config
        ));
    }

    #[test]
    fn test_should_ignore_temp_files() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        assert!(!should_watch_path(
            Path::new("/project/content/page.md~"),
            &config
        ));
        assert!(!should_watch_path(
            Path::new("/project/content/.page.md.tmp.12345"),
            &config
        ));
        assert!(!should_watch_path(
            Path::new("/project/content/.page.md.swp"),
            &config
        ));
    }

    #[test]
    fn test_process_create_event() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        // Create a temporary watcher for testing
        let (tx, _rx) = std::sync::mpsc::channel();
        let watcher = notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
            let _ = tx.send(());
        })
        .unwrap();
        let watcher = Arc::new(Mutex::new(watcher));

        let event = notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![PathBuf::from("/project/content/new-page.md")],
            attrs: Default::default(),
        };

        let events = process_notify_event(event, &config, &watcher);
        assert_eq!(events.len(), 1);
        match &events[0] {
            FileEvent::Changed(path) => {
                assert_eq!(path.as_str(), "/project/content/new-page.md");
            }
            _ => panic!("Expected Changed event"),
        }
    }

    #[test]
    fn test_process_remove_event() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        let (tx, _rx) = std::sync::mpsc::channel();
        let watcher = notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
            let _ = tx.send(());
        })
        .unwrap();
        let watcher = Arc::new(Mutex::new(watcher));

        let event = notify::Event {
            kind: EventKind::Remove(RemoveKind::File),
            paths: vec![PathBuf::from("/project/content/old-page.md")],
            attrs: Default::default(),
        };

        let events = process_notify_event(event, &config, &watcher);
        assert_eq!(events.len(), 1);
        match &events[0] {
            FileEvent::Removed(path) => {
                assert_eq!(path.as_str(), "/project/content/old-page.md");
            }
            _ => panic!("Expected Removed event"),
        }
    }

    #[test]
    fn test_process_rename_from_event() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        let (tx, _rx) = std::sync::mpsc::channel();
        let watcher = notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
            let _ = tx.send(());
        })
        .unwrap();
        let watcher = Arc::new(Mutex::new(watcher));

        // Rename From = file moved away (like delete)
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            paths: vec![PathBuf::from("/project/content/moved-page.md")],
            attrs: Default::default(),
        };

        let events = process_notify_event(event, &config, &watcher);
        assert_eq!(events.len(), 1);
        match &events[0] {
            FileEvent::Removed(path) => {
                assert_eq!(path.as_str(), "/project/content/moved-page.md");
            }
            _ => panic!("Expected Removed event for rename-from"),
        }
    }

    #[test]
    fn test_process_rename_to_event() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        let (tx, _rx) = std::sync::mpsc::channel();
        let watcher = notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
            let _ = tx.send(());
        })
        .unwrap();
        let watcher = Arc::new(Mutex::new(watcher));

        // Rename To = file moved here (like create)
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            paths: vec![PathBuf::from("/project/content/arrived-page.md")],
            attrs: Default::default(),
        };

        let events = process_notify_event(event, &config, &watcher);
        assert_eq!(events.len(), 1);
        match &events[0] {
            FileEvent::Changed(path) => {
                assert_eq!(path.as_str(), "/project/content/arrived-page.md");
            }
            _ => panic!("Expected Changed event for rename-to"),
        }
    }

    #[test]
    fn test_process_rename_both_event() {
        let base = Utf8Path::new("/project");
        let config = test_config(base);

        let (tx, _rx) = std::sync::mpsc::channel();
        let watcher = notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
            let _ = tx.send(());
        })
        .unwrap();
        let watcher = Arc::new(Mutex::new(watcher));

        // Rename Both = both paths in one event (inotify)
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            paths: vec![
                PathBuf::from("/project/content/old-name.md"),
                PathBuf::from("/project/content/new-name.md"),
            ],
            attrs: Default::default(),
        };

        let events = process_notify_event(event, &config, &watcher);
        assert_eq!(events.len(), 2);

        // First should be Removed (old path)
        match &events[0] {
            FileEvent::Removed(path) => {
                assert_eq!(path.as_str(), "/project/content/old-name.md");
            }
            _ => panic!("Expected Removed event for first path"),
        }

        // Second should be Changed (new path)
        match &events[1] {
            FileEvent::Changed(path) => {
                assert_eq!(path.as_str(), "/project/content/new-name.md");
            }
            _ => panic!("Expected Changed event for second path"),
        }
    }
}
