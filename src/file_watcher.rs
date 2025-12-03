//! File watcher for live reload in serve mode
//!
//! Handles:
//! - File creation, modification, and deletion
//! - File moves/renames (treating as delete + create)
//! - New directory detection (adds to watcher)

use camino::{Utf8Path, Utf8PathBuf};
use notify::{
    event::{CreateKind, ModifyKind, RenameMode},
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Configuration for the file watcher
pub struct WatcherConfig {
    pub content_dir: Utf8PathBuf,
    pub templates_dir: Utf8PathBuf,
    pub sass_dir: Utf8PathBuf,
    pub static_dir: Utf8PathBuf,
    pub data_dir: Utf8PathBuf,
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

/// Categorizes a path by which directory it belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCategory {
    Content,
    Template,
    Sass,
    Static,
    Data,
    Unknown,
}

impl WatcherConfig {
    /// Categorize a path by which watched directory it belongs to
    pub fn categorize(&self, path: &Utf8Path) -> PathCategory {
        if path.starts_with(&self.content_dir) {
            PathCategory::Content
        } else if path.starts_with(&self.templates_dir) {
            PathCategory::Template
        } else if path.starts_with(&self.sass_dir) {
            PathCategory::Sass
        } else if path.starts_with(&self.static_dir) {
            PathCategory::Static
        } else if path.starts_with(&self.data_dir) {
            PathCategory::Data
        } else {
            PathCategory::Unknown
        }
    }

    /// Get the relative path for a file within its category
    pub fn relative_path(&self, path: &Utf8Path) -> Option<Utf8PathBuf> {
        match self.categorize(path) {
            PathCategory::Content => path.strip_prefix(&self.content_dir).ok().map(|p| p.to_owned()),
            PathCategory::Template => path.strip_prefix(&self.templates_dir).ok().map(|p| p.to_owned()),
            PathCategory::Sass => path.strip_prefix(&self.sass_dir).ok().map(|p| p.to_owned()),
            PathCategory::Static => path.strip_prefix(&self.static_dir).ok().map(|p| p.to_owned()),
            PathCategory::Data => path.strip_prefix(&self.data_dir).ok().map(|p| p.to_owned()),
            PathCategory::Unknown => None,
        }
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
        PathCategory::Static | PathCategory::Data => true,
        PathCategory::Content | PathCategory::Template | PathCategory::Sass => {
            // For these, check extension
            path.extension()
                .map(|e| {
                    let e = e.to_string_lossy();
                    matches!(
                        e.as_ref(),
                        "md" | "scss" | "css" | "html" | "kdl" | "json" | "toml" | "yaml" | "yml"
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
                            if let Some(utf8) = Utf8PathBuf::from_path_buf(path.clone()).ok() {
                                events.push(FileEvent::DirectoryCreated(utf8));
                            }
                        }
                    }
                } else if should_watch_path(path, config) {
                    if let Some(utf8) = Utf8PathBuf::from_path_buf(path.clone()).ok() {
                        events.push(FileEvent::Changed(utf8));
                    }
                }
            }
        }

        // File modified
        EventKind::Modify(ModifyKind::Data(_)) | EventKind::Modify(ModifyKind::Any) => {
            for path in &event.paths {
                if should_watch_path(path, config) {
                    if let Some(utf8) = Utf8PathBuf::from_path_buf(path.clone()).ok() {
                        events.push(FileEvent::Changed(utf8));
                    }
                }
            }
        }

        // File renamed/moved
        EventKind::Modify(ModifyKind::Name(rename_mode)) => {
            match rename_mode {
                RenameMode::From => {
                    // Old path - treat as deletion
                    for path in &event.paths {
                        if should_watch_path(path, config) {
                            if let Some(utf8) = Utf8PathBuf::from_path_buf(path.clone()).ok() {
                                events.push(FileEvent::Removed(utf8));
                            }
                        }
                    }
                }
                RenameMode::To | RenameMode::Any => {
                    // New path - treat as creation
                    for path in &event.paths {
                        if path.is_dir() {
                            // New directory from rename
                            if let Ok(mut w) = watcher.lock() {
                                let _ = w.watch(path, RecursiveMode::Recursive);
                                if let Some(utf8) = Utf8PathBuf::from_path_buf(path.clone()).ok() {
                                    events.push(FileEvent::DirectoryCreated(utf8));
                                }
                            }
                        } else if should_watch_path(path, config) {
                            if let Some(utf8) = Utf8PathBuf::from_path_buf(path.clone()).ok() {
                                events.push(FileEvent::Changed(utf8));
                            }
                        }
                    }
                }
                RenameMode::Both => {
                    // Both paths provided - first is From, second is To
                    if event.paths.len() >= 2 {
                        let from_path = &event.paths[0];
                        let to_path = &event.paths[1];

                        if should_watch_path(from_path, config) {
                            if let Some(utf8) = Utf8PathBuf::from_path_buf(from_path.clone()).ok() {
                                events.push(FileEvent::Removed(utf8));
                            }
                        }

                        if to_path.is_dir() {
                            if let Ok(mut w) = watcher.lock() {
                                let _ = w.watch(to_path, RecursiveMode::Recursive);
                                if let Some(utf8) = Utf8PathBuf::from_path_buf(to_path.clone()).ok() {
                                    events.push(FileEvent::DirectoryCreated(utf8));
                                }
                            }
                        } else if should_watch_path(to_path, config) {
                            if let Some(utf8) = Utf8PathBuf::from_path_buf(to_path.clone()).ok() {
                                events.push(FileEvent::Changed(utf8));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // File/directory removed
        EventKind::Remove(_) => {
            for path in &event.paths {
                if should_watch_path(path, config) {
                    if let Some(utf8) = Utf8PathBuf::from_path_buf(path.clone()).ok() {
                        events.push(FileEvent::Removed(utf8));
                    }
                }
            }
        }

        _ => {}
    }

    events
}

/// Create and configure the file watcher
pub fn create_watcher(
    config: &WatcherConfig,
) -> color_eyre::Result<(Arc<Mutex<RecommendedWatcher>>, std::sync::mpsc::Receiver<notify::Result<notify::Event>>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;

    let watcher = Arc::new(Mutex::new(watcher));

    // Watch all configured directories
    {
        let mut w = watcher.lock().unwrap();
        w.watch(config.content_dir.as_std_path(), RecursiveMode::Recursive)?;

        if config.templates_dir.exists() {
            w.watch(config.templates_dir.as_std_path(), RecursiveMode::Recursive)?;
        }
        if config.sass_dir.exists() {
            w.watch(config.sass_dir.as_std_path(), RecursiveMode::Recursive)?;
        }
        if config.static_dir.exists() {
            w.watch(config.static_dir.as_std_path(), RecursiveMode::Recursive)?;
        }
        if config.data_dir.exists() {
            w.watch(config.data_dir.as_std_path(), RecursiveMode::Recursive)?;
        }
    }

    Ok((watcher, rx))
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
            data_dir: base.join("data"),
        }
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
            config.relative_path(Utf8Path::new("/other/file.txt")),
            None
        );
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
        }).unwrap();
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
        }).unwrap();
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
        }).unwrap();
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
        }).unwrap();
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
        }).unwrap();
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
