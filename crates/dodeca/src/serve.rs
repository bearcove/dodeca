//! HTTP server that serves content directly from the Salsa database
//!
//! No files are read from disk - everything is queried from Salsa on demand.
//! This enables instant incremental rebuilds with zero disk I/O.

/// Salsa cache version - bump this when making incompatible changes to Salsa inputs/queries
pub const SALSA_CACHE_VERSION: u32 = 4;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};

use color_eyre::Result;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tokio::sync::{broadcast, watch};

use crate::db::{
    Database, DataFile, DataRegistry, SassFile, SassRegistry, SourceFile, SourceRegistry,
    StaticFile, StaticRegistry, TemplateFile, TemplateRegistry,
};
use crate::error_pages::render_404_page;
use crate::queries::{css_output, serve_html, static_file_output, process_image, build_tree};
use crate::render::{RenderOptions, inject_livereload_with_build_info};
use crate::types::Route;
use crate::image::{InputFormat, OutputFormat, add_width_suffix};
use std::collections::HashSet;

use dodeca_protocol::{ClientMessage, ServerMessage, ScopeEntry, ScopeValue, facet_postcard};
use facet_value::DestructuredRef;

/// Format bytes as human-readable size (KB, MB, GB)
fn format_bytes(bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = KB * 1024;
    const GB: usize = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

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
            ScopeValue::Array { length: len, preview }
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
        DestructuredRef::Object(obj) => {
            obj.iter()
                .map(|(key, val)| ScopeEntry {
                    name: key.to_string(),
                    value: value_to_scope_value(val),
                    expandable: value_is_expandable(val),
                })
                .collect()
        }
        DestructuredRef::Array(arr) => {
            arr.iter()
                .enumerate()
                .map(|(idx, val)| ScopeEntry {
                    name: idx.to_string(),
                    value: value_to_scope_value(val),
                    expandable: value_is_expandable(val),
                })
                .collect()
        }
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
#[derive(Clone, Debug)]
pub enum LiveReloadMsg {
    /// Full page reload (fallback)
    Reload,
    /// Patches for a specific route
    Patches { route: String, patches: Vec<dodeca_protocol::Patch> },
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
    /// The Salsa database - all queries go through here
    /// Uses Mutex because Database contains RefCell (not Sync).
    /// For queries, we use snapshot pattern: lock, clone, release, then query the clone.
    pub db: Mutex<Database>,
    /// Source registry (Salsa input - update this to invalidate queries)
    pub source_registry: SourceRegistry,
    /// Template registry (Salsa input - update this to invalidate queries)
    pub template_registry: TemplateRegistry,
    /// SASS registry (Salsa input - update this to invalidate queries)
    pub sass_registry: SassRegistry,
    /// Static file registry (Salsa input - update this to invalidate queries)
    pub static_registry: StaticRegistry,
    /// Data file registry (Salsa input - update this to invalidate queries)
    pub data_registry: DataRegistry,
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
}

impl SiteServer {
    pub fn new(render_options: RenderOptions, stable_assets: Vec<String>) -> Self {
        let (livereload_tx, _) = broadcast::channel(16);
        let db = Database::new();

        // Create empty registries as Salsa inputs
        let source_registry = SourceRegistry::new(&db, Vec::new());
        let template_registry = TemplateRegistry::new(&db, Vec::new());
        let sass_registry = SassRegistry::new(&db, Vec::new());
        let static_registry = StaticRegistry::new(&db, Vec::new());
        let data_registry = DataRegistry::new(&db, Vec::new());

        Self {
            db: Mutex::new(db),
            source_registry,
            template_registry,
            sass_registry,
            static_registry,
            data_registry,
            search_files: RwLock::new(HashMap::new()),
            livereload_tx,
            render_options,
            html_cache: RwLock::new(HashMap::new()),
            css_cache: RwLock::new(None),
            stable_assets,
            current_errors: RwLock::new(HashMap::new()),
            code_execution_results: RwLock::new(Vec::new()),
        }
    }

    /// Update cached code execution results
    pub fn set_code_execution_results(&self, results: Vec<crate::db::CodeExecutionResult>) {
        *self.code_execution_results.write().unwrap() = results;
    }

    /// Get current error for a route (if any)
    pub fn get_error(&self, route: &str) -> Option<dodeca_protocol::ErrorInfo> {
        self.current_errors.read().unwrap().get(route).cloned()
    }

    /// Check if a path is configured as a stable asset
    fn is_stable_asset(&self, path: &str) -> bool {
        self.stable_assets.iter().any(|p| p == path)
    }

    /// Update the source registry with a new list of sources
    /// This invalidates all Salsa queries that depend on sources
    pub fn set_sources(&self, sources: Vec<SourceFile>) {
        use salsa::Setter;
        let mut db = self.db.lock().unwrap();
        self.source_registry.set_sources(&mut *db).to(sources);
    }

    /// Update the template registry with a new list of templates
    pub fn set_templates(&self, templates: Vec<TemplateFile>) {
        use salsa::Setter;
        let mut db = self.db.lock().unwrap();
        self.template_registry.set_templates(&mut *db).to(templates);
    }

    /// Update the sass registry with a new list of sass files
    pub fn set_sass_files(&self, files: Vec<SassFile>) {
        use salsa::Setter;
        let mut db = self.db.lock().unwrap();
        self.sass_registry.set_files(&mut *db).to(files);
    }

    /// Update the static registry with a new list of static files
    pub fn set_static_files(&self, files: Vec<StaticFile>) {
        use salsa::Setter;
        let mut db = self.db.lock().unwrap();
        self.static_registry.set_files(&mut *db).to(files);
    }

    /// Update the data registry with a new list of data files
    pub fn set_data_files(&self, files: Vec<DataFile>) {
        use salsa::Setter;
        let mut db = self.db.lock().unwrap();
        self.data_registry.set_files(&mut *db).to(files);
    }

    /// Get a clone of the current sources (for modification)
    pub fn get_sources(&self) -> Vec<SourceFile> {
        let db = self.db.lock().unwrap();
        self.source_registry.sources(&*db).clone()
    }

    /// Get a clone of the current templates (for modification)
    pub fn get_templates(&self) -> Vec<TemplateFile> {
        let db = self.db.lock().unwrap();
        self.template_registry.templates(&*db).clone()
    }

    /// Get a clone of the current sass files (for modification)
    pub fn get_sass_files(&self) -> Vec<SassFile> {
        let db = self.db.lock().unwrap();
        self.sass_registry.files(&*db).clone()
    }

    /// Get a clone of the current static files (for modification)
    pub fn get_static_files(&self) -> Vec<StaticFile> {
        let db = self.db.lock().unwrap();
        self.static_registry.files(&*db).clone()
    }

    /// Notify all connected browsers to reload
    /// Computes patches for all cached routes and sends them
    pub fn trigger_reload(&self) {
        use crate::plugins::diff_html_plugin;

        // Check for CSS changes first
        let old_css_path = {
            let cache = self.css_cache.read().unwrap();
            cache.clone()
        };
        let new_css_path = self.get_current_css_path();
        let css_changed = old_css_path != new_css_path;

        if css_changed {
            // Update CSS cache
            if let Some(ref path) = new_css_path {
                self.cache_css(path);
            }

            if let Some(ref path) = new_css_path {
                tracing::info!("CSS changed: {}", path);
                let _ = self.livereload_tx.send(LiveReloadMsg::CssUpdate {
                    path: path.clone(),
                });
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

        tracing::debug!("trigger_reload: checking {} cached routes", cached_routes.len());

        for route in cached_routes {
            // Get old HTML from cache
            let old_html = {
                let cache = self.html_cache.read().unwrap();
                cache.get(&route).cloned()
            };

            // Get new HTML (re-render) - this creates a fresh snapshot
            tracing::debug!("trigger_reload: re-rendering {}", route);
            let new_html = self.find_content(&route).and_then(|c| match c {
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
                    route, old_has_error, new_has_error, old != new
                );

                if old != new {
                    // If the new HTML is an error page, don't patch it in
                    // The error has already been sent to devtools via LiveReloadMsg::Error
                    // and we want to keep the old working HTML in cache
                    if new_has_error {
                        tracing::info!(
                            "🔴 {} - template error detected in trigger_reload",
                            route
                        );
                        continue;
                    }

                    // Update cache
                    {
                        let mut cache = self.html_cache.write().unwrap();
                        if let Some(html) = &new_html {
                            cache.insert(route.clone(), html.clone());
                        }
                    }

                    // Try to diff using the plugin
                    match diff_html_plugin(&old, &new) {
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

                                tracing::info!(
                                    "{} - patching: {} ({} patches)",
                                    route, summary, patch_count
                                );
                                let _ = self.livereload_tx.send(LiveReloadMsg::Patches {
                                    route: route.clone(),
                                    patches: diff_result.patches,
                                });
                            }
                        }
                        None => {
                            // Plugin not available or failed - fall back to full reload
                            tracing::debug!(
                                "{} - html_diff plugin not available, sending full reload",
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
    fn get_current_css_path(&self) -> Option<String> {
        // Snapshot pattern: read lock, clone, release, then query
        let db = self.db.lock().ok()?.clone();

        css_output(
            &db,
            self.source_registry,
            self.template_registry,
            self.sass_registry,
            self.static_registry,
            self.data_registry,
        )
        .map(|css| format!("/{}", css.cache_busted_path))
    }

    /// Load cached query results from disk
    pub fn load_cache(&self, cache_path: &std::path::Path) -> Result<()> {
        // Check version file first - if missing or mismatched, delete the cache
        let version_path = cache_path.with_extension("version");
        let version_ok = if version_path.exists() {
            match std::fs::read_to_string(&version_path) {
                Ok(v) => v.trim().parse::<u32>().ok() == Some(SALSA_CACHE_VERSION),
                Err(_) => false,
            }
        } else {
            false
        };

        if !version_ok {
            if cache_path.exists() {
                tracing::info!(
                    "Salsa cache version mismatch (expected v{}), deleting stale cache",
                    SALSA_CACHE_VERSION
                );
                let _ = std::fs::remove_file(cache_path);
            }
            return Ok(());
        }

        if !cache_path.exists() {
            tracing::info!("No cache file found, starting fresh");
            return Ok(());
        }

        let data = std::fs::read(cache_path)?;

        // Try to deserialize - if it fails (e.g., schema changed), delete the corrupt cache
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut db = self.db.lock().unwrap();
            let mut deserializer = postcard::Deserializer::from_bytes(&data);
            <dyn salsa::Database>::deserialize(&mut *db, &mut deserializer)
        }));

        match result {
            Ok(Ok(())) => {
                tracing::info!("Loaded cache ({})", format_bytes(data.len()));
                Ok(())
            }
            Ok(Err(e)) => {
                tracing::warn!("Cache corrupted, deleting: {e}");
                let _ = std::fs::remove_file(cache_path);
                let _ = std::fs::remove_file(&version_path);
                Ok(())
            }
            Err(_) => {
                tracing::warn!("Cache incompatible (schema changed), deleting");
                let _ = std::fs::remove_file(cache_path);
                let _ = std::fs::remove_file(&version_path);
                Ok(())
            }
        }
    }

    /// Save cached query results to disk
    pub fn save_cache(&self, cache_path: &std::path::Path) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let data = postcard::to_allocvec(&<dyn salsa::Database>::as_serialize(&mut *db))?;

        // Write atomically via temp file
        let temp_path = cache_path.with_extension("tmp");
        std::fs::write(&temp_path, &data)?;
        std::fs::rename(&temp_path, cache_path)?;

        // Write version file
        let version_path = cache_path.with_extension("version");
        std::fs::write(&version_path, SALSA_CACHE_VERSION.to_string())?;

        tracing::info!("Saved cache ({})", format_bytes(data.len()));
        Ok(())
    }

    /// Find content for a given path using lazy Salsa queries
    fn find_content(&self, path: &str) -> Option<ServeContent> {
        // Snapshot pattern: read lock, clone, release, then query
        let db = self.db.lock().ok()?.clone();

        // Get known routes for dead link detection (only in dev mode)
        let known_routes: Option<HashSet<String>> = if self.render_options.livereload {
            let site_tree = build_tree(&db, self.source_registry);
            let routes: HashSet<String> = site_tree.sections.keys()
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
        if let Some(html) = serve_html(
            &db,
            route,
            self.source_registry,
            self.template_registry,
            self.sass_registry,
            self.static_registry,
            self.data_registry,
        ) {
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
                    snapshot_id: format!("error-{}", std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()),
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

            let code_results = self.code_execution_results.read().unwrap();
            let html = inject_livereload_with_build_info(&html, self.render_options, known_routes.as_ref(), &code_results);
            return Some(ServeContent::Html(html));
        }

        // 2. Try to serve CSS (check if path matches cache-busted CSS path)
        if let Some(css) = css_output(&db, self.source_registry, self.template_registry, self.sass_registry, self.static_registry, self.data_registry) {
            let css_url = format!("/{}", css.cache_busted_path);
            if path == css_url {
                return Some(ServeContent::Css(css.content));
            }
        }

        // 3. Try to serve static files (match cache-busted paths)
        for file in self.static_registry.files(&db) {
            let original_path = file.path(&db).as_str();

            // Check if this is a processable image
            if InputFormat::is_processable(original_path) {
                use crate::cas::ImageVariantKey;
                use crate::queries::{image_metadata, image_input_hash};

                // Get metadata and input hash (fast - no encoding)
                let Some(metadata) = image_metadata(&db, *file) else { continue };
                let input_hash = image_input_hash(&db, *file);

                // Check each possible variant URL
                for &width in &metadata.variant_widths {
                    // Check JXL variant
                    let jxl_base = crate::image::change_extension(original_path, OutputFormat::Jxl.extension());
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
                    let jxl_cache_busted = format!("{}.{}.jxl",
                        jxl_variant_path.trim_end_matches(".jxl"),
                        jxl_key.url_hash()
                    );
                    if path == format!("/{jxl_cache_busted}") {
                        // NOW process the image (lazy!)
                        if let Some(processed) = process_image(&db, *file) {
                            if let Some(variant) = processed.jxl_variants.iter().find(|v| v.width == width) {
                                return Some(ServeContent::Static(variant.data.clone(), "image/jxl"));
                            }
                        }
                    }

                    // Check WebP variant
                    let webp_base = crate::image::change_extension(original_path, OutputFormat::WebP.extension());
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
                    let webp_cache_busted = format!("{}.{}.webp",
                        webp_variant_path.trim_end_matches(".webp"),
                        webp_key.url_hash()
                    );
                    if path == format!("/{webp_cache_busted}") {
                        // NOW process the image (lazy!)
                        if let Some(processed) = process_image(&db, *file) {
                            if let Some(variant) = processed.webp_variants.iter().find(|v| v.width == width) {
                                return Some(ServeContent::Static(variant.data.clone(), "image/webp"));
                            }
                        }
                    }
                }
            } else {
                // Non-image static file
                let output = static_file_output(&db, *file, self.source_registry, self.template_registry, self.sass_registry, self.static_registry, self.data_registry);
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
    pub fn get_scope_for_route(&self, route_path: &str, path: &[String]) -> Vec<ScopeEntry> {
        use facet_value::{VObject, VString};

        // Snapshot pattern: lock, clone, release, then query the clone
        let db = match self.db.lock() {
            Ok(db) => db.clone(),
            Err(_) => return vec![],
        };

        let site_tree = build_tree(&db, self.source_registry);

        // Normalize route
        let route_str = if route_path == "/" {
            "/".to_string()
        } else {
            let trimmed = route_path.trim_end_matches('/');
            if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() }
        };
        let route = Route::new(route_str);

        // Build scope based on whether this is a section or page
        let mut scope = VObject::new();

        // Add config (same as build_render_context_base)
        let mut config_map = VObject::new();
        let (site_title, site_description) = site_tree
            .sections
            .get(&Route::root())
            .map(|root| (root.title.to_string(), root.description.clone().unwrap_or_default()))
            .unwrap_or_else(|| ("Untitled".to_string(), String::new()));
        config_map.insert(VString::from("title"), facet_value::Value::from(site_title.as_str()));
        config_map.insert(VString::from("description"), facet_value::Value::from(site_description.as_str()));
        config_map.insert(VString::from("base_url"), facet_value::Value::from("/"));
        scope.insert(VString::from("config"), facet_value::Value::from(config_map));

        // Add current_path
        scope.insert(VString::from("current_path"), facet_value::Value::from(route.as_str()));

        // Check if it's a section or page
        if let Some(section) = site_tree.sections.get(&route) {
            // Add section data
            let mut section_map = VObject::new();
            section_map.insert(VString::from("title"), facet_value::Value::from(section.title.as_str()));
            section_map.insert(VString::from("permalink"), facet_value::Value::from(section.route.as_str()));
            section_map.insert(VString::from("weight"), facet_value::Value::from(section.weight as i64));
            if let Some(ref desc) = section.description {
                section_map.insert(VString::from("description"), facet_value::Value::from(desc.as_str()));
            }
            section_map.insert(VString::from("extra"), section.extra.clone());

            // Count pages in this section
            let page_count = site_tree.pages.values()
                .filter(|p| p.section_route == section.route)
                .count();
            section_map.insert(VString::from("pages_count"), facet_value::Value::from(page_count as i64));

            scope.insert(VString::from("section"), facet_value::Value::from(section_map));
        } else if let Some(page) = site_tree.pages.get(&route) {
            // Add page data
            let mut page_map = VObject::new();
            page_map.insert(VString::from("title"), facet_value::Value::from(page.title.as_str()));
            page_map.insert(VString::from("permalink"), facet_value::Value::from(page.route.as_str()));
            page_map.insert(VString::from("weight"), facet_value::Value::from(page.weight as i64));
            page_map.insert(VString::from("extra"), page.extra.clone());
            page_map.insert(VString::from("headings_count"), facet_value::Value::from(page.headings.len() as i64));
            scope.insert(VString::from("page"), facet_value::Value::from(page_map));

            // Add parent section
            if let Some(section) = site_tree.sections.get(&page.section_route) {
                let mut section_map = VObject::new();
                section_map.insert(VString::from("title"), facet_value::Value::from(section.title.as_str()));
                section_map.insert(VString::from("permalink"), facet_value::Value::from(section.route.as_str()));
                scope.insert(VString::from("section"), facet_value::Value::from(section_map));
            }
        }

        // Add root section info
        if let Some(root) = site_tree.sections.get(&Route::root()) {
            let mut root_map = VObject::new();
            root_map.insert(VString::from("title"), facet_value::Value::from(root.title.as_str()));

            // Count total sections and pages
            let section_count = site_tree.sections.len();
            let page_count = site_tree.pages.len();
            root_map.insert(VString::from("sections_count"), facet_value::Value::from(section_count as i64));
            root_map.insert(VString::from("pages_count"), facet_value::Value::from(page_count as i64));

            scope.insert(VString::from("root"), facet_value::Value::from(root_map));
        }

        // Load actual data files
        let raw_data = crate::queries::load_all_data_raw(&db, self.data_registry);
        let data_value = crate::data::parse_raw_data_files(&raw_data);
        scope.insert(VString::from("data"), data_value);

        // Convert scope to entries
        let scope_value: facet_value::Value = scope.into();
        value_to_scope_entries(&scope_value, path)
    }

    /// Evaluate an expression against the scope for a route (for REPL)
    pub fn eval_expression_for_route(&self, route_path: &str, expression: &str) -> Result<ScopeValue, String> {
        use facet_value::{VObject, VString};

        // Snapshot pattern: lock, clone, release, then query the clone
        let db = match self.db.lock() {
            Ok(db) => db.clone(),
            Err(_) => return Err("Failed to acquire database lock".to_string()),
        };

        let site_tree = build_tree(&db, self.source_registry);

        // Normalize route
        let route_str = if route_path == "/" {
            "/".to_string()
        } else {
            let trimmed = route_path.trim_end_matches('/');
            if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() }
        };
        let route = Route::new(route_str);

        // Build gingembre Context with scope values
        let mut ctx = gingembre::Context::new();

        // Add config
        let mut config_map = VObject::new();
        let (site_title, site_description) = site_tree
            .sections
            .get(&Route::root())
            .map(|root| (root.title.to_string(), root.description.clone().unwrap_or_default()))
            .unwrap_or_else(|| ("Untitled".to_string(), String::new()));
        config_map.insert(VString::from("title"), facet_value::Value::from(site_title.as_str()));
        config_map.insert(VString::from("description"), facet_value::Value::from(site_description.as_str()));
        config_map.insert(VString::from("base_url"), facet_value::Value::from("/"));
        ctx.set("config", facet_value::Value::from(config_map));

        // Add current_path
        ctx.set("current_path", facet_value::Value::from(route.as_str()));

        // Check if it's a section or page and add appropriate data
        if let Some(section) = site_tree.sections.get(&route) {
            let section_value = crate::render::section_to_value(section, &site_tree);
            ctx.set("section", section_value);
            ctx.set("page", facet_value::Value::NULL);
        } else if let Some(page) = site_tree.pages.get(&route) {
            let page_value = crate::render::page_to_value(page, &site_tree);
            ctx.set("page", page_value);

            // Add parent section
            if let Some(section) = site_tree.sections.get(&page.section_route) {
                let section_value = crate::render::section_to_value(section, &site_tree);
                ctx.set("section", section_value);
            }
        }

        // Add site tree info
        if let Some(root) = site_tree.sections.get(&Route::root()) {
            let root_value = crate::render::section_to_value(root, &site_tree);
            ctx.set("root", root_value);
        }

        // Load data files
        let raw_data = crate::queries::load_all_data_raw(&db, self.data_registry);
        let data_value = crate::data::parse_raw_data_files(&raw_data);
        ctx.set("data", data_value);

        // Evaluate the expression
        match gingembre::eval_expression(expression, &ctx) {
            Ok(value) => Ok(value_to_scope_value(&value)),
            Err(e) => {
                // Convert ANSI error to HTML for display in devtools
                let ansi_error = format!("{:?}", e);
                Err(crate::error_pages::ansi_to_html(&ansi_error))
            }
        }
    }

    /// Find routes similar to the requested path (for 404 suggestions)
    fn find_similar_routes(&self, path: &str) -> Vec<(String, String)> {
        // Snapshot pattern: lock, clone, release, then query the clone
        let db = match self.db.lock() {
            Ok(db) => db.clone(),
            Err(_) => return Vec::new(),
        };

        let site_tree = build_tree(&db, self.source_registry);

        let requested = path.trim_matches('/').to_lowercase();
        let requested_parts: Vec<&str> = requested.split('/').collect();

        let mut candidates: Vec<(String, String, usize)> = Vec::new();

        for (route, section) in &site_tree.sections {
            let route_str = route.as_str().trim_matches('/').to_lowercase();
            let score = similarity_score(&requested, &requested_parts, &route_str);
            if score > 0 {
                candidates.push((route.as_str().to_string(), section.title.as_str().to_string(), score));
            }
        }

        for (route, page) in &site_tree.pages {
            let route_str = route.as_str().trim_matches('/').to_lowercase();
            let score = similarity_score(&requested, &requested_parts, &route_str);
            if score > 0 {
                candidates.push((route.as_str().to_string(), page.title.as_str().to_string(), score));
            }
        }

        // Sort by score (descending) and take top 5
        candidates.sort_by(|a, b| b.2.cmp(&a.2));
        candidates.into_iter()
            .take(5)
            .map(|(route, title, _score)| (route, title))
            .collect()
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
    let common_prefix = requested.chars()
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

/// Cache-Control header for cache-busted assets (1 year, immutable)
const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// Cache-Control header for HTML (must revalidate to get new asset URLs)
const CACHE_NO_CACHE: &str = "no-cache";

/// Handler for all content requests
async fn content_handler(State(server): State<Arc<SiteServer>>, request: Request) -> Response {
    let path = request.uri().path();

    // Check search files first (pagefind - not part of build_site, no cache control)
    {
        let search_files = server.search_files.read().unwrap();
        if let Some(content) = search_files.get(path) {
            let mime = mime_from_extension(path);
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(content.clone()))
                .unwrap();
        }
    }

    // Try to serve from build_site output (HTML, CSS, static files - all cache-busted)
    if let Some(content) = server.find_content(path) {
        return match content {
            ServeContent::Html(html) => {
                // Cache HTML for smart reload patching
                server.cache_html(path, &html);
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                    .body(Body::from(html))
                    .unwrap()
            }
            ServeContent::Css(css) => {
                // Cache CSS path for hot reload detection
                server.cache_css(path);
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
                    .header(header::CACHE_CONTROL, CACHE_IMMUTABLE)
                    .body(Body::from(css))
                    .unwrap()
            }
            ServeContent::Static(bytes, mime) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, CACHE_IMMUTABLE)
                .body(Body::from(bytes))
                .unwrap(),
            ServeContent::StaticNoCache(bytes, mime) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                .body(Body::from(bytes))
                .unwrap(),
        };
    }

    // 404 - serve custom page in dev mode with livereload
    if server.render_options.livereload {
        let similar_routes = server.find_similar_routes(path);
        let html = render_404_page(path, &similar_routes);
        // Inject livereload so the page auto-refreshes when the missing page is created
        let html = inject_livereload_with_build_info(&html, server.render_options, None, &[]);
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
            .body(Body::from(html))
            .unwrap();
    }

    StatusCode::NOT_FOUND.into_response()
}

/// WebSocket handler for devtools (live reload, error reporting, etc.)
async fn devtools_ws_handler(
    ws: WebSocketUpgrade,
    State(server): State<Arc<SiteServer>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_devtools_socket(socket, server))
}

async fn handle_devtools_socket(socket: WebSocket, server: Arc<SiteServer>) {
    let (mut sender, mut receiver) = socket.split();
    let mut reload_rx = server.livereload_tx.subscribe();

    // Track the current route this client is viewing
    let mut current_route: Option<String> = None;

    tracing::info!("Browser connected for live reload");

    loop {
        tokio::select! {
            result = reload_rx.recv() => {
                match result {
                    Ok(LiveReloadMsg::Reload) => {
                        let msg = ServerMessage::Reload;
                        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                            if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(LiveReloadMsg::Patches { route, patches }) => {
                        // Only send patches if client is viewing this route
                        // (or if we don't know their route yet, send anyway)
                        if current_route.as_ref().is_none_or(|r| r == &route) {
                            let msg = ServerMessage::Patches(patches);
                            if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                                if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(LiveReloadMsg::CssUpdate { path }) => {
                        let msg = ServerMessage::CssChanged { path };
                        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                            if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(LiveReloadMsg::Error { route, message, template, line, snapshot_id }) => {
                        tracing::info!(
                            "🔴 WebSocket: received LiveReloadMsg::Error for {}, forwarding to browser",
                            route
                        );
                        let msg = ServerMessage::Error(dodeca_protocol::ErrorInfo {
                            route: route.clone(),
                            message,
                            template,
                            line,
                            column: None,
                            source_snippet: None,
                            snapshot_id,
                            available_variables: vec![],
                        });
                        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                            tracing::debug!("🔴 WebSocket: sending error ({} bytes)", bytes.len());
                            if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                tracing::warn!("🔴 WebSocket: failed to send error to browser");
                                break;
                            }
                            tracing::info!("🔴 WebSocket: error sent to browser for {}", route);
                        }
                    }
                    Ok(LiveReloadMsg::ErrorResolved { route }) => {
                        let msg = ServerMessage::ErrorResolved { route };
                        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                            if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        // Binary messages are devtools protocol messages
                        if let Ok(msg) = facet_postcard::from_bytes::<ClientMessage>(&data) {
                            match msg {
                                ClientMessage::Route { path } => {
                                    tracing::info!("Browser viewing {}", path);
                                    current_route = Some(path.clone());

                                    // Send any existing error for this route
                                    if let Some(error_info) = server.get_error(&path) {
                                        let msg = ServerMessage::Error(error_info);
                                        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                                            if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::GetScope { request_id, snapshot_id: _, path } => {
                                    // Get scope for current route (snapshot_id is ignored for now)
                                    let route = current_route.clone().unwrap_or_else(|| "/".to_string());
                                    let scope_path = path.unwrap_or_default();
                                    let scope = server.get_scope_for_route(&route, &scope_path);

                                    tracing::info!(
                                        "[devtools] GetScope request {request_id}: route={}, path={:?}, entries={}",
                                        route, scope_path, scope.len()
                                    );

                                    let msg = ServerMessage::ScopeResponse {
                                        request_id,
                                        scope,
                                    };
                                    if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                                        if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                ClientMessage::Eval { request_id, snapshot_id: _, expression } => {
                                    // Evaluate expression against the current route's scope
                                    let route = current_route.clone().unwrap_or_else(|| "/".to_string());
                                    tracing::info!(
                                        "[devtools] Eval request {request_id}: route={}, expr={}",
                                        route, expression
                                    );

                                    let result = server.eval_expression_for_route(&route, &expression);
                                    let msg = ServerMessage::EvalResponse {
                                        request_id,
                                        result: result.into(),
                                    };
                                    if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                                        if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                ClientMessage::DismissError { route } => {
                                    tracing::info!("[devtools] Dismissing error for {}", route);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    tracing::info!("Browser disconnected");
}

/// Middleware to log HTTP requests with status code and latency
async fn log_requests(request: Request, next: Next) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let start = Instant::now();

    let response = next.run(request).await;

    let status = response.status().as_u16();
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Format: "METHOD /path -> STATUS in TIMEms" for highlight_message parser
    if status >= 500 {
        tracing::error!("{} {} -> {} in {:.1}ms", method, path, status, latency_ms);
    } else if status >= 400 {
        tracing::warn!("{} {} -> {} in {:.1}ms", method, path, status, latency_ms);
    } else {
        tracing::info!("{} {} -> {} in {:.1}ms", method, path, status, latency_ms);
    }

    response
}

/// Embedded devtools WASM client files (built by build.rs)
const DEVTOOLS_JS: &str = include_str!("../../../crates/dodeca-devtools/pkg/dodeca_devtools.js");
const DEVTOOLS_WASM: &[u8] =
    include_bytes!("../../../crates/dodeca-devtools/pkg/dodeca_devtools_bg.wasm");

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
        let js_hash = compute_hash(DEVTOOLS_JS.as_bytes());
        let wasm_hash = compute_hash(DEVTOOLS_WASM);
        (
            format!("/_/{}.js", js_hash),
            format!("/_/{}.wasm", wasm_hash),
        )
    });
    URLS.clone()
}

/// Embedded JS snippets required by Dioxus WASM
const SNIPPETS: &[(&str, &str)] = &[
    (
        "snippets/dioxus-cli-config-e5fab7f8a0eb9fbb/inline0.js",
        include_str!("../../../crates/dodeca-devtools/pkg/snippets/dioxus-cli-config-e5fab7f8a0eb9fbb/inline0.js"),
    ),
    (
        "snippets/dioxus-interpreter-js-267e64abc8a52eaa/inline0.js",
        include_str!("../../../crates/dodeca-devtools/pkg/snippets/dioxus-interpreter-js-267e64abc8a52eaa/inline0.js"),
    ),
    (
        "snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/patch_console.js",
        include_str!("../../../crates/dodeca-devtools/pkg/snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/patch_console.js"),
    ),
    (
        "snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/hydrate.js",
        include_str!("../../../crates/dodeca-devtools/pkg/snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/hydrate.js"),
    ),
    (
        "snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/set_attribute.js",
        include_str!("../../../crates/dodeca-devtools/pkg/snippets/dioxus-interpreter-js-267e64abc8a52eaa/src/js/set_attribute.js"),
    ),
    (
        "snippets/dioxus-web-807c31b5ece9dd6a/inline0.js",
        include_str!("../../../crates/dodeca-devtools/pkg/snippets/dioxus-web-807c31b5ece9dd6a/inline0.js"),
    ),
    (
        "snippets/dioxus-web-807c31b5ece9dd6a/src/js/eval.js",
        include_str!("../../../crates/dodeca-devtools/pkg/snippets/dioxus-web-807c31b5ece9dd6a/src/js/eval.js"),
    ),
];

/// Handler for devtools JS snippets
async fn devtools_snippet_handler(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Response {
    // The path comes in without the leading "snippets/" since that's part of the route
    let full_path = format!("snippets/{}", path);
    for (snippet_path, content) in SNIPPETS {
        if full_path == *snippet_path {
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/javascript")
                .body(Body::from(*content))
                .unwrap();
        }
    }
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Snippet not found"))
        .unwrap()
}

/// Rewrite relative snippet imports to absolute paths
fn rewrite_devtools_js() -> String {
    // The generated JS has imports like:
    //   import { X } from './snippets/foo/bar.js';
    // We need to rewrite them to absolute paths:
    //   import { X } from '/_/snippets/foo/bar.js';
    DEVTOOLS_JS.replace("from './snippets/", "from '/_/snippets/")
}

/// Handler for devtools assets (JS and WASM) - routes based on extension
async fn devtools_asset_handler(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Response {
    if path.ends_with(".js") {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/javascript")
            .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
            .body(Body::from(rewrite_devtools_js()))
            .unwrap()
    } else if path.ends_with(".wasm") {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/wasm")
            .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
            .body(Body::from(DEVTOOLS_WASM.to_vec()))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not found"))
            .unwrap()
    }
}

/// Build the axum router
pub fn build_router(server: Arc<SiteServer>) -> Router {
    Router::new()
        // Devtools WebSocket endpoint
        .route("/_/ws", get(devtools_ws_handler))
        // Dioxus JS snippets (dynamically imported by the WASM module)
        // Must come before the catch-all since snippets are under /_/
        .route("/_/snippets/{*path}", get(devtools_snippet_handler))
        // Cache-busted devtools assets (hash in filename, extension-based routing)
        .route("/_/{*path}", get(devtools_asset_handler))
        // Legacy endpoints for backwards compat
        .route("/__dodeca", get(devtools_ws_handler))
        .route("/__livereload", get(devtools_ws_handler))
        .fallback(content_handler)
        .with_state(server)
        .layer(middleware::from_fn(log_requests))
}

/// Bound listeners ready to serve
pub struct BoundListeners {
    pub listeners: Vec<tokio::net::TcpListener>,
    pub port: u16,
}

/// Bind to available port(s) on the given IPs.
/// - If `preferred` is Some(port), tries that exact port (returns error if unavailable)
/// - If `preferred` is None, tries 4000-4019, then lets OS choose (port 0)
pub async fn bind_listeners(ips: &[Ipv4Addr], preferred: Option<u16>) -> Result<BoundListeners> {
    use std::io::ErrorKind;

    let mut listeners = Vec::new();
    let mut actual_port = 0u16;

    for (i, ip) in ips.iter().enumerate() {
        let (listener, port) = if i == 0 {
            // First IP - find available port
            if let Some(port) = preferred {
                let addr = format!("{ip}:{port}");
                let listener = tokio::net::TcpListener::bind(&addr).await?;
                // If caller requested port 0, capture the OS-assigned port
                let actual = if port == 0 {
                    listener.local_addr()?.port()
                } else {
                    port
                };
                (listener, actual)
            } else {
                // Try 4000-4019, then port 0
                let mut bound = None;
                for port in 4000..4020 {
                    let addr = format!("{ip}:{port}");
                    match tokio::net::TcpListener::bind(&addr).await {
                        Ok(listener) => {
                            bound = Some((listener, port));
                            break;
                        }
                        Err(e) if e.kind() == ErrorKind::AddrInUse => continue,
                        Err(e) => return Err(e.into()),
                    }
                }
                match bound {
                    Some(b) => b,
                    None => {
                        // All preferred ports in use - let OS choose
                        let addr = format!("{ip}:0");
                        let listener = tokio::net::TcpListener::bind(&addr).await?;
                        let port = listener.local_addr()?.port();
                        (listener, port)
                    }
                }
            }
        } else {
            // Subsequent IPs use the same port as the first
            let addr = format!("{ip}:{actual_port}");
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            (listener, actual_port)
        };

        if i == 0 {
            actual_port = port;
        }
        listeners.push(listener);
    }

    Ok(BoundListeners { listeners, port: actual_port })
}

/// Run HTTP servers on pre-bound listeners
pub async fn run_on_listeners(
    server: Arc<SiteServer>,
    bound: BoundListeners,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    use tokio::task::JoinSet;

    let app = build_router(server);
    let mut join_set = JoinSet::new();

    for listener in bound.listeners {
        let app_clone = app.clone();
        let mut shutdown_rx_clone = shutdown_rx.clone();

        join_set.spawn(async move {
            let shutdown_future = async move {
                while !*shutdown_rx_clone.borrow() {
                    if shutdown_rx_clone.changed().await.is_err() {
                        break;
                    }
                }
            };

            axum::serve(listener, app_clone)
                .with_graceful_shutdown(shutdown_future)
                .await
        });
    }

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            result = join_set.join_next() => {
                if let Some(res) = result {
                    if let Err(e) = res {
                        eprintln!("Server task error: {e}");
                    }
                } else {
                    break;
                }
            }
        }
    }

    while (join_set.join_next().await).is_some() {}

    Ok(())
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
