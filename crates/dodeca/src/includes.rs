//! Tracked file inclusion for the `include` shortcode.
//!
//! The `include` shortcode reads the [`IncludedFileRegistry`](crate::db::IncludedFileRegistry)
//! input (recording a picante dependency on it) and uses that content when
//! present, falling back to a direct disk read otherwise. Paths it references are
//! noted here; the serve loop drains them via [`refresh`], reads the files,
//! republishes the registry, and watches them. Because the shortcode *always*
//! reads the registry, republishing it after a file changes invalidates exactly
//! the pages that embed that file — so an edited README hot-reloads its host page
//! without an untracked read on every render.

use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

use camino::Utf8Path;
use tokio::sync::Notify;

use crate::db::{Database, IncludedFileEntry, IncludedFileRegistry};

/// Project-root-relative paths referenced by `include` shortcodes so far.
static KNOWN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Signalled when a *new* include path is first seen, so the serve loop can load
/// and watch it promptly (the first render of an include happens on a request,
/// not a file-change batch).
fn dirty() -> &'static Notify {
    static NOTIFY: OnceLock<Notify> = OnceLock::new();
    NOTIFY.get_or_init(Notify::new)
}

/// Await the next "a new include appeared" signal. The serve loop calls
/// [`refresh`] after this resolves.
pub async fn wait_dirty() {
    dirty().notified().await;
}

/// Resolve the content of an included file: read the registry first (recording
/// the dependency), then fall back to a direct read from `project_root`. Notes
/// the path so the serve loop loads + watches it.
pub fn read(rel: &str, project_root: &Utf8Path) -> Option<String> {
    note(rel);
    // Always read the registry so the calling render records a dependency on it.
    let from_registry = crate::db::TASK_DB
        .try_with(|db| IncludedFileRegistry::files(db.as_ref()).ok().flatten())
        .ok()
        .flatten()
        .and_then(|files| files.into_iter().find(|f| f.path == rel).map(|f| f.content));
    if from_registry.is_some() {
        return from_registry;
    }
    std::fs::read_to_string(project_root.join(rel)).ok()
}

fn note(rel: &str) {
    let mut known = KNOWN.lock().unwrap();
    if known.insert(rel.to_string()) {
        dirty().notify_one();
    }
}

/// Re-read every known included file and republish the [`IncludedFileRegistry`]
/// if its contents changed. Returns the absolute paths to watch. Cheap (includes
/// are few) and only sets the input on a real change, so it won't spuriously
/// invalidate pages.
pub fn refresh(db: &Database, project_root: &Utf8Path) -> Vec<camino::Utf8PathBuf> {
    let known: Vec<String> = KNOWN.lock().unwrap().iter().cloned().collect();

    let mut entries = Vec::new();
    let mut watch = Vec::new();
    for rel in known {
        let abs = project_root.join(&rel);
        watch.push(abs.clone());
        if let Ok(content) = std::fs::read_to_string(&abs) {
            entries.push(IncludedFileEntry { path: rel, content });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let current = IncludedFileRegistry::files(db)
        .ok()
        .flatten()
        .unwrap_or_default();
    if current != entries
        && let Err(e) = IncludedFileRegistry::set(db, entries)
    {
        tracing::warn!(error = %e, "includes: failed to publish registry");
    }
    watch
}
