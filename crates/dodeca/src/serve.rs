//! HTTP server that serves content directly from the picante database
//!
//! No files are read from disk - everything is queried from picante on demand.
//! This enables instant incremental rebuilds with zero disk I/O.

/// Picante cache version - bump this when making incompatible changes to picante inputs/queries
pub const PICANTE_CACHE_VERSION: u32 = 4;

use eyre::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, watch};

use crate::db::{
    DataFile, DataRegistry, Database, DatabaseSnapshot, SassFile, SassRegistry, SourceFile,
    SourceRegistry, StaticFile, StaticRegistry, TemplateFile, TemplateRegistry,
};
use crate::image::{InputFormat, OutputFormat, add_width_suffix};
use crate::queries::{build_tree, css_output, process_image, serve_html, static_file_output};
use crate::render::{RenderOptions, inject_livereload_with_build_info};
use crate::types::Route;
use std::collections::HashSet;

use dodeca_protocol::{ScopeEntry, ScopeValue};
use facet_value::DestructuredRef;

// ============================================================================
// Scope conversion for devtools
// ============================================================================

/// Convert a facet_value::Value to a ScopeValue for the devtools protocol
fn value_to_scope_value(value: &facet_value::Value) -> ScopeValue {
    match value.destructure_ref() {
        DestructuredRef::Null => ScopeValue::Null,
        DestructuredRef::Bool(b) => ScopeValue::Bool(b),
        DestructuredRef::Number(n) => {
            let f = n.to_f64().unwrap_or(0.0);
            ScopeValue::Number(f)
        }
        DestructuredRef::String(s) => {
            let s_str = s.to_string();
            // Truncate long strings for preview
            if s_str.len() > 100 {
                ScopeValue::String(format!("{}...", &s_str[..100]))
            } else {
                ScopeValue::String(s_str)
            }
        }
        DestructuredRef::Bytes(b) => ScopeValue::String(format!("<{} bytes>", b.len())),
        DestructuredRef::Array(arr) => {
            let len = arr.len();
            let preview = if len == 0 {
                "[]".to_string()
            } else if len <= 3 {
                let items: Vec<String> = arr.iter().take(3).map(value_preview).collect();
                format!("[{}]", items.join(", "))
            } else {
                let items: Vec<String> = arr.iter().take(3).map(value_preview).collect();
                format!("[{}, ...]", items.join(", "))
            };
            ScopeValue::Array {
                length: len,
                preview,
            }
        }
        DestructuredRef::Object(obj) => {
            let fields = obj.len();
            let preview = if fields == 0 {
                "{}".to_string()
            } else {
                let keys: Vec<String> = obj.keys().take(3).map(|k| k.to_string()).collect();
                if fields <= 3 {
                    format!("{{{}}}", keys.join(", "))
                } else {
                    format!("{{{}, ...}}", keys.join(", "))
                }
            };
            ScopeValue::Object { fields, preview }
        }
        DestructuredRef::DateTime(dt) => ScopeValue::String(format!("{:?}", dt)),
        DestructuredRef::QName(qn) => ScopeValue::String(format!("{:?}", qn)),
        DestructuredRef::Uuid(uuid) => ScopeValue::String(format!("{:?}", uuid)),
    }
}

/// Generate a short preview string for a value
fn value_preview(value: &facet_value::Value) -> String {
    match value.destructure_ref() {
        DestructuredRef::Null => "null".to_string(),
        DestructuredRef::Bool(b) => b.to_string(),
        DestructuredRef::Number(n) => n.to_f64().map(|f| f.to_string()).unwrap_or("0".to_string()),
        DestructuredRef::String(s) => {
            let s_str = s.to_string();
            if s_str.len() > 20 {
                format!("\"{}...\"", &s_str[..20])
            } else {
                format!("\"{}\"", s_str)
            }
        }
        DestructuredRef::Bytes(b) => format!("<{} bytes>", b.len()),
        DestructuredRef::Array(arr) => format!("[{} items]", arr.len()),
        DestructuredRef::Object(obj) => format!("{{{} fields}}", obj.len()),
        DestructuredRef::DateTime(_) => "<datetime>".to_string(),
        DestructuredRef::QName(_) => "<qname>".to_string(),
        DestructuredRef::Uuid(_) => "<uuid>".to_string(),
    }
}

/// Check if a value can be expanded (has children)
fn value_is_expandable(value: &facet_value::Value) -> bool {
    match value.destructure_ref() {
        DestructuredRef::Array(arr) => !arr.is_empty(),
        DestructuredRef::Object(obj) => !obj.is_empty(),
        _ => false,
    }
}

/// Convert a facet_value::Value to a list of ScopeEntry (for the top-level or expanded path)
fn value_to_scope_entries(value: &facet_value::Value, path: &[String]) -> Vec<ScopeEntry> {
    // Navigate to the requested path
    let target = navigate_value(value, path);
    let target = match target {
        Some(v) => v,
        None => return vec![],
    };

    match target.destructure_ref() {
        DestructuredRef::Object(obj) => obj
            .iter()
            .map(|(key, val)| ScopeEntry {
                name: key.to_string(),
                value: value_to_scope_value(val),
                expandable: value_is_expandable(val),
            })
            .collect(),
        DestructuredRef::Array(arr) => arr
            .iter()
            .enumerate()
            .map(|(idx, val)| ScopeEntry {
                name: idx.to_string(),
                value: value_to_scope_value(val),
                expandable: value_is_expandable(val),
            })
            .collect(),
        _ => {
            // Scalar value at path - return as single entry
            vec![ScopeEntry {
                name: path.last().cloned().unwrap_or_else(|| "value".to_string()),
                value: value_to_scope_value(&target),
                expandable: false,
            }]
        }
    }
}

/// Navigate into a value by path
fn navigate_value(value: &facet_value::Value, path: &[String]) -> Option<facet_value::Value> {
    let mut current = value.clone();
    for segment in path {
        current = match current.destructure_ref() {
            DestructuredRef::Object(obj) => obj.get(segment.as_str())?.clone(),
            DestructuredRef::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                arr.get(idx)?.clone()
            }
            _ => return None,
        };
    }
    Some(current)
}

/// Message types for livereload WebSocket
///
/// These variants are serialized and sent over WebSocket to the browser,
/// so the fields are read during serialization even though Rust doesn't see direct reads.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum LiveReloadMsg {
    /// Full page reload (fallback)
    Reload,
    /// Patches for a specific route
    Patches {
        route: String,
        patches: Vec<dodeca_protocol::Patch>,
    },
    /// CSS update (new cache-busted path)
    CssUpdate { path: String },
    /// Template error occurred
    Error {
        route: String,
        message: String,
        template: Option<String>,
        line: Option<u32>,
        snapshot_id: String,
    },
    /// Error was resolved (template renders successfully now)
    ErrorResolved { route: String },
}

/// Summarize patch operations for logging
fn summarize_patches(patches: &[dodeca_protocol::Patch]) -> String {
    use dodeca_protocol::Patch;

    let mut replace = 0;
    let mut insert = 0;
    let mut remove = 0;
    let mut set_text = 0;
    let mut set_attr = 0;
    let mut remove_attr = 0;

    for patch in patches {
        match patch {
            Patch::Replace { .. } => replace += 1,
            Patch::InsertBefore { .. } | Patch::InsertAfter { .. } | Patch::AppendChild { .. } => {
                insert += 1
            }
            Patch::Remove { .. } => remove += 1,
            Patch::SetText { .. } => set_text += 1,
            Patch::SetAttribute { .. } => set_attr += 1,
            Patch::RemoveAttribute { .. } => remove_attr += 1,
        }
    }

    let mut parts = Vec::new();
    if replace > 0 {
        parts.push(format!("{} replace", replace));
    }
    if insert > 0 {
        parts.push(format!("{} insert", insert));
    }
    if remove > 0 {
        parts.push(format!("{} remove", remove));
    }
    if set_text > 0 {
        parts.push(format!("{} text", set_text));
    }
    if set_attr > 0 {
        parts.push(format!("{} attr", set_attr));
    }
    if remove_attr > 0 {
        parts.push(format!("{} -attr", remove_attr));
    }

    if parts.is_empty() {
        "no ops".to_string()
    } else {
        parts.join(", ")
    }
}

/// Shared state for the dev server
pub struct SiteServer {
    /// The picante database - all queries go through here
    pub db: Arc<Database>,
    /// Search index files (pagefind): path -> content
    pub search_files: RwLock<HashMap<String, Vec<u8>>>,
    /// Live reload broadcast
    pub livereload_tx: broadcast::Sender<LiveReloadMsg>,
    /// Render options (dev mode, etc.)
    pub render_options: RenderOptions,
    /// Cached HTML for each route (for computing patches)
    html_cache: RwLock<HashMap<String, String>>,
    /// Cached CSS path (cache-busted) for detecting CSS-only changes
    css_cache: RwLock<Option<String>>,
    /// Asset paths that should be served at original paths (no cache-busting)
    stable_assets: Vec<String>,
    /// Current errors by route (for sending to newly connected clients)
    current_errors: RwLock<HashMap<String, dodeca_protocol::ErrorInfo>>,
    /// Cached code execution results for build info display
    code_execution_results: RwLock<Vec<crate::db::CodeExecutionResult>>,
    /// Revision readiness gate
    revision_tx: watch::Sender<crate::revision::RevisionState>,
}

impl SiteServer {
    pub fn new(render_options: RenderOptions, stable_assets: Vec<String>) -> Self {
        let (livereload_tx, _) = broadcast::channel(16);
        let db = Database::new(None);
        let (revision_tx, _) = watch::channel(crate::revision::RevisionState {
            generation: 0,
            status: crate::revision::RevisionStatus::Building,
            reason: Some("startup".to_string()),
            started_at: None,
        });

        Self {
            db: Arc::new(db),
            search_files: RwLock::new(HashMap::new()),
            livereload_tx,
            render_options,
            html_cache: RwLock::new(HashMap::new()),
            css_cache: RwLock::new(None),
            stable_assets,
            current_errors: RwLock::new(HashMap::new()),
            code_execution_results: RwLock::new(Vec::new()),
            revision_tx,
        }
    }

    pub fn begin_revision(&self, reason: impl Into<String>) -> crate::revision::RevisionToken {
        let reason = reason.into();
        let next_generation = self.revision_tx.borrow().generation + 1;
        let started_at = std::time::Instant::now();
        let state = crate::revision::RevisionState {
            generation: next_generation,
            status: crate::revision::RevisionStatus::Building,
            reason: Some(reason.clone()),
            started_at: Some(started_at),
        };
        self.revision_tx.send_replace(state);
        tracing::debug!(
            generation = next_generation,
            reason = %reason,
            "revision: begin"
        );
        crate::revision::RevisionToken {
            generation: next_generation,
            started_at,
        }
    }

    pub fn end_revision(&self, token: crate::revision::RevisionToken) {
        let current = self.revision_tx.borrow().clone();
        if current.generation != token.generation {
            tracing::debug!(
                current_generation = current.generation,
                token_generation = token.generation,
                "revision: ignoring stale end"
            );
            return;
        }

        let state = crate::revision::RevisionState {
            generation: token.generation,
            status: crate::revision::RevisionStatus::Ready,
            reason: None,
            started_at: None,
        };
        self.revision_tx.send_replace(state);
        tracing::debug!(
            generation = token.generation,
            elapsed_ms = token.started_at.elapsed().as_millis(),
            "revision: ready"
        );
    }

    pub async fn wait_revision_ready(&self) {
        let mut rx = self.revision_tx.subscribe();
        loop {
            let state = rx.borrow().clone();
            if state.status == crate::revision::RevisionStatus::Ready {
                return;
            }

            tracing::debug!(
                generation = state.generation,
                reason = state.reason.as_deref().unwrap_or(""),
                "revision: waiting"
            );

            if rx.changed().await.is_err() {
                return;
            }

            let state = rx.borrow().clone();
            if state.status == crate::revision::RevisionStatus::Ready {
                tracing::debug!(generation = state.generation, "revision: ready");
                return;
            }
        }
    }

    /// Get the current revision generation
    pub fn current_generation(&self) -> u64 {
        self.revision_tx.borrow().generation
    }

    /// Update cached code execution results
    pub fn set_code_execution_results(&self, results: Vec<crate::db::CodeExecutionResult>) {
        *self.code_execution_results.write().unwrap() = results;
    }

    /// Check if a path is configured as a stable asset
    fn is_stable_asset(&self, path: &str) -> bool {
        self.stable_assets.iter().any(|p| p == path)
    }

    /// Update the source registry with a new list of sources
    /// This invalidates all queries that depend on sources
    pub fn set_sources(&self, sources: Vec<SourceFile>) {
        SourceRegistry::set(&*self.db, sources).expect("failed to set sources");
    }

    /// Update the template registry with a new list of templates
    pub fn set_templates(&self, templates: Vec<TemplateFile>) {
        TemplateRegistry::set(&*self.db, templates).expect("failed to set templates");
    }

    /// Update the sass registry with a new list of sass files
    pub fn set_sass_files(&self, files: Vec<SassFile>) {
        SassRegistry::set(&*self.db, files).expect("failed to set sass files");
    }

    /// Update the static registry with a new list of static files
    pub fn set_static_files(&self, files: Vec<StaticFile>) {
        StaticRegistry::set(&*self.db, files).expect("failed to set static files");
    }

    /// Update the data registry with a new list of data files
    pub fn set_data_files(&self, files: Vec<DataFile>) {
        DataRegistry::set(&*self.db, files).expect("failed to set data files");
    }

    /// Get a clone of the current sources (for modification)
    pub fn get_sources(&self) -> Vec<SourceFile> {
        SourceRegistry::sources(&*self.db)
            .expect("failed to get sources")
            .unwrap_or_default()
    }

    /// Get a clone of the current templates (for modification)
    pub fn get_templates(&self) -> Vec<TemplateFile> {
        TemplateRegistry::templates(&*self.db)
            .expect("failed to get templates")
            .unwrap_or_default()
    }

    /// Get a clone of the current sass files (for modification)
    pub fn get_sass_files(&self) -> Vec<SassFile> {
        SassRegistry::files(&*self.db)
            .expect("failed to get sass files")
            .unwrap_or_default()
    }

    /// Get a clone of the current static files (for modification)
    pub fn get_static_files(&self) -> Vec<StaticFile> {
        StaticRegistry::files(&*self.db)
            .expect("failed to get static files")
            .unwrap_or_default()
    }

    /// Notify all connected browsers to reload
    /// Computes patches for all cached routes and sends them
    pub async fn trigger_reload(&self) {
        use crate::cells::diff_html_cell;

        // Check for CSS changes first
        let old_css_path = {
            let cache = self.css_cache.read().unwrap();
            cache.clone()
        };
        let new_css_path = self.get_current_css_path().await;
        let css_changed = old_css_path != new_css_path;

        if css_changed {
            // Update CSS cache
            if let Some(ref path) = new_css_path {
                self.cache_css(path);
            }

            if let Some(ref path) = new_css_path {
                tracing::debug!("CSS changed: {}", path);
                let _ = self
                    .livereload_tx
                    .send(LiveReloadMsg::CssUpdate { path: path.clone() });
            }
        }

        // Get all cached routes and compute patches
        let cached_routes: Vec<String> = {
            let cache = self.html_cache.read().unwrap();
            cache.keys().cloned().collect()
        };

        if cached_routes.is_empty() {
            // No pages have been visited yet - nothing to patch
            // This can happen if files change before any page is loaded
            tracing::debug!("No cached routes, nothing to patch");
            return;
        }

        tracing::debug!(
            "trigger_reload: checking {} cached routes",
            cached_routes.len()
        );

        for route in cached_routes {
            // Get old HTML from cache
            let old_html = {
                let cache = self.html_cache.read().unwrap();
                cache.get(&route).cloned()
            };

            // Get new HTML (re-render) - this creates a fresh snapshot
            tracing::debug!("trigger_reload: re-rendering {}", route);
            let new_html = self.find_content(&route).await.and_then(|c| match c {
                ServeContent::Html(html) => Some(html),
                _ => None,
            });

            // Handle case where route was deleted (old exists, new doesn't)
            if old_html.is_some() && new_html.is_none() {
                tracing::info!("{} - route deleted, sending full reload", route);
                // Remove from cache
                {
                    let mut cache = self.html_cache.write().unwrap();
                    cache.remove(&route);
                }
                // Send full page reload since we can't patch a deleted page
                let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
                continue;
            }

            if let (Some(old), Some(new)) = (old_html, new_html.clone()) {
                let old_has_error = old.contains(crate::render::RENDER_ERROR_MARKER);
                let new_has_error = new.contains(crate::render::RENDER_ERROR_MARKER);
                tracing::debug!(
                    "trigger_reload: {} - old_has_error={}, new_has_error={}, html_changed={}",
                    route,
                    old_has_error,
                    new_has_error,
                    old != new
                );

                if old != new {
                    // If the new HTML is an error page, don't patch it in
                    // The error has already been sent to devtools via LiveReloadMsg::Error
                    // and we want to keep the old working HTML in cache
                    if new_has_error {
                        tracing::info!("🔴 {} - template error detected in trigger_reload", route);
                        continue;
                    }

                    // Update cache
                    {
                        let mut cache = self.html_cache.write().unwrap();
                        if let Some(html) = &new_html {
                            cache.insert(route.clone(), html.clone());
                        }
                    }

                    // Try to diff using the cell
                    match diff_html_cell(&old, &new).await {
                        Some(diff_result) => {
                            if diff_result.patches.is_empty() {
                                // DOM structure identical but HTML differs (whitespace/comments?)
                                // This is a no-op - no need to reload for invisible changes
                                tracing::debug!(
                                    "{} - HTML changed but DOM identical, no-op",
                                    route
                                );
                            } else {
                                // Summarize patch operations
                                let summary = summarize_patches(&diff_result.patches);
                                let patch_count = diff_result.patches.len();

                                tracing::debug!(
                                    "{} - patching: {} ({} patches)",
                                    route,
                                    summary,
                                    patch_count
                                );
                                let _ = self.livereload_tx.send(LiveReloadMsg::Patches {
                                    route: route.clone(),
                                    patches: diff_result.patches,
                                });
                            }
                        }
                        None => {
                            // Cell not available or failed - fall back to full reload
                            tracing::debug!(
                                "{} - html_diff cell not available, sending full reload",
                                route
                            );
                            let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
                        }
                    }
                }
            }
        }

        // Note: we don't send a fallback reload here. All meaningful changes
        // (templates, content, CSS, static assets) result in cache-busted URL
        // changes which appear as HTML patches. If nothing changed, nothing changed.
    }

    /// Cache HTML for a route (called when serving pages)
    fn cache_html(&self, route: &str, html: &str) {
        let mut cache = self.html_cache.write().unwrap();
        cache.insert(route.to_string(), html.to_string());
    }

    /// Cache CSS path (called when serving CSS)
    fn cache_css(&self, path: &str) {
        let mut cache = self.css_cache.write().unwrap();
        *cache = Some(path.to_string());
    }

    /// Get current CSS path from database
    async fn get_current_css_path(&self) -> Option<String> {
        let snapshot = DatabaseSnapshot::from_database(&self.db).await;
        let css = css_output(&snapshot).await.ok().flatten()?;
        Some(format!("/{}", css.cache_busted_path))
    }

    /// Load cached query results from disk
    pub fn load_cache(&self, cache_path: &std::path::Path) -> Result<()> {
        // Check version file first - if missing or mismatched, delete the cache
        let version_path = cache_path.with_extension("version");
        let version_ok = if version_path.exists() {
            match std::fs::read_to_string(&version_path) {
                Ok(v) => v.trim().parse::<u32>().ok() == Some(PICANTE_CACHE_VERSION),
                Err(_) => false,
            }
        } else {
            false
        };

        if !version_ok {
            if cache_path.exists() {
                tracing::info!(
                    "Picante cache version mismatch (expected v{}), deleting stale cache",
                    PICANTE_CACHE_VERSION
                );
                let _ = std::fs::remove_file(cache_path);
            }
            return Ok(());
        }

        if !cache_path.exists() {
            tracing::info!("No cache file found, starting fresh");
            return Ok(());
        }

        // TODO: picante cache serialization not yet implemented
        // For now, we always start fresh
        tracing::debug!("Cache loading disabled (picante migration in progress)");
        Ok(())
    }

    /// Save cached query results to disk
    pub fn save_cache(&self, cache_path: &std::path::Path) -> Result<()> {
        // TODO: picante cache serialization not yet implemented
        // For now, skip saving
        let _ = cache_path;
        tracing::debug!("Cache saving disabled (picante migration in progress)");
        Ok(())
    }

    /// Find content for a given path using lazy picante queries
    async fn find_content(&self, path: &str) -> Option<ServeContent> {
        tracing::debug!(path, "find_content: called");
        let db = self.db.clone();
        let snapshot = DatabaseSnapshot::from_database(&self.db).await;
        tracing::debug!(path, "find_content: got database snapshot");

        // Get known routes for dead link detection (only in dev mode)
        let known_routes: Option<HashSet<String>> = if self.render_options.livereload {
            let site_tree = build_tree(&snapshot).await.ok()?.ok()?;
            let routes: HashSet<String> = site_tree
                .sections
                .keys()
                .chain(site_tree.pages.keys())
                .map(|r| r.as_str().to_string())
                .collect();
            Some(routes)
        } else {
            None
        };

        // 1. Try to serve as HTML page (by route)
        let route_path = if path == "/" {
            "/".to_string()
        } else {
            path.trim_end_matches('/').to_string()
        };

        let route = Route::new(route_path.clone());
        tracing::debug!(route = %route.as_str(), "find_content: calling serve_html");
        // Set task-local db for render functions to access
        let serve_html_result = crate::db::TASK_DB
            .scope(db, serve_html(&snapshot, route))
            .await;
        tracing::debug!(route = %route_path, has_result = serve_html_result.is_ok(), "find_content: serve_html returned");
        if let Some(html) = serve_html_result.ok().flatten() {
            // Check if this is an error page and notify devtools
            if html.contains(crate::render::RENDER_ERROR_MARKER) {
                // Extract error message HTML from between <pre> tags
                // Keep the HTML spans for color - they'll be displayed with dangerous_inner_html
                let error_msg = html
                    .split("<pre>")
                    .nth(1)
                    .and_then(|s| s.split("</pre>").next())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Unknown template error".to_string());

                let error_info = dodeca_protocol::ErrorInfo {
                    route: path.to_string(),
                    message: error_msg,
                    template: None, // TODO: extract from error message
                    line: None,     // TODO: extract from error message
                    column: None,
                    source_snippet: None,
                    snapshot_id: format!(
                        "error-{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                    ),
                    available_variables: vec![],
                };

                tracing::info!(
                    "🔴 find_content: error detected for {}, sending LiveReloadMsg::Error",
                    path
                );

                // Store for newly connecting clients
                {
                    let mut errors = self.current_errors.write().unwrap();
                    errors.insert(path.to_string(), error_info.clone());
                }

                let send_result = self.livereload_tx.send(LiveReloadMsg::Error {
                    route: error_info.route.clone(),
                    message: error_info.message.clone(),
                    template: error_info.template.clone(),
                    line: error_info.line,
                    snapshot_id: error_info.snapshot_id.clone(),
                });
                tracing::debug!(
                    "🔴 find_content: LiveReloadMsg::Error send result: {:?} (receivers: {})",
                    send_result.is_ok(),
                    self.livereload_tx.receiver_count()
                );
            } else {
                // Page rendered successfully - clear any previous error
                {
                    let mut errors = self.current_errors.write().unwrap();
                    errors.remove(path);
                }
                let _ = self.livereload_tx.send(LiveReloadMsg::ErrorResolved {
                    route: path.to_string(),
                });
            }

            let code_results: Vec<_> = self.code_execution_results.read().unwrap().clone();
            let html = inject_livereload_with_build_info(
                &html,
                self.render_options,
                known_routes.as_ref(),
                &code_results,
            )
            .await;
            return Some(ServeContent::Html(html));
        }

        // 2. Try to serve CSS (check if path matches cache-busted CSS path)
        if let Some(css) = css_output(&snapshot).await.ok().flatten() {
            let css_url = format!("/{}", css.cache_busted_path);
            if path == css_url {
                return Some(ServeContent::Css(css.content));
            }
        }

        // 3. Try to serve static files (match cache-busted paths)
        let static_files = StaticRegistry::files(&snapshot).ok()?.unwrap_or_default();
        for file in static_files.iter() {
            let original_path = file.path(&snapshot).ok()?.as_str().to_string();
            let original_path = original_path.as_str();

            // Check if this is a processable image
            if InputFormat::is_processable(original_path) {
                use crate::cas::ImageVariantKey;
                use crate::queries::{image_input_hash, image_metadata};

                // Get metadata and input hash (fast - no encoding)
                let Some(metadata) = image_metadata(&snapshot, *file).await.ok().flatten() else {
                    continue;
                };
                let input_hash = image_input_hash(&snapshot, *file).await.ok()?;

                // Check each possible variant URL
                for &width in &metadata.variant_widths {
                    // Check JXL variant
                    let jxl_base = crate::image::change_extension(
                        original_path,
                        OutputFormat::Jxl.extension(),
                    );
                    let jxl_variant_path = if width == metadata.width {
                        jxl_base.clone()
                    } else {
                        add_width_suffix(&jxl_base, width)
                    };
                    let jxl_key = ImageVariantKey {
                        input_hash,
                        format: OutputFormat::Jxl,
                        width,
                    };
                    let jxl_cache_busted = format!(
                        "{}.{}.jxl",
                        jxl_variant_path.trim_end_matches(".jxl"),
                        jxl_key.url_hash()
                    );
                    if path == format!("/{jxl_cache_busted}") {
                        // NOW process the image (lazy!)
                        if let Some(processed) =
                            process_image(&snapshot, *file).await.ok().flatten()
                            && let Some(variant) =
                                processed.jxl_variants.iter().find(|v| v.width == width)
                        {
                            return Some(ServeContent::Static(variant.data.clone(), "image/jxl"));
                        }
                    }

                    // Check WebP variant
                    let webp_base = crate::image::change_extension(
                        original_path,
                        OutputFormat::WebP.extension(),
                    );
                    let webp_variant_path = if width == metadata.width {
                        webp_base.clone()
                    } else {
                        add_width_suffix(&webp_base, width)
                    };
                    let webp_key = ImageVariantKey {
                        input_hash,
                        format: OutputFormat::WebP,
                        width,
                    };
                    let webp_cache_busted = format!(
                        "{}.{}.webp",
                        webp_variant_path.trim_end_matches(".webp"),
                        webp_key.url_hash()
                    );
                    if path == format!("/{webp_cache_busted}") {
                        // NOW process the image (lazy!)
                        if let Some(processed) =
                            process_image(&snapshot, *file).await.ok().flatten()
                            && let Some(variant) =
                                processed.webp_variants.iter().find(|v| v.width == width)
                        {
                            return Some(ServeContent::Static(variant.data.clone(), "image/webp"));
                        }
                    }
                }
            } else {
                // Non-image static file
                let output = static_file_output(&snapshot, *file).await.ok()?;
                let static_url = format!("/{}", output.cache_busted_path);
                if path == static_url {
                    let mime = mime_from_extension(path);
                    return Some(ServeContent::Static(output.content, mime));
                }

                // Also serve stable assets at their original paths (no cache-busting)
                if self.is_stable_asset(original_path) {
                    let original_url = format!("/{}", original_path);
                    if path == original_url {
                        let mime = mime_from_extension(path);
                        return Some(ServeContent::StaticNoCache(output.content, mime));
                    }
                }
            }
        }

        None
    }

    /// Get the template scope for a route (for devtools scope explorer)
    ///
    /// Returns a list of top-level scope entries that can be expanded.
    /// The `path` parameter is used to drill into nested values.
    pub async fn get_scope_for_route(&self, route_path: &str, path: &[String]) -> Vec<ScopeEntry> {
        use facet_value::{VObject, VString};

        let snapshot = DatabaseSnapshot::from_database(&self.db).await;

        let site_tree = match build_tree(&snapshot).await {
            Ok(Ok(tree)) => tree,
            Ok(Err(_)) | Err(_) => return vec![],
        };

        // Normalize route
        let route_str = if route_path == "/" {
            "/".to_string()
        } else {
            let trimmed = route_path.trim_end_matches('/');
            if trimmed.is_empty() {
                "/".to_string()
            } else {
                trimmed.to_string()
            }
        };
        let route = Route::new(route_str);

        // Build scope based on whether this is a section or page
        let mut scope = VObject::new();

        // Add config (same as build_render_context_base)
        let mut config_map = VObject::new();
        let (site_title, site_description) = site_tree
            .sections
            .get(&Route::root())
            .map(|root| {
                (
                    root.title.to_string(),
                    root.description.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_else(|| ("Untitled".to_string(), String::new()));
        let base_url = crate::config::global_config()
            .map(|c| c.base_url.clone())
            .unwrap_or_else(|| "/".to_string());
        config_map.insert(
            VString::from("title"),
            facet_value::Value::from(site_title.as_str()),
        );
        config_map.insert(
            VString::from("description"),
            facet_value::Value::from(site_description.as_str()),
        );
        config_map.insert(
            VString::from("base_url"),
            facet_value::Value::from(base_url.as_str()),
        );
        scope.insert(
            VString::from("config"),
            facet_value::Value::from(config_map),
        );

        // Add current_path
        scope.insert(
            VString::from("current_path"),
            facet_value::Value::from(route.as_str()),
        );

        // Check if it's a section or page
        if let Some(section) = site_tree.sections.get(&route) {
            // Add section data
            let mut section_map = VObject::new();
            section_map.insert(
                VString::from("title"),
                facet_value::Value::from(section.title.as_str()),
            );
            section_map.insert(
                VString::from("permalink"),
                facet_value::Value::from(section.route.as_str()),
            );
            section_map.insert(
                VString::from("weight"),
                facet_value::Value::from(section.weight as i64),
            );
            if let Some(ref desc) = section.description {
                section_map.insert(
                    VString::from("description"),
                    facet_value::Value::from(desc.as_str()),
                );
            }
            section_map.insert(VString::from("extra"), section.extra.clone());

            // Count pages in this section
            let page_count = site_tree
                .pages
                .values()
                .filter(|p| p.section_route == section.route)
                .count();
            section_map.insert(
                VString::from("pages_count"),
                facet_value::Value::from(page_count as i64),
            );

            scope.insert(
                VString::from("section"),
                facet_value::Value::from(section_map),
            );
        } else if let Some(page) = site_tree.pages.get(&route) {
            // Add page data
            let mut page_map = VObject::new();
            page_map.insert(
                VString::from("title"),
                facet_value::Value::from(page.title.as_str()),
            );
            page_map.insert(
                VString::from("permalink"),
                facet_value::Value::from(page.route.as_str()),
            );
            page_map.insert(
                VString::from("weight"),
                facet_value::Value::from(page.weight as i64),
            );
            page_map.insert(VString::from("extra"), page.extra.clone());
            page_map.insert(
                VString::from("headings_count"),
                facet_value::Value::from(page.headings.len() as i64),
            );
            scope.insert(VString::from("page"), facet_value::Value::from(page_map));

            // Add parent section
            if let Some(section) = site_tree.sections.get(&page.section_route) {
                let mut section_map = VObject::new();
                section_map.insert(
                    VString::from("title"),
                    facet_value::Value::from(section.title.as_str()),
                );
                section_map.insert(
                    VString::from("permalink"),
                    facet_value::Value::from(section.route.as_str()),
                );
                scope.insert(
                    VString::from("section"),
                    facet_value::Value::from(section_map),
                );
            }
        }

        // Add root section info
        if let Some(root) = site_tree.sections.get(&Route::root()) {
            let mut root_map = VObject::new();
            root_map.insert(
                VString::from("title"),
                facet_value::Value::from(root.title.as_str()),
            );

            // Count total sections and pages
            let section_count = site_tree.sections.len();
            let page_count = site_tree.pages.len();
            root_map.insert(
                VString::from("sections_count"),
                facet_value::Value::from(section_count as i64),
            );
            root_map.insert(
                VString::from("pages_count"),
                facet_value::Value::from(page_count as i64),
            );

            scope.insert(VString::from("root"), facet_value::Value::from(root_map));
        }

        // Load actual data files
        let raw_data = crate::queries::load_all_data_raw(&snapshot)
            .await
            .unwrap_or_default();
        let data_value = crate::data::parse_raw_data_files(&raw_data);
        scope.insert(VString::from("data"), data_value);

        // Convert scope to entries
        let scope_value: facet_value::Value = scope.into();
        value_to_scope_entries(&scope_value, path)
    }

    /// Evaluate an expression against the scope for a route (for REPL)
    pub async fn eval_expression_for_route(
        &self,
        route_path: &str,
        expression: &str,
    ) -> Result<ScopeValue, String> {
        use facet_value::{VObject, VString};

        let snapshot = DatabaseSnapshot::from_database(&self.db).await;

        let site_tree = build_tree(&snapshot)
            .await
            .map_err(|e| format!("Failed to build tree: {:?}", e))?
            .map_err(|e| format!("Source parse errors: {:?}", e))?;

        // Normalize route
        let route_str = if route_path == "/" {
            "/".to_string()
        } else {
            let trimmed = route_path.trim_end_matches('/');
            if trimmed.is_empty() {
                "/".to_string()
            } else {
                trimmed.to_string()
            }
        };
        let route = Route::new(route_str);

        // Build gingembre Context with scope values
        let mut ctx = gingembre::Context::new();

        // Add config
        let mut config_map = VObject::new();
        let (site_title, site_description) = site_tree
            .sections
            .get(&Route::root())
            .map(|root| {
                (
                    root.title.to_string(),
                    root.description.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_else(|| ("Untitled".to_string(), String::new()));
        let base_url = crate::config::global_config()
            .map(|c| c.base_url.clone())
            .unwrap_or_else(|| "/".to_string());
        config_map.insert(
            VString::from("title"),
            facet_value::Value::from(site_title.as_str()),
        );
        config_map.insert(
            VString::from("description"),
            facet_value::Value::from(site_description.as_str()),
        );
        config_map.insert(
            VString::from("base_url"),
            facet_value::Value::from(base_url.as_str()),
        );
        ctx.set("config", facet_value::Value::from(config_map));

        // Add current_path
        ctx.set("current_path", facet_value::Value::from(route.as_str()));

        // Check if it's a section or page and add appropriate data
        if let Some(section) = site_tree.sections.get(&route) {
            let section_value = crate::render::section_to_value(section, &site_tree, &base_url);
            ctx.set("section", section_value);
            ctx.set("page", facet_value::Value::NULL);
        } else if let Some(page) = site_tree.pages.get(&route) {
            let page_value = crate::render::page_to_value(page, &site_tree);
            ctx.set("page", page_value);

            // Add parent section
            if let Some(section) = site_tree.sections.get(&page.section_route) {
                let section_value = crate::render::section_to_value(section, &site_tree, &base_url);
                ctx.set("section", section_value);
            }
        }

        // Add site tree info
        if let Some(root) = site_tree.sections.get(&Route::root()) {
            let root_value = crate::render::section_to_value(root, &site_tree, &base_url);
            ctx.set("root", root_value);
        }

        // Load data files
        let raw_data = crate::queries::load_all_data_raw(&snapshot)
            .await
            .unwrap_or_default();
        let data_value = crate::data::parse_raw_data_files(&raw_data);
        ctx.set("data", data_value);

        // Evaluate the expression
        match gingembre::eval_expression(expression, &ctx).await {
            Ok(value) => Ok(value_to_scope_value(&value)),
            Err(e) => {
                // Convert ANSI error to HTML for display in devtools
                let ansi_error = format!("{:?}", e);
                Err(crate::error_pages::ansi_to_html(&ansi_error))
            }
        }
    }

    /// Find content for RPC serving (returns protocol ServeContent type)
    ///
    /// This wraps find_content and converts the result to the protocol's ServeContent.
    pub async fn find_content_for_rpc(&self, path: &str) -> cell_http_proto::ServeContent {
        use cell_http_proto::ServeContent as RpcServeContent;

        // Get current generation
        let generation = self.current_generation();

        match self.find_content(path).await {
            Some(ServeContent::Html(html)) => {
                // Cache HTML for smart reload patching
                self.cache_html(path, &html);
                // Extract route from path
                let route = if path == "/" {
                    "/".to_string()
                } else {
                    path.trim_end_matches('/').to_string()
                };
                RpcServeContent::Html {
                    content: html,
                    route,
                    generation,
                }
            }
            Some(ServeContent::Css(css)) => {
                self.cache_css(path);
                RpcServeContent::Css {
                    content: css,
                    generation,
                }
            }
            Some(ServeContent::Static(bytes, mime)) => RpcServeContent::Static {
                content: bytes,
                mime: mime.to_string(),
                generation,
            },
            Some(ServeContent::StaticNoCache(bytes, mime)) => RpcServeContent::StaticNoCache {
                content: bytes,
                mime: mime.to_string(),
                generation,
            },
            None => {
                // 404 with similar routes - render the page on the host side
                let similar = self.find_similar_routes(path).await;
                let html = crate::error_pages::render_404_page(path, &similar);
                RpcServeContent::NotFound { html, generation }
            }
        }
    }

    /// Find routes similar to the requested path (for 404 suggestions)
    pub async fn find_similar_routes(&self, path: &str) -> Vec<(String, String)> {
        let snapshot = DatabaseSnapshot::from_database(&self.db).await;

        let site_tree = match build_tree(&snapshot).await {
            Ok(Ok(tree)) => tree,
            Ok(Err(_)) | Err(_) => return Vec::new(),
        };

        let requested = path.trim_matches('/').to_lowercase();
        let requested_parts: Vec<&str> = requested.split('/').collect();

        let mut candidates: Vec<(String, String, usize)> = Vec::new();

        for (route, section) in &site_tree.sections {
            let route_str = route.as_str().trim_matches('/').to_lowercase();
            let score = similarity_score(&requested, &requested_parts, &route_str);
            if score > 0 {
                candidates.push((
                    route.as_str().to_string(),
                    section.title.as_str().to_string(),
                    score,
                ));
            }
        }

        for (route, page) in &site_tree.pages {
            let route_str = route.as_str().trim_matches('/').to_lowercase();
            let score = similarity_score(&requested, &requested_parts, &route_str);
            if score > 0 {
                candidates.push((
                    route.as_str().to_string(),
                    page.title.as_str().to_string(),
                    score,
                ));
            }
        }

        // Sort by score (descending) and take top 5
        candidates.sort_by(|a, b| b.2.cmp(&a.2));
        candidates
            .into_iter()
            .take(5)
            .map(|(route, title, _score)| (route, title))
            .collect()
    }

    /// Find the redirect URL for a rule identifier.
    ///
    /// Returns the full URL (e.g., "/spec/core/#r-channel.id.allocation")
    /// if the rule exists, or None if not found.
    pub async fn find_rule_redirect(&self, rule_id: &str) -> Option<String> {
        let snapshot = DatabaseSnapshot::from_database(&self.db).await;

        let site_tree = match build_tree(&snapshot).await {
            Ok(Ok(tree)) => tree,
            Ok(Err(_)) | Err(_) => return None,
        };

        // Search for the rule in sections
        for (route, section) in &site_tree.sections {
            for rule in &section.rules {
                if rule.id == rule_id {
                    return Some(format!("{}#{}", route.as_str(), rule.anchor_id));
                }
            }
        }

        // Search for the rule in pages
        for (route, page) in &site_tree.pages {
            for rule in &page.rules {
                if rule.id == rule_id {
                    return Some(format!("{}#{}", route.as_str(), rule.anchor_id));
                }
            }
        }

        None
    }
}

/// Calculate similarity score between requested path and a route
fn similarity_score(requested: &str, requested_parts: &[&str], route: &str) -> usize {
    let mut score = 0;

    // Exact match gets highest score
    if requested == route {
        return 1000;
    }

    // Check for common path segments
    let route_parts: Vec<&str> = route.split('/').collect();
    for part in requested_parts {
        if route_parts.contains(part) {
            score += 10;
        }
    }

    // Check for substring matches
    if route.contains(requested) || requested.contains(route) {
        score += 20;
    }

    // Check for common prefix
    let common_prefix = requested
        .chars()
        .zip(route.chars())
        .take_while(|(a, b)| a == b)
        .count();
    if common_prefix > 2 {
        score += common_prefix;
    }

    // Penalize very long routes when looking for short paths
    if requested.len() < 10 && route.len() > 30 {
        score = score.saturating_sub(5);
    }

    score
}

/// Content types that can be served
enum ServeContent {
    Html(String),
    Css(String),
    Static(Vec<u8>, &'static str),
    /// Static file served at original path (no caching, for favicon etc.)
    StaticNoCache(Vec<u8>, &'static str),
}

fn devtools_js_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dodeca-devtools/pkg/dodeca_devtools.js")
}

fn devtools_wasm_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dodeca-devtools/pkg/dodeca_devtools_bg.wasm")
}

fn load_devtools_js() -> Option<String> {
    std::fs::read_to_string(devtools_js_path()).ok()
}

fn load_devtools_wasm() -> Option<Vec<u8>> {
    std::fs::read(devtools_wasm_path()).ok()
}

/// Compute a short hash for cache busting
fn compute_hash(data: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    format!("{:012x}", hasher.finish())
}

/// Get cache-busted devtools URLs
pub fn devtools_urls() -> (String, String) {
    use std::sync::LazyLock;
    static URLS: LazyLock<(String, String)> = LazyLock::new(|| {
        let js_hash = load_devtools_js()
            .map(|js| compute_hash(js.as_bytes()))
            .unwrap_or_else(|| "missing".to_string());
        let wasm_hash = load_devtools_wasm()
            .map(|bytes| compute_hash(&bytes))
            .unwrap_or_else(|| "missing".to_string());
        (
            format!("/_/{}.js", js_hash),
            format!("/_/{}.wasm", wasm_hash),
        )
    });
    URLS.clone()
}

/// Embedded JS snippets required by Dioxus WASM
const SNIPPETS: &[(&str, &str)] = &[
    // (
    //     "snippets/dioxus-cli-config-e5fab7f8a0eb9fbb/inline0.js",
    //     include_str!(
    //         "../../../crates/dodeca-devtools/pkg/snippets/dioxus-cli-config-e5fab7f8a0eb9fbb/inline0.js"
    //     ),
    // ),
    // (
    //     "snippets/dioxus-interpreter-js-267e64abc8a52eaa/inline0.js",
    //     include_str!(
    //         "../../../crates/dodeca-devtools/pkg/snippets/dioxus-interpreter-js-267e64abc8a52eaa/inline0.js"
    //     ),
    // ),
    // (
    //     "snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/patch_console.js",
    //     include_str!(
    //         "../../../crates/dodeca-devtools/pkg/snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/patch_console.js"
    //     ),
    // ),
    // (
    //     "snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/hydrate.js",
    //     include_str!(
    //         "../../../crates/dodeca-devtools/pkg/snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/hydrate.js"
    //     ),
    // ),
    // (
    //     "snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/set_attribute.js",
    //     include_str!(
    //         "../../../crates/dodeca-devtools/pkg/snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/set_attribute.js"
    //     ),
    // ),
    // (
    //     "snippets/dioxus-web-807c31b5ece9dd6a/inline0.js",
    //     include_str!(
    //         "../../../crates/dodeca-devtools/pkg/snippets/dioxus-web-807c31b5ece9dd6a/inline0.js"
    //     ),
    // ),
    // (
    //     "snippets/dioxus-web-807c31b5ece9dd6a/src/js/eval.js",
    //     include_str!(
    //         "../../../crates/dodeca-devtools/pkg/snippets/dioxus-web-807c31b5ece9dd6a/src/js/eval.js"
    //     ),
    // ),
];

/// Get devtools asset content by path (for RPC serving)
///
/// Returns (content, mime_type) if found.
pub fn get_devtools_asset(path: &str) -> Option<(Vec<u8>, &'static str)> {
    // Strip the /_/ prefix
    let asset_path = path.strip_prefix("/_/")?;

    // Check for snippets
    if let Some(snippet_path) = asset_path.strip_prefix("snippets/") {
        let full_path = format!("snippets/{}", snippet_path);
        for (p, content) in SNIPPETS {
            if full_path == *p {
                return Some((content.as_bytes().to_vec(), "application/javascript"));
            }
        }
        return None;
    }

    // Check for JS (cache-busted)
    if asset_path.ends_with(".js") {
        if let Some(js) = load_devtools_js() {
            return Some((
                rewrite_devtools_js(&js).into_bytes(),
                "application/javascript",
            ));
        }
        tracing::warn!(
            path = %devtools_js_path().display(),
            "devtools js missing"
        );
        return None;
    }

    // Check for WASM (cache-busted)
    if asset_path.ends_with(".wasm") {
        if let Some(bytes) = load_devtools_wasm() {
            return Some((bytes, "application/wasm"));
        }
        tracing::warn!(
            path = %devtools_wasm_path().display(),
            "devtools wasm missing"
        );
        return None;
    }

    None
}

/// Get search file content by path (for RPC serving)
pub fn get_search_file_content(
    search_files: &RwLock<HashMap<String, Vec<u8>>>,
    path: &str,
) -> Option<Vec<u8>> {
    let files = search_files.read().ok()?;
    files.get(path).cloned()
}

/// Rewrite relative snippet imports to absolute paths
fn rewrite_devtools_js(js: &str) -> String {
    // The generated JS has imports like:
    //   import { X } from './snippets/foo/bar.js';
    // We need to rewrite them to absolute paths:
    //   import { X } from '/_/snippets/foo/bar.js';
    js.replace("from './snippets/", "from '/_/snippets/")
}

/// Guess MIME type from file extension
pub fn mime_from_extension(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("eot") => "application/vnd.ms-fontobject",
        Some("xml") => "application/xml",
        Some("txt") => "text/plain; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        Some("jxl") => "image/jxl",
        Some("wasm") => "application/wasm",
        // Pagefind-specific extensions
        Some("pf_index") | Some("pf_meta") | Some("pagefind") => "application/octet-stream",
        _ => "application/octet-stream",
    }
}
