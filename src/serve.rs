//! HTTP server that serves content directly from the Salsa database
//!
//! No files are read from disk - everything is queried from Salsa on demand.
//! This enables instant incremental rebuilds with zero disk I/O.

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
use crate::render::{RenderOptions, inject_livereload};
use crate::types::Route;
use crate::image::{InputFormat, OutputFormat, add_width_suffix};
use std::collections::HashSet;

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

/// Message types for livereload WebSocket
#[derive(Clone, Debug)]
pub enum LiveReloadMsg {
    /// Full page reload (fallback)
    Reload,
    /// Patches for a specific route (serialized with postcard)
    Patches { route: String, data: Vec<u8> },
    /// CSS update (new cache-busted path)
    CssUpdate { path: String },
}

/// Summarize patch operations for logging
fn summarize_patches(patches: &[crate::html_diff::Patch]) -> String {
    use crate::html_diff::Patch;

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
    /// Uses Mutex instead of RwLock because Database contains RefCell (not Sync)
    pub db: Mutex<Database>,
    /// Source files (for creating registries)
    pub sources: RwLock<Vec<SourceFile>>,
    /// Template files (for creating registries)
    pub templates: RwLock<Vec<TemplateFile>>,
    /// SASS files (for creating registries)
    pub sass_files: RwLock<Vec<SassFile>>,
    /// Static files (in Salsa)
    pub static_files: RwLock<Vec<StaticFile>>,
    /// Data files (for template variables)
    pub data_files: RwLock<Vec<DataFile>>,
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
}

impl SiteServer {
    pub fn new(render_options: RenderOptions, stable_assets: Vec<String>) -> Self {
        let (livereload_tx, _) = broadcast::channel(16);
        Self {
            db: Mutex::new(Database::new()),
            sources: RwLock::new(Vec::new()),
            templates: RwLock::new(Vec::new()),
            sass_files: RwLock::new(Vec::new()),
            static_files: RwLock::new(Vec::new()),
            data_files: RwLock::new(Vec::new()),
            search_files: RwLock::new(HashMap::new()),
            livereload_tx,
            render_options,
            html_cache: RwLock::new(HashMap::new()),
            css_cache: RwLock::new(None),
            stable_assets,
        }
    }

    /// Check if a path is configured as a stable asset
    fn is_stable_asset(&self, path: &str) -> bool {
        self.stable_assets.iter().any(|p| p == path)
    }

    /// Notify all connected browsers to reload
    /// Computes patches for all cached routes and sends them
    pub fn trigger_reload(&self) {
        use crate::html_diff::{parse_html, diff, serialize_patches};

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
                tracing::info!("🎨 CSS changed: {}", path);
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
            if !css_changed {
                tracing::info!("🔄 No cached routes, sending full reload");
                let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
            }
            return;
        }

        let mut any_html_changed = false;
        for route in cached_routes {
            // Get old HTML from cache
            let old_html = {
                let cache = self.html_cache.read().unwrap();
                cache.get(&route).cloned()
            };

            // Get new HTML (re-render)
            let new_html = self.find_content(&route).and_then(|c| match c {
                ServeContent::Html(html) => Some(html),
                _ => None,
            });

            if let (Some(old), Some(new)) = (old_html, new_html.clone()) {
                if old != new {
                    any_html_changed = true;

                    // Update cache
                    {
                        let mut cache = self.html_cache.write().unwrap();
                        if let Some(html) = &new_html {
                            cache.insert(route.clone(), html.clone());
                        }
                    }

                    // Try to parse both DOMs
                    let old_dom = parse_html(&old);
                    let new_dom = parse_html(&new);

                    match (old_dom, new_dom) {
                        (Some(old_dom), Some(new_dom)) => {
                            let diff_result = diff(&old_dom, &new_dom);

                            if diff_result.patches.is_empty() {
                                // DOM structure identical but HTML differs (whitespace/comments?)
                                tracing::info!(
                                    "🔄 {} - HTML changed but DOM identical, full reload",
                                    route
                                );
                                let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
                            } else {
                                // Summarize patch operations
                                let summary = summarize_patches(&diff_result.patches);

                                match serialize_patches(&diff_result.patches) {
                                    Ok(data) => {
                                        tracing::info!(
                                            "✨ {} - patching: {} ({} bytes)",
                                            route, summary, data.len()
                                        );
                                        let _ = self.livereload_tx.send(LiveReloadMsg::Patches {
                                            route: route.clone(),
                                            data,
                                        });
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "🔄 {} - patch serialization failed: {}, full reload",
                                            route, e
                                        );
                                        let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
                                    }
                                }
                            }
                        }
                        (None, _) => {
                            tracing::warn!(
                                "🔄 {} - failed to parse old HTML, full reload",
                                route
                            );
                            let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
                        }
                        (_, None) => {
                            tracing::warn!(
                                "🔄 {} - failed to parse new HTML, full reload",
                                route
                            );
                            let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
                        }
                    }
                }
            }
        }

        // If neither HTML nor CSS changed, send a generic reload (for static assets, etc.)
        if !any_html_changed && !css_changed {
            tracing::info!("🔄 No HTML/CSS changes detected, refreshing for static assets");
            let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
        }
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
        let db = self.db.lock().ok()?;
        let sources = self.sources.read().ok()?;
        let templates = self.templates.read().ok()?;
        let sass_files = self.sass_files.read().ok()?;
        let static_files_vec = self.static_files.read().ok()?;
        let data_files_vec = self.data_files.read().ok()?;

        let source_registry = SourceRegistry::new(&*db, sources.clone());
        let template_registry = TemplateRegistry::new(&*db, templates.clone());
        let sass_registry = SassRegistry::new(&*db, sass_files.clone());
        let static_registry = StaticRegistry::new(&*db, static_files_vec.clone());
        let data_registry = DataRegistry::new(&*db, data_files_vec.clone());

        css_output(
            &*db,
            source_registry,
            template_registry,
            sass_registry,
            static_registry,
            data_registry,
        )
        .map(|css| format!("/{}", css.cache_busted_path))
    }

    /// Load cached query results from disk
    pub fn load_cache(&self, cache_path: &std::path::Path) -> Result<()> {
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
                Ok(())
            }
            Err(_) => {
                tracing::warn!("Cache incompatible (schema changed), deleting");
                let _ = std::fs::remove_file(cache_path);
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

        tracing::info!("Saved cache ({})", format_bytes(data.len()));
        Ok(())
    }

    /// Find content for a given path using lazy Salsa queries
    fn find_content(&self, path: &str) -> Option<ServeContent> {
        let db = self.db.lock().ok()?;
        let sources = self.sources.read().ok()?;
        let templates = self.templates.read().ok()?;
        let sass_files = self.sass_files.read().ok()?;
        let static_files_vec = self.static_files.read().ok()?;
        let data_files_vec = self.data_files.read().ok()?;

        let source_registry = SourceRegistry::new(&*db, sources.clone());
        let template_registry = TemplateRegistry::new(&*db, templates.clone());
        let sass_registry = SassRegistry::new(&*db, sass_files.clone());
        let static_registry = StaticRegistry::new(&*db, static_files_vec.clone());
        let data_registry = DataRegistry::new(&*db, data_files_vec.clone());

        // Get known routes for dead link detection (only in dev mode)
        let known_routes: Option<HashSet<String>> = if self.render_options.livereload {
            let site_tree = build_tree(&*db, source_registry);
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
            &*db,
            route,
            source_registry,
            template_registry,
            sass_registry,
            static_registry,
            data_registry,
        ) {
            let html = inject_livereload(&html, self.render_options, known_routes.as_ref());
            return Some(ServeContent::Html(html));
        }

        // 2. Try to serve CSS (check if path matches cache-busted CSS path)
        if let Some(css) = css_output(&*db, source_registry, template_registry, sass_registry, static_registry, data_registry) {
            let css_url = format!("/{}", css.cache_busted_path);
            if path == css_url {
                return Some(ServeContent::Css(css.content));
            }
        }

        // 3. Try to serve static files (match cache-busted paths)
        for file in static_registry.files(&*db) {
            let original_path = file.path(&*db).as_str();

            // Check if this is a processable image
            if InputFormat::is_processable(original_path) {
                use crate::cas::ImageVariantKey;
                use crate::queries::{image_metadata, image_input_hash};

                // Get metadata and input hash (fast - no encoding)
                let Some(metadata) = image_metadata(&*db, *file) else { continue };
                let input_hash = image_input_hash(&*db, *file);

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
                        if let Some(processed) = process_image(&*db, *file) {
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
                        if let Some(processed) = process_image(&*db, *file) {
                            if let Some(variant) = processed.webp_variants.iter().find(|v| v.width == width) {
                                return Some(ServeContent::Static(variant.data.clone(), "image/webp"));
                            }
                        }
                    }
                }
            } else {
                // Non-image static file
                let output = static_file_output(&*db, *file, source_registry, template_registry, sass_registry, static_registry, data_registry);
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

    /// Find routes similar to the requested path (for 404 suggestions)
    fn find_similar_routes(&self, path: &str) -> Vec<(String, String)> {
        let db = match self.db.lock() {
            Ok(db) => db,
            Err(_) => return Vec::new(),
        };
        let sources = match self.sources.read() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let source_registry = SourceRegistry::new(&*db, sources.clone());
        let site_tree = build_tree(&*db, source_registry);

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
        let html = inject_livereload(&html, server.render_options, None);
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
            .body(Body::from(html))
            .unwrap();
    }

    StatusCode::NOT_FOUND.into_response()
}

/// WebSocket handler for live reload
async fn livereload_handler(
    ws: WebSocketUpgrade,
    State(server): State<Arc<SiteServer>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_livereload_socket(socket, server))
}

async fn handle_livereload_socket(socket: WebSocket, server: Arc<SiteServer>) {
    let (mut sender, mut receiver) = socket.split();
    let mut reload_rx = server.livereload_tx.subscribe();

    // Track the current route this client is viewing
    let mut current_route: Option<String> = None;

    tracing::info!("🔌 Browser connected for live reload");

    // Send initial connection confirmation
    let _ = sender.send(Message::Text("connected".into())).await;

    loop {
        tokio::select! {
            result = reload_rx.recv() => {
                match result {
                    Ok(LiveReloadMsg::Reload) => {
                        if sender.send(Message::Text("reload".into())).await.is_err() {
                            break;
                        }
                    }
                    Ok(LiveReloadMsg::Patches { route, data }) => {
                        // Only send patches if client is viewing this route
                        // (or if we don't know their route yet, send anyway)
                        if current_route.as_ref().is_none_or(|r| r == &route) {
                            if sender.send(Message::Binary(data.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(LiveReloadMsg::CssUpdate { path }) => {
                        // Send CSS update as text message with css: prefix
                        if sender.send(Message::Text(format!("css:{}", path).into())).await.is_err() {
                            break;
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
                    Some(Ok(Message::Text(text))) => {
                        // Client can send its current route
                        if let Some(route) = text.strip_prefix("route:") {
                            tracing::info!("🔌 Browser viewing {}", route);
                            current_route = Some(route.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    tracing::info!("🔌 Browser disconnected");
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

/// Embedded WASM client files (built by build.rs)
const LIVERELOAD_JS: &str = include_str!("../crates/livereload-client/pkg/livereload_client.js");
const LIVERELOAD_WASM: &[u8] = include_bytes!("../crates/livereload-client/pkg/livereload_client_bg.wasm");

/// Handler for livereload WASM JS module
async fn livereload_js_handler() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/javascript")
        .body(Body::from(LIVERELOAD_JS))
        .unwrap()
}

/// Handler for livereload WASM binary
async fn livereload_wasm_handler() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/wasm")
        .body(Body::from(LIVERELOAD_WASM.to_vec()))
        .unwrap()
}

/// Build the axum router
pub fn build_router(server: Arc<SiteServer>) -> Router {
    Router::new()
        .route("/__livereload", get(livereload_handler))
        .route("/__livereload.js", get(livereload_js_handler))
        .route("/__livereload.wasm", get(livereload_wasm_handler))
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
