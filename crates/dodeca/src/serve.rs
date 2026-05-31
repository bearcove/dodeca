//! HTTP server that serves content directly from the picante database
//!
//! No files are read from disk - everything is queried from picante on demand.
//! This enables instant incremental rebuilds with zero disk I/O.

/// Picante cache version - bump this when making incompatible changes to picante inputs/queries
pub const PICANTE_CACHE_VERSION: u32 = 6;

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use eyre::{Result, bail, eyre};
use hotmeal_server::LiveReloadServer;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{broadcast, watch};

use crate::db::{
    DataFile, DataRegistry, Database, DatabaseSnapshot, MarkdownRenderSettings, SassFile,
    SassRegistry, SourceFile, SourceRegistry, StaticFile, StaticRegistry, TemplateFile,
    TemplateRegistry,
};
use crate::image::{InputFormat, OutputFormat, add_width_suffix};
use crate::queries::{
    build_tree, css_output, process_image, render_page_markdown, serve_html, static_file_output,
};
use crate::render::{RenderOptions, inject_livereload_with_build_info};
use crate::types::Route;
use std::collections::HashSet;

use dodeca_protocol::{DeadLinkTarget, ScopeEntry, ScopeValue};
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
    /// Patches for a specific route (postcard-serialized blob)
    Patches { route: String, patches: Vec<u8> },
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

/// Hook the binary installs to run the authoring LSP **in process** for the
/// browser editor. Given the LSP side of an in-memory duplex (Content-Length
/// framed JSON-RPC), serve the LSP on it for the session's lifetime. `dodeca`
/// defines this trait; `ddc` — which depends on both `dodeca` and
/// `dodeca-authoring-lsp` — provides the impl, breaking the crate cycle by
/// inversion of control (no subprocess).
pub trait LspRunner: Send + Sync {
    fn serve(
        &self,
        transport: tokio::io::DuplexStream,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
}

/// [`AuthoringProjectProvider`](crate::authoring_model::AuthoringProjectProvider)
/// backed by a live `SiteServer` — snapshots its db + overlays open documents.
struct HostAuthoringProvider {
    server: Arc<SiteServer>,
}

impl crate::authoring_model::AuthoringProjectProvider for HostAuthoringProvider {
    fn project<'a>(
        &'a self,
        overlays: Vec<(String, String)>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = eyre::Result<crate::authoring_model::AuthoringProject>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { self.server.authoring_project_overlay(overlays).await })
    }
}

/// A project provider that reuses `server`'s built db, for the in-process LSP.
pub fn authoring_project_provider(
    server: Arc<SiteServer>,
) -> Arc<dyn crate::authoring_model::AuthoringProjectProvider> {
    Arc::new(HostAuthoringProvider { server })
}

/// Shared state for the dev server
pub struct SiteServer {
    /// The picante database - all queries go through here
    pub db: Arc<Database>,
    /// Live reload broadcast (legacy - will be removed)
    pub livereload_tx: broadcast::Sender<LiveReloadMsg>,
    /// Render options (dev mode, etc.)
    pub render_options: RenderOptions,
    /// Content directory used to resolve source paths from rendered HTML.
    source_root: Option<Utf8PathBuf>,
    /// Live reload server: caches HTML + head injections per route, computes patches
    live_reload: Mutex<LiveReloadServer>,
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
    /// Connected browsers (keyed by a unique ID for removal on disconnect)
    browsers: std::sync::Mutex<BrowserRegistry>,
    /// Git-backed sources as `(name, checkout dir)`, for the `/_dodeca/pull`
    /// webhook to `git pull` on demand.
    git_checkouts: RwLock<Vec<(String, Utf8PathBuf)>>,
    /// Status-page context: the resolved sources and the content port. Set once
    /// at serve startup; read by `status_html`.
    status_sources: RwLock<Vec<crate::config::ResolvedSource>>,
    site_port: RwLock<u16>,
    /// When the server started, for the status page's uptime.
    started: std::time::Instant,
    /// Live in-browser editing sessions, keyed by opaque token. Minted at the
    /// identity-bearing `GET /_dodeca/edit/<page>` load, presented on `edit_*`.
    edit_sessions: crate::edit_session::EditSessionStore,
    /// Local-only dev override (`ddc serve --dev-editor`): when set, this
    /// identity is treated as a verified editor, bypassing oauth2-proxy. Refused
    /// on non-loopback binds by the CLI. Never set in production.
    dev_editor: RwLock<Option<cell_http_proto::Identity>>,
    /// In-process authoring-LSP runner, injected by the binary (see [`LspRunner`]).
    lsp_runner: RwLock<Option<Arc<dyn LspRunner>>>,
}

/// Registry of connected browsers for direct event pushing.
///
/// Browsers are keyed by their roam `conn_id`, which is available in the
/// RPC context when they call `subscribe()`.
#[derive(Default)]
struct BrowserRegistry {
    browsers: HashMap<u64, RegisteredBrowser>,
}

/// A registered browser connection.
struct RegisteredBrowser {
    /// The route this browser is subscribed to (if any)
    route: Option<String>,
    /// Client for calling BrowserService::on_event()
    client: dodeca_protocol::BrowserServiceClient,
}

fn normalize_route(route: &str) -> String {
    if route == "/" {
        "/".to_string()
    } else {
        let trimmed = route.trim_end_matches('/');
        if trimmed.is_empty() {
            "/".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

impl SiteServer {
    pub fn new(
        render_options: RenderOptions,
        stable_assets: Vec<String>,
        source_root: Option<Utf8PathBuf>,
    ) -> Self {
        let (livereload_tx, _) = broadcast::channel(16);
        let db = Database::new(None);
        let (revision_tx, _) = watch::channel(crate::revision::RevisionState {
            generation: 0,
            status: crate::revision::RevisionStatus::Building,
            reason: Some("startup".to_string()),
            started_at: None,
        });

        let db = Arc::new(db);
        MarkdownRenderSettings::set(&*db, render_options.source_maps)
            .expect("failed to initialize markdown render settings");

        Self {
            db,
            livereload_tx,
            render_options,
            source_root,
            live_reload: Mutex::new(LiveReloadServer::new()),
            css_cache: RwLock::new(None),
            stable_assets,
            current_errors: RwLock::new(HashMap::new()),
            code_execution_results: RwLock::new(Vec::new()),
            revision_tx,
            browsers: std::sync::Mutex::new(BrowserRegistry::default()),
            git_checkouts: RwLock::new(Vec::new()),
            status_sources: RwLock::new(Vec::new()),
            site_port: RwLock::new(0),
            started: std::time::Instant::now(),
            edit_sessions: crate::edit_session::EditSessionStore::default(),
            dev_editor: RwLock::new(None),
            lsp_runner: RwLock::new(None),
        }
    }

    /// Set the local dev editor override (see the `dev_editor` field).
    pub fn set_dev_editor(&self, identity: Option<cell_http_proto::Identity>) {
        *self.dev_editor.write().unwrap() = identity;
    }

    /// Install the in-process authoring-LSP runner (see [`LspRunner`]).
    pub fn set_lsp_runner(&self, runner: Arc<dyn LspRunner>) {
        *self.lsp_runner.write().unwrap() = Some(runner);
    }

    /// Provide the status page its context (resolved sources + content port),
    /// known once the server has bound. The live bits (generation, errors, page
    /// counts) are read fresh each render.
    pub fn set_status_context(&self, sources: Vec<crate::config::ResolvedSource>, site_port: u16) {
        *self.status_sources.write().unwrap() = sources;
        *self.site_port.write().unwrap() = site_port;
    }

    /// Render the status page HTML from current server state. Pure string — the
    /// http cell serves it (HTTP stays out of this crate).
    pub fn status_html(&self) -> String {
        let sources = self.status_sources.read().unwrap().clone();
        let data = crate::status::StatusData {
            sources: &sources,
            source_keys: self.source_keys(),
            generation: self.current_generation(),
            error_routes: self.error_routes(),
            uptime_secs: self.started.elapsed().as_secs(),
            site_port: *self.site_port.read().unwrap(),
        };
        crate::status::render_status_html(&data)
    }

    /// Register the git-backed sources (`(name, checkout dir)`) the
    /// `/_dodeca/pull` webhook can pull. Called once at serve startup.
    pub fn set_git_checkouts(&self, checkouts: Vec<(String, Utf8PathBuf)>) {
        *self.git_checkouts.write().unwrap() = checkouts;
    }

    /// Handle a `/_dodeca/pull[/<name>]` webhook: `git pull --ff-only` the
    /// matching source checkout(s) in the background (the file watcher
    /// re-renders the pulled changes). Returns how many pulls were kicked off.
    pub fn pull_git_sources(&self, name: Option<&str>) -> usize {
        let checkouts = self.git_checkouts.read().unwrap().clone();
        let mut started = 0;
        for (source_name, checkout) in checkouts {
            if name.is_some_and(|target| target != source_name) {
                continue;
            }
            started += 1;
            tokio::spawn(async move {
                match tokio::process::Command::new("git")
                    .args(["-C", checkout.as_str(), "pull", "--ff-only"])
                    .status()
                    .await
                {
                    Ok(s) if !s.success() => {
                        tracing::warn!(checkout = %checkout, "webhook pull: `git pull` failed")
                    }
                    Err(e) => {
                        tracing::warn!(checkout = %checkout, error = %e, "webhook pull: spawn failed")
                    }
                    _ => tracing::info!(checkout = %checkout, "webhook pull: updated"),
                }
            });
        }
        started
    }

    pub async fn open_source_in_editor(&self, source_file: &str, line: u32) -> Result<()> {
        tracing::debug!(
            source_file,
            line,
            "open_source_in_editor: resolving source location"
        );

        let source_root = self
            .source_root
            .as_ref()
            .ok_or_else(|| eyre!("source root is not available in this server mode"))?;
        tracing::debug!(
            source_root = %source_root,
            source_file,
            "open_source_in_editor: using source root"
        );

        let source_path = Utf8Path::new(source_file);
        if source_path.is_absolute()
            || source_path
                .components()
                .any(|component| matches!(component, Utf8Component::ParentDir))
        {
            bail!("refusing to open source path outside the content directory: {source_file}");
        }

        let disk_path = source_root.join(source_path);
        let disk_path = if disk_path.is_absolute() {
            disk_path
        } else {
            Utf8PathBuf::from_path_buf(std::env::current_dir()?.join(disk_path.as_std_path()))
                .map_err(|path| eyre!("source path is not UTF-8: {}", path.display()))?
        };
        let line = line.max(1);
        tracing::debug!(
            source_file,
            disk_path = %disk_path,
            line,
            "open_source_in_editor: resolved disk path"
        );

        let associated_app = associated_app_for_file(&disk_path).await;
        tracing::debug!(
            source_file,
            disk_path = %disk_path,
            line,
            associated_app = ?associated_app,
            "open_source_in_editor: resolved associated app"
        );

        if let Some(command) = line_aware_editor_command(associated_app.as_ref(), &disk_path, line)
        {
            tracing::info!(
                editor = command.editor,
                program = %command.program,
                args = ?command.args,
                source_file,
                disk_path = %disk_path,
                line,
                "opening source in associated editor"
            );

            spawn_source_opener(command.program, command.args, "line-aware editor")?;
        } else {
            tracing::debug!(
                source_file,
                disk_path = %disk_path,
                line,
                associated_app = ?associated_app,
                "open_source_in_editor: associated app is not a known line-aware editor"
            );
            open_plain_source(&disk_path, associated_app.as_ref()).await?;
        }

        Ok(())
    }

    pub async fn open_source_id_in_editor(&self, route_path: &str, sid: &str) -> Result<()> {
        tracing::debug!(
            route = %route_path,
            sid,
            "open_source_id_in_editor: resolving source ID"
        );

        let snapshot = DatabaseSnapshot::from_database(&self.db).await;
        let site_tree = build_tree(&snapshot)
            .await?
            .map_err(|errors| eyre!("source parse errors while resolving source ID: {errors:?}"))?;
        let route = Route::new(normalize_route(route_path));

        let source_map = if let Some(section) = site_tree.sections.get(&route) {
            &section.source_map
        } else if let Some(page) = site_tree.pages.get(&route) {
            &page.source_map
        } else {
            bail!("route is not available for source lookup: {route}");
        };

        let entry = source_map
            .get_by_sid(sid)
            .ok_or_else(|| eyre!("source ID {sid:?} is not available for route {route}"))?;
        let source_file = source_map
            .source_path
            .as_deref()
            .ok_or_else(|| eyre!("source map for route {route} has no source path"))?
            .to_string();
        let line = entry.line_start.max(1);

        tracing::debug!(
            route = %route,
            sid,
            source_file = %source_file,
            line,
            kind = ?entry.kind,
            byte_start = entry.byte_start,
            byte_end = entry.byte_end,
            "open_source_id_in_editor: source ID resolved"
        );

        self.open_source_in_editor(&source_file, line).await
    }

    pub async fn open_dead_link_in_editor(
        &self,
        route_path: &str,
        target: DeadLinkTarget,
    ) -> Result<()> {
        tracing::debug!(
            route = %route_path,
            target = ?target,
            "open_dead_link_in_editor: resolving dead link target"
        );

        let stub = dead_link_stub_for_target(&target)?;
        self.create_dead_link_stub(&stub).await?;
        self.open_source_in_editor(stub.source_file.as_str(), 1)
            .await
    }

    async fn create_dead_link_stub(&self, stub: &DeadLinkStub) -> Result<()> {
        let source_root = self
            .source_root
            .as_ref()
            .ok_or_else(|| eyre!("source root is not available in this server mode"))?;

        if stub.source_file.is_absolute()
            || stub
                .source_file
                .components()
                .any(|component| matches!(component, Utf8Component::ParentDir))
        {
            bail!(
                "refusing to create source path outside the content directory: {}",
                stub.source_file
            );
        }

        let disk_path = source_root.join(&stub.source_file);
        if let Some(parent) = disk_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let content = dead_link_stub_content(&stub.title);
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&disk_path)
            .await
        {
            Ok(mut file) => {
                file.write_all(content.as_bytes()).await?;
                tracing::info!(
                    source_file = %stub.source_file,
                    disk_path = %disk_path,
                    title = %stub.title,
                    "created dead link source stub"
                );
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                tracing::debug!(
                    source_file = %stub.source_file,
                    disk_path = %disk_path,
                    "dead link source stub already exists"
                );
            }
            Err(err) => return Err(err.into()),
        }

        Ok(())
    }

    /// Register a browser connection for receiving devtools events.
    ///
    /// The `conn_id` is the roam connection ID, which uniquely identifies this
    /// browser's virtual connection. It's used as the key for routing events.
    pub fn register_browser(&self, conn_id: u64, client: dodeca_protocol::BrowserServiceClient) {
        let mut registry = self.browsers.lock().unwrap();
        registry.browsers.insert(
            conn_id,
            RegisteredBrowser {
                route: None,
                client,
            },
        );
        tracing::info!(conn_id, "Browser registered");
    }

    /// Set the route a browser is subscribed to.
    pub fn set_browser_route(&self, conn_id: u64, route: String) {
        let normalized = normalize_route(&route);
        let mut registry = self.browsers.lock().unwrap();
        if let Some(browser) = registry.browsers.get_mut(&conn_id) {
            tracing::debug!(conn_id, route = %normalized, "Browser subscribed to route");
            browser.route = Some(normalized);
        } else {
            tracing::warn!(conn_id, route = %normalized, "set_browser_route: browser not found");
        }
    }

    /// Unregister a browser connection.
    pub fn unregister_browser(&self, conn_id: u64) {
        let mut registry = self.browsers.lock().unwrap();
        if registry.browsers.remove(&conn_id).is_some() {
            tracing::info!(conn_id, "Browser unregistered");
        }
    }

    /// Notify all connected browsers of a devtools event.
    ///
    /// For route-specific events (like Patches), only browsers subscribed
    /// to that route will receive the event.
    ///
    /// Failed sends (disconnected browsers) are cleaned up asynchronously.
    pub fn notify_browsers(self: &Arc<Self>, event: dodeca_protocol::DevtoolsEvent) {
        // Collect browsers to notify (under lock)
        let to_notify: Vec<(u64, dodeca_protocol::BrowserServiceClient)> = {
            let registry = self.browsers.lock().unwrap();
            let browser_count = registry.browsers.len();

            if browser_count == 0 {
                tracing::trace!(event = %crate::cell_server::event_summary(&event), "No browsers to notify");
                return;
            }

            tracing::debug!(
                event = %crate::cell_server::event_summary(&event),
                browser_count,
                "Notifying browsers"
            );

            registry
                .browsers
                .iter()
                .filter_map(|(browser_id, browser)| {
                    // For route-specific events, check if this browser is subscribed
                    let should_send = match (&event, &browser.route) {
                        (
                            dodeca_protocol::DevtoolsEvent::Patches { route, .. },
                            Some(browser_route),
                        ) => normalize_route(route) == normalize_route(browser_route),
                        (dodeca_protocol::DevtoolsEvent::Patches { .. }, None) => false,
                        // Errors go to specific routes
                        (dodeca_protocol::DevtoolsEvent::Error(_), _) => true,
                        (dodeca_protocol::DevtoolsEvent::ErrorResolved { .. }, _) => true,
                        // Global events go to everyone
                        _ => true,
                    };

                    if should_send {
                        Some((*browser_id, browser.client.clone()))
                    } else {
                        None
                    }
                })
                .collect()
        };

        // Spawn notification tasks and collect failures
        let server = Arc::clone(self);
        crate::spawn::spawn(async move {
            let mut failed_ids = Vec::new();

            // Send to all browsers concurrently
            let futures: Vec<_> = to_notify
                .into_iter()
                .map(|(browser_id, client)| {
                    let event_clone = event.clone();
                    async move {
                        let result = client.on_event(event_clone).await;
                        (browser_id, result)
                    }
                })
                .collect();

            let results = futures_util::future::join_all(futures).await;

            for (browser_id, result) in results {
                if let Err(e) = result {
                    tracing::debug!(browser_id, error = ?e, "Failed to send event to browser (disconnected?)");
                    failed_ids.push(browser_id);
                }
            }

            // Clean up disconnected browsers
            for browser_id in failed_ids {
                server.unregister_browser(browser_id);
            }
        });
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
        let start = std::time::Instant::now();
        let mut warned = false;
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

            // Warn if waiting too long
            if !warned && start.elapsed() > std::time::Duration::from_secs(5) {
                tracing::warn!(
                    generation = state.generation,
                    reason = state.reason.as_deref().unwrap_or(""),
                    elapsed_secs = start.elapsed().as_secs(),
                    "wait_revision_ready: still waiting after 5s - possible deadlock"
                );
                warned = true;
            }

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

    /// All loaded source registry keys (mount-prefixed paths) — for the status
    /// page to count pages per source.
    pub fn source_keys(&self) -> Vec<String> {
        let db = &*self.db;
        SourceRegistry::sources(db)
            .ok()
            .flatten()
            .unwrap_or_default()
            .iter()
            .filter_map(|s| s.path(db).ok().map(|p| p.as_str().to_string()))
            .collect()
    }

    /// Routes currently rendering with an error — for the status page.
    pub fn error_routes(&self) -> Vec<String> {
        let mut routes: Vec<String> = self
            .current_errors
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        routes.sort();
        routes
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

    /// Notify all connected browsers to reload
    /// Computes patches for all cached routes and sends them
    pub async fn trigger_reload(self: &Arc<Self>) {
        // Check for CSS changes first
        let old_css_path = {
            let cache = self.css_cache.read().unwrap();
            cache.clone()
        };
        // Wrap in TASK_DB scope - css_output can trigger rendering via font subsetting
        let new_css_path = crate::db::TASK_DB
            .scope(self.db.clone(), self.get_current_css_path())
            .await;
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
                // Also notify via RPC
                self.notify_browsers(dodeca_protocol::DevtoolsEvent::CssChanged {
                    path: path.clone(),
                });
            }
        }

        // Get all cached routes from LiveReloadServer
        let cached_routes: Vec<String> = { self.live_reload.lock().unwrap().cached_routes() };

        if cached_routes.is_empty() {
            tracing::debug!("No cached routes, nothing to patch");
            return;
        }

        tracing::debug!(
            "trigger_reload: checking {} cached routes",
            cached_routes.len()
        );

        for route in cached_routes {
            // Get new HTML + head_injections (re-render)
            tracing::debug!("trigger_reload: re-rendering {}", route);
            let new_content = self.find_content(&route).await;
            let (new_html, new_head_injections) = match new_content {
                Some(ServeContent::Html {
                    html,
                    head_injections,
                }) => (Some(html), head_injections),
                _ => (None, Vec::new()),
            };

            // Handle case where route was deleted
            if new_html.is_none() {
                tracing::info!("{} - route deleted, sending full reload", route);
                self.live_reload.lock().unwrap().remove_route(&route);
                let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
                self.notify_browsers(dodeca_protocol::DevtoolsEvent::Reload);
                continue;
            }

            let new_html = new_html.unwrap();

            // If the new HTML is an error page, don't patch it in
            if new_html.contains(crate::render::RENDER_ERROR_MARKER) {
                tracing::info!("🔴 {} - template error detected in trigger_reload", route);
                continue;
            }

            // Use LiveReloadServer to diff, handling both HTML patches and head injection changes
            let head_injections_joined = new_head_injections.join("");
            let event = self.live_reload.lock().unwrap().diff_route_with_head(
                &route,
                &new_html,
                &head_injections_joined,
            );

            match event {
                Some(hotmeal_server::LiveReloadEvent::Patches {
                    route: patch_route,
                    patches_blob,
                }) => {
                    let patch_bytes = patches_blob.len();
                    tracing::debug!("{} - patching: {} bytes", patch_route, patch_bytes);
                    let _ = self.livereload_tx.send(LiveReloadMsg::Patches {
                        route: patch_route.clone(),
                        patches: patches_blob.clone(),
                    });
                    self.notify_browsers(dodeca_protocol::DevtoolsEvent::Patches {
                        route: patch_route,
                        patches: patches_blob,
                    });
                }
                Some(hotmeal_server::LiveReloadEvent::HeadChanged { .. }) => {
                    tracing::debug!("{} - head injections changed, sending full reload", route);
                    let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
                    self.notify_browsers(dodeca_protocol::DevtoolsEvent::Reload);
                }
                Some(hotmeal_server::LiveReloadEvent::Reload) => {
                    tracing::debug!("{} - full reload requested", route);
                    let _ = self.livereload_tx.send(LiveReloadMsg::Reload);
                    self.notify_browsers(dodeca_protocol::DevtoolsEvent::Reload);
                }
                None => {
                    // No changes for this route
                }
            }
        }
    }

    /// Cache HTML for a route (called when serving pages)
    fn cache_html(&self, route: &str, html: &str) {
        self.live_reload.lock().unwrap().cache_html(route, html);
    }

    /// Cache head injections for a route (called when serving pages)
    fn cache_head_injections(&self, route: &str, head_injections: &[String]) {
        let joined = head_injections.join("");
        self.live_reload
            .lock()
            .unwrap()
            .cache_head_injections(route, &joined);
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
    pub async fn load_cache(&self, cache_path: &std::path::Path) -> Result<()> {
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

        match self.db.load_from_cache(cache_path).await {
            Ok(true) => {
                tracing::info!("Loaded picante cache from {:?}", cache_path);
            }
            Ok(false) => {
                tracing::debug!("No cache file found");
            }
            Err(e) => {
                tracing::warn!("Failed to load cache: {:?}", e);
            }
        }
        MarkdownRenderSettings::set(&*self.db, self.render_options.source_maps)?;
        Ok(())
    }

    /// Save cached query results to disk
    pub async fn save_cache(&self, cache_path: &std::path::Path) -> Result<()> {
        // Write version file
        let version_path = cache_path.with_extension("version");
        if let Err(e) = std::fs::write(&version_path, PICANTE_CACHE_VERSION.to_string()) {
            tracing::warn!("Failed to write cache version file: {}", e);
        }

        match self.db.save_to_cache(cache_path).await {
            Ok(()) => {
                tracing::info!("Saved picante cache to {:?}", cache_path);
            }
            Err(e) => {
                tracing::warn!("Failed to save cache: {:?}", e);
            }
        }
        Ok(())
    }

    /// Find content for a given path using lazy picante queries
    async fn find_content(self: &Arc<Self>, path: &str) -> Option<ServeContent> {
        tracing::debug!(path, "find_content: called");
        let db = self.db.clone();
        let snapshot = DatabaseSnapshot::from_database(&self.db).await;
        tracing::debug!(path, "find_content: got database snapshot");

        // Wrap all content finding in TASK_DB scope - rendering can be triggered by
        // font subsetting (static_file_output -> font_char_analysis -> all_rendered_html)
        crate::db::TASK_DB
            .scope(db, self.find_content_inner(path, snapshot))
            .await
    }

    /// Render the page owned by `source_key` with `buffer` overlaid on that one
    /// file — the editor's live preview. Uses a db **snapshot** with the one
    /// input overridden in isolation, then the real `serve_html` pipeline, so
    /// the preview is byte-identical to publish and never touches live state or
    /// what other viewers see. Returns `None` if the key has no route.
    pub async fn preview_overlay(
        self: &Arc<Self>,
        source_key: &str,
        buffer: &str,
    ) -> eyre::Result<Option<(String, Vec<dodeca_protocol::SidLine>)>> {
        let db = self.db.clone();
        let snapshot = DatabaseSnapshot::from_database(&self.db).await;

        // Override just this one source file in the snapshot's isolated copy.
        let mut sources = SourceRegistry::sources(&snapshot)
            .ok()
            .flatten()
            .unwrap_or_default();
        let overlaid = SourceFile::new(
            &snapshot,
            crate::types::SourcePath::new(source_key.to_string()),
            crate::types::SourceContent::new(buffer.to_string()),
            0,
        )
        .map_err(|e| eyre::eyre!("overlay source file: {e:?}"))?;
        let pos = sources.iter().position(|s| {
            s.path(&snapshot)
                .ok()
                .map(|p| p.as_str() == source_key)
                .unwrap_or(false)
        });
        match pos {
            Some(pos) => sources[pos] = overlaid,
            None => sources.push(overlaid),
        }
        SourceRegistry::set(&snapshot, sources)
            .map_err(|e| eyre::eyre!("set overlaid sources: {e:?}"))?;

        let source_key = source_key.to_string();
        crate::db::TASK_DB
            .scope(db, async move {
                let routes = crate::queries::source_to_route_map(&snapshot)
                    .await
                    .unwrap_or_default();
                let Some(route) = routes.get(&source_key).cloned() else {
                    return Ok(None);
                };
                let route = Route::new(route);
                let html = match serve_html(&snapshot, route.clone()).await {
                    Ok(Ok(Some(served))) => served.html,
                    _ => return Ok(None),
                };
                // The page's source map (data-sid → source line) for scroll sync.
                // Built on the same snapshot, so the sids match the rendered HTML.
                let source_map = match build_tree(&snapshot).await {
                    Ok(Ok(tree)) => tree
                        .pages
                        .get(&route)
                        .map(|p| &p.source_map)
                        .or_else(|| tree.sections.get(&route).map(|s| &s.source_map))
                        .map(|sm| {
                            sm.entries
                                .iter()
                                .map(|e| dodeca_protocol::SidLine {
                                    sid: e.id.clone(),
                                    line: e.line_start.max(1),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                Ok(Some((html, source_map)))
            })
            .await
    }

    /// Mint an editor session token, but only if `identity` is a verified
    /// editor per the site's `auth` config. `None` (fail closed) when there's
    /// no `auth`, no identity, or the identity isn't on an allowlist.
    pub fn mint_edit_token(&self, identity: Option<&cell_http_proto::Identity>) -> Option<String> {
        // Local dev bypass: act as the configured dev editor regardless of
        // forwarded identity or auth config.
        if let Some(dev) = self.dev_editor.read().unwrap().clone() {
            return Some(self.edit_sessions.mint(dev));
        }
        let cfg = crate::config::global_config()?;
        let auth = cfg.auth.as_ref()?;
        if !crate::authz::is_editor(identity, auth) {
            return None;
        }
        // is_editor is false for a None identity, so unwrap is safe here.
        Some(self.edit_sessions.mint(identity?.clone()))
    }

    /// Resolve an editor token to its identity, re-checking edit rights against
    /// the *current* config so revoking access takes effect mid-session.
    fn resolve_editor(&self, token: &str) -> Option<cell_http_proto::Identity> {
        let identity = self.edit_sessions.resolve(token)?;
        // Local dev bypass: any live token is a valid editor session.
        if self.dev_editor.read().unwrap().is_some() {
            return Some(identity);
        }
        let cfg = crate::config::global_config()?;
        let auth = cfg.auth.as_ref()?;
        crate::authz::is_editor(Some(&identity), auth).then_some(identity)
    }

    /// The editable source key that renders `route` (inverse of source→route).
    async fn source_key_for_route(self: &Arc<Self>, route: &str) -> Option<String> {
        let db = self.db.clone();
        let snapshot = DatabaseSnapshot::from_database(&self.db).await;
        let want = normalize_route(route);
        crate::db::TASK_DB
            .scope(db, async move {
                let map = crate::queries::source_to_route_map(&snapshot)
                    .await
                    .unwrap_or_default();
                map.into_iter()
                    .find(|(_key, r)| normalize_route(r) == want)
                    .map(|(key, _)| key)
            })
            .await
    }

    /// Raw markdown currently held for `source_key` in the live db.
    fn source_content(&self, source_key: &str) -> Option<String> {
        self.get_sources().into_iter().find_map(|sf| {
            let path = sf.path(&*self.db).ok()?;
            if path.as_str() != source_key {
                return None;
            }
            Some(sf.content(&*self.db).ok()?.as_str().to_string())
        })
    }

    /// Editor: load the raw markdown of the page at `route`.
    pub async fn edit_load(
        self: &Arc<Self>,
        token: &str,
        route: &str,
    ) -> dodeca_protocol::EditLoad {
        use dodeca_protocol::EditLoad;
        if self.resolve_editor(token).is_none() {
            return EditLoad::Denied;
        }
        let Some(source_key) = self.source_key_for_route(route).await else {
            return EditLoad::NotFound;
        };
        // file:// URI of the on-disk source, so the editor model URI matches the
        // path the LSP keys documents by.
        let sources = self.status_sources.read().unwrap().clone();
        let uri = crate::build_context::source_for_key(&sources, &source_key)
            .map(|(source, rel)| format!("file://{}", source.content_dir.join(&rel)))
            .unwrap_or_default();
        match self.source_content(&source_key) {
            Some(content) => {
                let base = self.edit_base_oid(&source_key).await;
                EditLoad::Ok {
                    source_key: source_key.clone(),
                    route: normalize_route(route),
                    uri,
                    content,
                    base,
                }
            }
            None => EditLoad::NotFound,
        }
    }

    /// The editable source key whose on-disk path is `abs`, if any (inverse of
    /// `source_for_key`'s path-building — for the file-system provider).
    fn source_key_for_path(&self, abs: &str) -> Option<String> {
        let abs = Utf8Path::new(abs);
        let sources = self.status_sources.read().unwrap();
        for source in sources.iter() {
            if let Ok(rel) = abs.strip_prefix(&source.content_dir) {
                return Some(crate::build_context::mounted_key(
                    &source.mount,
                    rel.as_str(),
                ));
            }
        }
        None
    }

    /// Resolve a `source_key` to its git repo root, absolute on-disk file path,
    /// and content-dir-relative path (the inputs every git edit operation needs).
    fn repo_and_file(
        &self,
        source_key: &str,
    ) -> Option<(camino::Utf8PathBuf, camino::Utf8PathBuf, String)> {
        let sources = self.status_sources.read().unwrap().clone();
        let (source, rel) = crate::build_context::source_for_key(&sources, source_key)?;
        // Git repo root: the checkout for git-backed sources, else the content
        // dir itself (a local source that happens to be in a repo).
        let repo = source
            .checkout_dir
            .clone()
            .unwrap_or_else(|| source.content_dir.clone());
        let file = source.content_dir.join(&rel);
        Some((repo, file, rel))
    }

    /// The on-disk blob oid for `source_key`, the conflict-detection base handed
    /// to the editor at load time (empty when the file doesn't exist yet).
    async fn edit_base_oid(&self, source_key: &str) -> String {
        match self.repo_and_file(source_key) {
            Some((repo, file, _)) => git_blob_oid(&repo, &file).await.unwrap_or_default(),
            None => String::new(),
        }
    }

    /// Editor: read a source file by `file://` URI (file-system provider).
    pub async fn edit_read(self: &Arc<Self>, token: &str, uri: &str) -> dodeca_protocol::EditRead {
        use dodeca_protocol::EditRead;
        if self.resolve_editor(token).is_none() {
            return EditRead::Denied;
        }
        let path = uri.strip_prefix("file://").unwrap_or(uri);
        let Some(source_key) = self.source_key_for_path(path) else {
            return EditRead::NotFound;
        };
        match self.source_content(&source_key) {
            Some(content) => {
                let base = self.edit_base_oid(&source_key).await;
                EditRead::Ok { content, base }
            }
            None => EditRead::NotFound,
        }
    }

    /// Editor: list every editable page (file tree).
    pub async fn edit_list(self: &Arc<Self>, token: &str) -> dodeca_protocol::EditList {
        use dodeca_protocol::{EditEntry, EditList};
        if self.resolve_editor(token).is_none() {
            return EditList::Denied;
        }
        let db = self.db.clone();
        let snapshot = DatabaseSnapshot::from_database(&self.db).await;
        let map = crate::db::TASK_DB
            .scope(db, async move {
                crate::queries::source_to_route_map(&snapshot)
                    .await
                    .unwrap_or_default()
            })
            .await;
        let sources = self.status_sources.read().unwrap().clone();
        let mut entries: Vec<EditEntry> = map
            .into_iter()
            .filter_map(|(source_key, route)| {
                let (source, rel) = crate::build_context::source_for_key(&sources, &source_key)?;
                let route = normalize_route(&route);
                let title = match route.trim_start_matches('/') {
                    "" => "/".to_string(),
                    rest => rest.to_string(),
                };
                Some(EditEntry {
                    uri: format!("file://{}", source.content_dir.join(&rel)),
                    source_key,
                    route,
                    title,
                })
            })
            .collect();
        entries.sort_by(|a, b| a.route.cmp(&b.route));
        EditList::Ok { entries }
    }

    /// Build an authoring project from a **snapshot** of the live db with the
    /// given source overlays applied (open documents). Reuses the host's already
    /// computed + memoized renders — only the overlaid sources' dependents
    /// recompute. Powers the in-process LSP without re-loading from disk.
    pub async fn authoring_project_overlay(
        &self,
        overlays: Vec<(String, String)>,
    ) -> Result<crate::authoring_model::AuthoringProject> {
        use crate::authoring_model::{ProjectBuildInputs, build_authoring_project_on_db};
        use crate::db::{
            DataRegistry, SourceFile, SourceRegistry, StaticRegistry, TemplateRegistry,
        };

        let content_dir = self
            .status_sources
            .read()
            .unwrap()
            .first()
            .map(|s| s.content_dir.clone())
            .ok_or_else(|| eyre!("no workspace content dir"))?;

        let db = self.db.clone();
        let snapshot = DatabaseSnapshot::from_database(&self.db).await;

        // Override open documents in the snapshot's isolated copy.
        let mut sources = SourceRegistry::sources(&snapshot)
            .ok()
            .flatten()
            .unwrap_or_default();
        for (path, content) in &overlays {
            let Some(key) = self.source_key_for_path(path) else {
                continue;
            };
            let file = SourceFile::new(
                &snapshot,
                crate::types::SourcePath::new(key.clone()),
                crate::types::SourceContent::new(content.clone()),
                0,
            )
            .map_err(|e| eyre!("overlay source: {e:?}"))?;
            match sources.iter().position(|s| {
                s.path(&snapshot)
                    .ok()
                    .map(|p| p.as_str() == key)
                    .unwrap_or(false)
            }) {
                Some(i) => sources[i] = file,
                None => sources.push(file),
            }
        }
        SourceRegistry::set(&snapshot, sources).map_err(|e| eyre!("set overlays: {e:?}"))?;

        crate::db::TASK_DB
            .scope(db, async move {
                let sources = SourceRegistry::sources(&snapshot)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|f| Some(((*f.path(&snapshot).ok()?).clone(), f)))
                    .collect();
                let templates = TemplateRegistry::templates(&snapshot)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|f| Some(((*f.path(&snapshot).ok()?).clone(), f)))
                    .collect();
                let static_files = StaticRegistry::files(&snapshot)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|f| Some(((*f.path(&snapshot).ok()?).clone(), f)))
                    .collect();
                let data_files = DataRegistry::files(&snapshot)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|f| Some(((*f.path(&snapshot).ok()?).clone(), f)))
                    .collect();
                build_authoring_project_on_db(ProjectBuildInputs {
                    db: &snapshot,
                    content_dir: &content_dir,
                    sources: &sources,
                    templates: &templates,
                    static_files: &static_files,
                    data_files: &data_files,
                })
                .await
            })
            .await
    }

    /// Editor: live preview of `buffer` overlaid on `source_key`, isolated.
    pub async fn edit_preview(
        self: &Arc<Self>,
        token: &str,
        source_key: &str,
        buffer: &str,
    ) -> dodeca_protocol::EditPreview {
        use dodeca_protocol::EditPreview;
        if self.resolve_editor(token).is_none() {
            return EditPreview::Denied;
        }
        match self.preview_overlay(source_key, buffer).await {
            Ok(Some((html, source_map))) => EditPreview::Ok { html, source_map },
            Ok(None) => EditPreview::NotFound,
            Err(error) => {
                tracing::warn!(source_key, %error, "edit_preview render failed");
                EditPreview::NotFound
            }
        }
    }

    /// Editor: commit `buffer` to `source_key` authored as the editing user.
    pub async fn edit_save(
        self: &Arc<Self>,
        token: &str,
        source_key: &str,
        buffer: &str,
        base: &str,
        message: &str,
    ) -> dodeca_protocol::EditSave {
        use dodeca_protocol::EditSave;
        let Some(identity) = self.resolve_editor(token) else {
            return EditSave::Denied;
        };
        let Some((repo, file, rel)) = self.repo_and_file(source_key) else {
            return EditSave::NotFound;
        };
        // Optimistic concurrency: refuse to clobber a file that changed on disk
        // since the editor loaded it. `base` empty means "didn't exist at load";
        // only treat a present-now file as a conflict in that case.
        let current = git_blob_oid(&repo, &file).await.unwrap_or_default();
        let conflict = if base.is_empty() {
            !current.is_empty()
        } else {
            base != current
        };
        if conflict {
            tracing::info!(source_key, base, current, "edit_save conflict");
            return EditSave::Conflict { current };
        }
        match commit_as_user(&repo, &file, buffer, &identity, &rel, message).await {
            Ok(commit) => {
                let base = git_blob_oid(&repo, &file).await.unwrap_or_default();
                EditSave::Ok { commit, base }
            }
            Err(error) => {
                tracing::warn!(source_key, %error, "edit_save failed");
                EditSave::Error {
                    message: error.to_string(),
                }
            }
        }
    }

    /// Editor: run an authoring LSP session **in process**, bridged to the vox
    /// channel. The browser sends one JSON-RPC message per `incoming` chunk; we
    /// add `Content-Length` framing onto an in-memory duplex that the installed
    /// [`LspRunner`] serves the LSP on, and strip framing from its replies back
    /// into one `outgoing` chunk per message. The runner is injected by the
    /// binary (which depends on both `dodeca` and the LSP crate), so there's no
    /// subprocess and no dependency cycle.
    pub async fn run_lsp_session(
        &self,
        token: &str,
        mut incoming: vox::Rx<String>,
        outgoing: vox::Tx<String>,
    ) {
        use tokio::io::{AsyncWriteExt, BufReader};

        if self.resolve_editor(token).is_none() {
            tracing::warn!("lsp: rejecting session with invalid/expired editor token");
            outgoing.close(Default::default()).await.ok();
            return;
        }

        let Some(runner) = self.lsp_runner.read().unwrap().clone() else {
            tracing::warn!("lsp: no LspRunner installed (binary did not wire it up)");
            outgoing.close(Default::default()).await.ok();
            return;
        };

        // In-memory duplex: the runner serves the LSP on `lsp_side`; we bridge the
        // vox channel onto `host_side` with LSP `Content-Length` framing.
        let (host_side, lsp_side) = tokio::io::duplex(64 * 1024);
        let (host_read, mut host_write) = tokio::io::split(host_side);
        tracing::info!("lsp: session started (in-process)");

        // Browser → LSP: frame each message with Content-Length.
        let to_lsp = async move {
            while let Ok(Some(message)) = incoming.recv().await {
                let text = message.get();
                let header = format!("Content-Length: {}\r\n\r\n", text.len());
                if host_write.write_all(header.as_bytes()).await.is_err()
                    || host_write.write_all(text.as_bytes()).await.is_err()
                    || host_write.flush().await.is_err()
                {
                    break;
                }
            }
            host_write.shutdown().await.ok();
        };

        // LSP → browser: strip framing, one message per chunk.
        let to_browser = async move {
            let mut reader = BufReader::new(host_read);
            loop {
                match read_lsp_message(&mut reader).await {
                    Ok(Some(body)) => {
                        let text = String::from_utf8_lossy(&body).into_owned();
                        if outgoing.send(text).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        tracing::debug!(%error, "lsp: failed reading LSP frame");
                        break;
                    }
                }
            }
            outgoing.close(Default::default()).await.ok();
        };

        tokio::join!(to_lsp, to_browser, runner.serve(lsp_side));
        tracing::info!("lsp: session ended");
    }

    /// Inner implementation of find_content, runs within TASK_DB scope
    async fn find_content_inner(
        self: &Arc<Self>,
        path: &str,
        snapshot: DatabaseSnapshot,
    ) -> Option<ServeContent> {
        // Get known routes for dead link detection (only in dev mode)
        let known_routes: Option<HashSet<String>> = if self.render_options.livereload {
            match build_tree(&snapshot).await {
                Ok(Ok(site_tree)) => {
                    let routes: HashSet<String> = site_tree
                        .sections
                        .keys()
                        .chain(site_tree.pages.keys())
                        .map(|r| r.as_str().to_string())
                        .collect();
                    Some(routes)
                }
                Ok(Err(errors)) => {
                    tracing::debug!(
                        error_count = errors.len(),
                        "known route collection skipped because source parsing failed"
                    );
                    None
                }
                Err(err) => {
                    tracing::debug!(
                        error = %err,
                        "known route collection skipped because build_tree failed"
                    );
                    None
                }
            }
        } else {
            None
        };

        // 1. Try to serve rendered markdown for page.md requests.
        if let Some(markdown_route) = markdown_route_from_path(path) {
            let tree_result = build_tree(&snapshot).await;
            let route_exists = match tree_result {
                Ok(Ok(tree)) => {
                    tree.pages.contains_key(&markdown_route)
                        || tree.sections.contains_key(&markdown_route)
                }
                Ok(Err(errors)) => {
                    tracing::debug!(
                        error_count = errors.len(),
                        "markdown route skipped because source parsing failed"
                    );
                    false
                }
                Err(err) => {
                    tracing::debug!(
                        error = %err,
                        "markdown route skipped because build_tree failed"
                    );
                    false
                }
            };

            if route_exists {
                match render_page_markdown(&snapshot, markdown_route).await {
                    Ok(Ok(markdown)) => {
                        return Some(ServeContent::StaticNoCache(
                            markdown.0.into_bytes(),
                            "text/markdown; charset=utf-8",
                        ));
                    }
                    Ok(Err(error)) => {
                        tracing::debug!(error = %error, "render_page_markdown returned SiteError");
                    }
                    Err(error) => {
                        tracing::error!(error = ?error, "render_page_markdown returned PicanteError");
                        return None;
                    }
                }
            }
        }

        // 2. Try to serve as HTML page (by route)
        let route_path = if path == "/" {
            "/".to_string()
        } else {
            path.trim_end_matches('/').to_string()
        };

        let route = Route::new(route_path.clone());
        tracing::debug!(route = %route.as_str(), "find_content: calling serve_html");
        let serve_html_result = serve_html(&snapshot, route).await;
        tracing::debug!(route = %route_path, has_result = serve_html_result.is_ok(), "find_content: serve_html returned");

        // Process the result, tracking if we got an error for devtools notification
        let (html, head_injections, maybe_render_error, is_error_page) = match serve_html_result {
            Ok(Ok(Some(served))) => (Some(served.html), served.head_injections, None, false),
            Ok(Ok(None)) => (None, Vec::new(), None, false),
            Ok(Err(site_error)) => {
                use crate::queries::SiteError;
                match site_error {
                    SiteError::Parse(build_error) => {
                        // Format parse errors using the standard error page
                        let error_text = build_error
                            .errors
                            .iter()
                            .map(|e| format!("{}: {}", e.path, e.error))
                            .collect::<Vec<_>>()
                            .join("\n");
                        (
                            Some(crate::error_pages::render_generic_error_page(
                                &format!("Failed to parse {} file(s)", build_error.errors.len()),
                                &error_text,
                            )),
                            Vec::new(),
                            None, // Parse errors don't have structured ErrorInfo yet
                            true,
                        )
                    }
                    SiteError::Render(render_error) => {
                        // Build ErrorInfo from the structured error
                        let loc = render_error.error.location.as_ref();
                        let (line, column) = loc
                            .map(|l| {
                                let (line, col) =
                                    crate::error_pages::offset_to_line_col(&l.source, l.offset);
                                (Some(line as u32), Some(col as u32))
                            })
                            .unwrap_or((None, None));

                        // Build source snippet from location
                        let source_snippet = loc.and_then(|l| {
                            let error_line = line? as usize;
                            let lines: Vec<&str> = l.source.lines().collect();
                            let start = error_line.saturating_sub(3).max(1);
                            let end = (error_line + 2).min(lines.len());

                            let snippet_lines: Vec<dodeca_protocol::SourceLine> = lines
                                .iter()
                                .enumerate()
                                .skip(start - 1)
                                .take(end - start + 1)
                                .map(|(i, content)| dodeca_protocol::SourceLine {
                                    number: (i + 1) as u32,
                                    content: content.to_string(),
                                })
                                .collect();

                            Some(dodeca_protocol::SourceSnippet {
                                lines: snippet_lines,
                                error_line: error_line as u32,
                            })
                        });

                        let error_info = dodeca_protocol::ErrorInfo {
                            route: path.to_string(),
                            message: render_error.error.message.clone(),
                            template: loc.map(|l| l.filename.clone()),
                            line,
                            column,
                            source_snippet,
                            snapshot_id: format!(
                                "error-{}",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis()
                            ),
                            available_variables: vec![],
                        };

                        // Format as HTML error page for the browser
                        let html =
                            crate::error_pages::render_structured_error_page(&render_error.error);
                        (Some(html), Vec::new(), Some(error_info), true)
                    }
                    SiteError::WikiLinks(wiki_error) => (
                        Some(crate::error_pages::render_generic_error_page(
                            "Failed to resolve wiki links",
                            &wiki_error.to_string(),
                        )),
                        Vec::new(),
                        None,
                        true,
                    ),
                }
            }
            Err(e) => {
                tracing::error!(error = ?e, "serve_html returned PicanteError");
                return None;
            }
        };

        if let Some(html) = html {
            // Handle error notification to devtools
            if let Some(error_info) = maybe_render_error {
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
                // Also notify via RPC
                self.notify_browsers(dodeca_protocol::DevtoolsEvent::Error(error_info));
            } else if !is_error_page {
                // Page rendered successfully - clear any previous error
                {
                    let mut errors = self.current_errors.write().unwrap();
                    errors.remove(path);
                }
                let _ = self.livereload_tx.send(LiveReloadMsg::ErrorResolved {
                    route: path.to_string(),
                });
                // Also notify via RPC
                self.notify_browsers(dodeca_protocol::DevtoolsEvent::ErrorResolved {
                    route: path.to_string(),
                });
            }

            let code_results: Vec<_> = self.code_execution_results.read().unwrap().clone();
            // Skip dead link checking for error pages - no point checking our own error HTML
            let routes_for_dead_links = if is_error_page {
                None
            } else {
                known_routes.as_ref()
            };
            let html = inject_livereload_with_build_info(
                &html,
                self.render_options,
                routes_for_dead_links,
                &code_results,
                &head_injections,
            )
            .await;
            return Some(ServeContent::Html {
                html,
                head_injections,
            });
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

        // 4. Search assets, served under `/search/`.
        if let Some(rel) = path.strip_prefix('/')
            && rel.starts_with("search/")
        {
            // Runtime assets (wasm core, loader, UI, CSS) live under a
            // content-versioned directory — safe to cache immutably.
            if let Some(bytes) = crate::search::runtime_asset(rel) {
                return Some(ServeContent::Static(
                    bytes.to_vec(),
                    mime_from_extension(path),
                ));
            }
            // Index files (manifest, shards, fragments) live at stable paths
            // and are regenerated every build, so they MUST NOT be cached
            // `immutable` — `StaticNoCache` keeps `ddc serve` fresh after edits.
            if let Ok(files) = crate::search::search_index_files(&snapshot).await {
                for file in files {
                    if let crate::db::OutputFile::Static { path: p, content } = file
                        && p.as_str() == rel
                    {
                        return Some(ServeContent::StaticNoCache(
                            content,
                            mime_from_extension(path),
                        ));
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
        let data_value = crate::data::parse_raw_data_files(&raw_data).await;
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
        use crate::template_host::{RenderContext, RenderContextGuard};
        use facet_value::{VObject, VString};
        use std::sync::Arc;

        let snapshot = DatabaseSnapshot::from_database(&self.db).await;

        let site_tree = build_tree(&snapshot)
            .await
            .map_err(|e| format!("Failed to build tree: {:?}", e))?
            .map_err(|e| format!("Source parse errors: {:?}", e))?;

        // Pre-load all templates for sync access during evaluation
        let templates = crate::queries::load_all_templates(&snapshot)
            .await
            .map_err(|e| format!("Failed to load templates: {:?}", e))?;

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

        // Build context Value for the expression evaluation
        let mut ctx = VObject::new();

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
        ctx.insert(
            VString::from("config"),
            facet_value::Value::from(config_map),
        );

        // Add current_path
        ctx.insert(
            VString::from("current_path"),
            facet_value::Value::from(route.as_str()),
        );

        // Check if it's a section or page and add appropriate data
        if let Some(section) = site_tree.sections.get(&route) {
            let section_value = crate::render::section_to_value(section, &site_tree, &base_url);
            ctx.insert(VString::from("section"), section_value);
            ctx.insert(VString::from("page"), facet_value::Value::NULL);
        } else if let Some(page) = site_tree.pages.get(&route) {
            let page_value = crate::render::page_to_value(page, &site_tree);
            ctx.insert(VString::from("page"), page_value);

            // Add parent section
            if let Some(section) = site_tree.sections.get(&page.section_route) {
                let section_value = crate::render::section_to_value(section, &site_tree, &base_url);
                ctx.insert(VString::from("section"), section_value);
            }
        }

        // Add site tree info
        if let Some(root) = site_tree.sections.get(&Route::root()) {
            let root_value = crate::render::section_to_value(root, &site_tree, &base_url);
            ctx.insert(VString::from("root"), root_value);
        }

        // Create render context for the cell (handles template loading and data resolution)
        let render_context = RenderContext::new(templates, self.db.clone(), Arc::new(site_tree));
        let guard = RenderContextGuard::new(render_context);

        // Convert context to Value
        let context_value: facet_value::Value = ctx.into();

        // Evaluate the expression via cell
        match crate::cells::eval_expression_cell(guard.id(), expression, context_value).await {
            Ok(cell_gingembre_proto::EvalResult::Success { value }) => {
                Ok(value_to_scope_value(&value))
            }
            Ok(cell_gingembre_proto::EvalResult::Error { message }) => {
                // Convert ANSI error to HTML for display in devtools
                Err(crate::error_pages::ansi_to_html(&message))
            }
            Err(e) => Err(format!("Expression evaluation failed: {}", e)),
        }
    }

    /// Find content for RPC serving (returns protocol ServeContent type)
    ///
    /// This wraps find_content and converts the result to the protocol's ServeContent.
    pub async fn find_content_for_rpc(
        self: &Arc<Self>,
        path: &str,
    ) -> cell_http_proto::ServeContent {
        use cell_http_proto::ServeContent as RpcServeContent;

        // Get current generation
        let generation = self.current_generation();

        match self.find_content(path).await {
            Some(ServeContent::Html {
                html,
                head_injections,
            }) => {
                // Cache HTML and head injections for smart reload patching
                self.cache_html(path, &html);
                self.cache_head_injections(path, &head_injections);
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
                // Static asset misses should return a direct 404; route suggestions are for pages.
                let similar = if should_suggest_routes_for_404(path) {
                    self.find_similar_routes(path).await
                } else {
                    Vec::new()
                };
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
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.2));
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
            for rule in &section.reqs {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeadLinkStub {
    source_file: Utf8PathBuf,
    title: String,
}

fn dead_link_stub_for_target(target: &DeadLinkTarget) -> Result<DeadLinkStub> {
    match target {
        DeadLinkTarget::Wiki { key, title } => {
            let slug = path_segment_slug(key)
                .or_else(|| path_segment_slug(title))
                .ok_or_else(|| eyre!("dead wiki link target has no usable slug"))?;
            let title = useful_title(title).unwrap_or_else(|| title_from_slug(&slug));
            Ok(DeadLinkStub {
                source_file: Utf8PathBuf::from(format!("{slug}.md")),
                title,
            })
        }
        DeadLinkTarget::Internal { href, title } => {
            let route = internal_href_route(href)?;
            let source_path = source_path_for_route(&route)?;
            let fallback_slug = route
                .trim_matches('/')
                .rsplit('/')
                .next()
                .filter(|segment| !segment.is_empty())
                .unwrap_or("untitled");
            let title = useful_title(title).unwrap_or_else(|| title_from_slug(fallback_slug));
            Ok(DeadLinkStub {
                source_file: Utf8PathBuf::from(source_path),
                title,
            })
        }
    }
}

fn internal_href_route(href: &str) -> Result<String> {
    if !href.starts_with('/') || href.starts_with("//") || href.starts_with("/__") {
        bail!("dead internal link is not authorable: {href}");
    }

    let path = href
        .split(['#', '?'])
        .next()
        .filter(|path| !path.is_empty())
        .ok_or_else(|| eyre!("dead internal link has no route: {href}"))?;
    let route = normalize_route(path);
    if route == "/" {
        bail!("refusing to create a stub for the content root");
    }

    Ok(route)
}

fn source_path_for_route(route: &str) -> Result<String> {
    let mut parts = Vec::new();
    for segment in route.trim_matches('/').split('/') {
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains('\\') {
            bail!("dead internal link route contains an unsafe segment: {route}");
        }
        parts.push(segment);
    }

    if parts.is_empty() {
        bail!("dead internal link route has no path segments: {route}");
    }

    Ok(format!("{}.md", parts.join("/")))
}

fn dead_link_stub_content(title: &str) -> String {
    format!(
        "+++\ntitle = \"{}\"\n+++\n",
        toml_basic_string_escape(title)
    )
}

fn useful_title(title: &str) -> Option<String> {
    let title = title.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

fn path_segment_slug(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut last_was_dash = true;

    for c in input.chars() {
        if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                slug.push(lower);
            }
            last_was_dash = false;
        } else if (c == '-' || c == '_' || c.is_whitespace()) && !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() { None } else { Some(slug) }
}

fn title_from_slug(slug: &str) -> String {
    let mut title = String::new();
    let mut capitalize_next = true;
    for c in slug.chars() {
        if c == '-' || c == '_' {
            if !title.ends_with(' ') && !title.is_empty() {
                title.push(' ');
            }
            capitalize_next = true;
        } else if capitalize_next {
            for upper in c.to_uppercase() {
                title.push(upper);
            }
            capitalize_next = false;
        } else {
            title.push(c);
        }
    }

    if title.is_empty() {
        "Untitled".to_string()
    } else {
        title
    }
}

fn toml_basic_string_escape(input: &str) -> String {
    let mut escaped = String::new();
    for c in input.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => escaped.push(' '),
            c => escaped.push(c),
        }
    }
    escaped
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssociatedApp {
    path: Utf8PathBuf,
    bundle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorCommand {
    editor: &'static str,
    program: String,
    args: Vec<String>,
}

#[cfg(target_os = "macos")]
async fn associated_app_for_file(path: &Utf8Path) -> Option<AssociatedApp> {
    tracing::debug!(
        path = %path,
        "associated_app_for_file: querying LaunchServices"
    );

    let script = r#"ObjC.import("AppKit");
function run(argv) {
    const url = $.NSURL.fileURLWithPath(argv[0]);
    const app = $.NSWorkspace.sharedWorkspace.URLForApplicationToOpenURL(url);
    if (!app) return "";
    const bundle = $.NSBundle.bundleWithURL(app);
    const bundleId = bundle ? bundle.bundleIdentifier.js : "";
    return app.path.js + "\n" + bundleId;
}"#;

    let output = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", script, path.as_str()])
        .output()
        .await
        .map_err(|err| {
            tracing::debug!(
                path = %path,
                error = %err,
                "associated_app_for_file: failed to spawn osascript"
            );
            err
        })
        .ok()?;

    if !output.status.success() {
        tracing::warn!(
            status = %output.status,
            stderr = %String::from_utf8_lossy(&output.stderr),
            path = %path,
            "failed to query associated application"
        );
        return None;
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| {
            tracing::debug!(
                path = %path,
                error = %err,
                "associated_app_for_file: osascript output was not UTF-8"
            );
            err
        })
        .ok()?;
    let associated_app = associated_app_from_osascript_output(&stdout);
    tracing::debug!(
        path = %path,
        raw_output = %stdout.trim_end(),
        associated_app = ?associated_app,
        "associated_app_for_file: LaunchServices query completed"
    );
    associated_app
}

#[cfg(not(target_os = "macos"))]
async fn associated_app_for_file(_path: &Utf8Path) -> Option<AssociatedApp> {
    None
}

#[cfg(any(target_os = "macos", test))]
fn associated_app_from_osascript_output(output: &str) -> Option<AssociatedApp> {
    let mut lines = output.lines();
    let path = lines.next()?.trim();
    if path.is_empty() {
        return None;
    }
    let bundle_id = lines
        .next()
        .map(str::trim)
        .filter(|bundle_id| !bundle_id.is_empty())
        .map(ToOwned::to_owned);

    Some(AssociatedApp {
        path: Utf8PathBuf::from(path),
        bundle_id,
    })
}

fn line_aware_editor_command(
    app: Option<&AssociatedApp>,
    disk_path: &Utf8Path,
    line: u32,
) -> Option<EditorCommand> {
    let Some(app) = app else {
        tracing::debug!(
            disk_path = %disk_path,
            line,
            "line_aware_editor_command: no associated app"
        );
        return None;
    };
    let bundle_id = app.bundle_id.as_deref().unwrap_or_default();
    let app_name = app
        .path
        .file_stem()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path_line = format!("{disk_path}:{line}");
    tracing::debug!(
        app_path = %app.path,
        bundle_id,
        app_name,
        disk_path = %disk_path,
        line,
        "line_aware_editor_command: checking associated app"
    );

    if bundle_id == "dev.zed.Zed" || app_name == "zed" {
        return Some(EditorCommand {
            editor: "Zed",
            program: app_cli_or_name(&app.path, "Contents/MacOS/cli", "zed"),
            args: vec![path_line],
        });
    }

    if bundle_id == "com.microsoft.VSCode"
        || bundle_id == "com.vscodium"
        || app_name == "visual studio code"
        || app_name == "vscodium"
    {
        return Some(EditorCommand {
            editor: "Visual Studio Code",
            program: app_cli_or_name(&app.path, "Contents/Resources/app/bin/code", "code"),
            args: vec!["--goto".to_string(), path_line],
        });
    }

    if app_name == "cursor" {
        return Some(EditorCommand {
            editor: "Cursor",
            program: app_cli_or_name(&app.path, "Contents/Resources/app/bin/cursor", "cursor"),
            args: vec!["--goto".to_string(), path_line],
        });
    }

    if bundle_id.starts_with("com.sublimetext.") || app_name == "sublime text" {
        return Some(EditorCommand {
            editor: "Sublime Text",
            program: app_cli_or_name(&app.path, "Contents/SharedSupport/bin/subl", "subl"),
            args: vec![path_line],
        });
    }

    if bundle_id == "com.macromates.TextMate" || app_name == "textmate" {
        return Some(EditorCommand {
            editor: "TextMate",
            program: app_cli_or_name(&app.path, "Contents/Resources/mate", "mate"),
            args: vec!["-l".to_string(), line.to_string(), disk_path.to_string()],
        });
    }

    if bundle_id == "com.barebones.bbedit" || app_name == "bbedit" {
        return Some(EditorCommand {
            editor: "BBEdit",
            program: "bbedit".to_string(),
            args: vec![format!("+{line}"), disk_path.to_string()],
        });
    }

    if bundle_id == "org.vim.MacVim" || app_name == "macvim" {
        return Some(EditorCommand {
            editor: "MacVim",
            program: app_cli_or_name(&app.path, "Contents/bin/mvim", "mvim"),
            args: vec![format!("+{line}"), disk_path.to_string()],
        });
    }

    tracing::debug!(
        app_path = %app.path,
        bundle_id,
        app_name,
        disk_path = %disk_path,
        line,
        "line_aware_editor_command: no line-aware mapping for associated app"
    );

    None
}

fn app_cli_or_name(app_path: &Utf8Path, cli_relative_path: &str, fallback_name: &str) -> String {
    let candidate = app_path.join(cli_relative_path);
    if candidate.exists() {
        tracing::debug!(
            app_path = %app_path,
            cli = %candidate,
            fallback_name,
            "app_cli_or_name: using bundled editor CLI"
        );
        candidate.to_string()
    } else {
        tracing::debug!(
            app_path = %app_path,
            cli = %candidate,
            fallback_name,
            "app_cli_or_name: bundled editor CLI missing, using PATH fallback"
        );
        fallback_name.to_string()
    }
}

#[cfg(target_os = "macos")]
async fn open_plain_source(path: &Utf8Path, app: Option<&AssociatedApp>) -> Result<()> {
    let mut args = Vec::new();
    if let Some(bundle_id) = app.and_then(|app| app.bundle_id.as_deref()) {
        args.extend(["-b".to_string(), bundle_id.to_string()]);
    } else if let Some(app) = app {
        args.extend(["-a".to_string(), app.path.to_string()]);
    }
    args.push(path.to_string());

    tracing::info!(
        app = ?app,
        program = "/usr/bin/open",
        args = ?args,
        path = %path,
        "opening source with associated application"
    );

    spawn_source_opener("/usr/bin/open".to_string(), args, "associated application")
}

#[cfg(not(target_os = "macos"))]
async fn open_plain_source(path: &Utf8Path, _app: Option<&AssociatedApp>) -> Result<()> {
    tracing::info!(path = %path, "opening source with platform default application");
    open::that_detached(path.as_str())
        .map_err(|err| eyre!("failed to open source file {path}: {err}"))?;
    Ok(())
}

fn spawn_source_opener(program: String, args: Vec<String>, kind: &'static str) -> Result<()> {
    let mut child = Command::new(&program)
        .args(&args)
        .spawn()
        .map_err(|err| eyre!("failed to spawn source opener {program}: {err}"))?;
    let pid = child.id();

    tracing::debug!(
        program = %program,
        args = ?args,
        pid,
        kind,
        "spawned source opener"
    );

    crate::spawn::spawn(async move {
        match child.wait().await {
            Ok(status) if status.success() => {
                tracing::debug!(
                    program = %program,
                    args = ?args,
                    pid,
                    status = %status,
                    kind,
                    "source opener exited successfully"
                );
            }
            Ok(status) => {
                tracing::warn!(
                    program = %program,
                    args = ?args,
                    pid,
                    status = %status,
                    kind,
                    "source opener exited with non-zero status"
                );
            }
            Err(err) => {
                tracing::warn!(
                    program = %program,
                    args = ?args,
                    pid,
                    error = %err,
                    kind,
                    "failed to wait for source opener"
                );
            }
        }
    });

    Ok(())
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
    Html {
        html: String,
        head_injections: Vec<String>,
    },
    Css(String),
    Static(Vec<u8>, &'static str),
    /// Static file served at original path (no caching, for favicon etc.)
    StaticNoCache(Vec<u8>, &'static str),
}

/// Embedded devtools JavaScript (compiled at build time by wasm-pack)
static DEVTOOLS_JS: &str = include_str!("../../dodeca-devtools/pkg/dodeca_devtools.js");

/// Embedded devtools WebAssembly (compiled at build time by wasm-pack)
static DEVTOOLS_WASM: &[u8] = include_bytes!("../../dodeca-devtools/pkg/dodeca_devtools_bg.wasm");

fn load_devtools_js() -> Option<String> {
    Some(DEVTOOLS_JS.to_string())
}

fn load_devtools_wasm() -> Option<Vec<u8>> {
    Some(DEVTOOLS_WASM.to_vec())
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

// Built browser-editor assets (vite/Monaco bundle), generated at build time.
// Maps each served path (e.g. `edit.js`, `edit.css`, hashed chunks) to bytes.
// Empty when the editor wasn't built (no node/pnpm) — `/_/edit/*` then 404s.
include!(concat!(env!("OUT_DIR"), "/editor_assets.rs"));

/// Serve a built editor asset for a `/_/edit/<path>` request. Bare `/_/edit/`
/// resolves to the entry bundle. Any `?v=` cache-bust query is ignored.
pub fn get_editor_asset(path: &str) -> Option<(Vec<u8>, &'static str)> {
    let rel = path.strip_prefix("/_/edit/")?;
    let rel = rel.split('?').next().unwrap_or(rel);
    let rel = if rel.is_empty() { "edit.js" } else { rel };
    EDITOR_ASSETS
        .iter()
        .find(|(name, _)| *name == rel)
        .map(|(_, bytes)| (bytes.to_vec(), editor_asset_mime(rel)))
}

fn editor_asset_mime(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") => "application/json; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("ttf") => "font/ttf",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

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
        let js = load_devtools_js().expect("devtools JS is embedded at compile time");
        return Some((
            rewrite_devtools_js(&js).into_bytes(),
            "application/javascript",
        ));
    }

    // Check for WASM (cache-busted)
    if asset_path.ends_with(".wasm") {
        let bytes = load_devtools_wasm().expect("devtools WASM is embedded at compile time");
        return Some((bytes, "application/wasm"));
    }

    None
}

/// Rewrite relative snippet imports to absolute paths
fn rewrite_devtools_js(js: &str) -> String {
    // The generated JS has imports like:
    //   import { X } from './snippets/foo/bar.js';
    // We need to rewrite them to absolute paths:
    //   import { X } from '/_/snippets/foo/bar.js';
    js.replace("from './snippets/", "from '/_/snippets/")
}

fn markdown_route_from_path(path: &str) -> Option<Route> {
    let route = path.strip_suffix(".md")?;
    if route == "/index" || route.is_empty() {
        return Some(Route::root());
    }
    Some(Route::new(route.to_string()))
}

fn should_suggest_routes_for_404(path: &str) -> bool {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    !file_name.contains('.')
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

/// Write `buffer` to `file`, then commit it to `repo` authored and committed as
/// the editing user, and push. Push relies on the repo's configured remote
/// credentials (the service account's token) — the user is only the git
/// author/committer, for attribution. Returns the new commit hash.
async fn commit_as_user(
    repo: &Utf8Path,
    file: &Utf8Path,
    buffer: &str,
    identity: &cell_http_proto::Identity,
    rel: &str,
    message: &str,
) -> Result<String> {
    if let Some(parent) = file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(file, buffer).await?;

    git(repo, &["add", file.as_str()]).await?;

    // Nothing staged → the buffer already matched what's on disk.
    if git_status(repo, &["diff", "--cached", "--quiet"])
        .await?
        .success()
    {
        bail!("no changes to save");
    }

    let name = if identity.name.trim().is_empty() {
        identity.user.as_str()
    } else {
        identity.name.as_str()
    };
    let name_cfg = format!("user.name={name}");
    let email_cfg = format!("user.email={}", identity.email);
    let message = if message.trim().is_empty() {
        format!("docs: edit {rel} via web editor")
    } else {
        message.to_string()
    };
    git(
        repo,
        &["-c", &name_cfg, "-c", &email_cfg, "commit", "-m", &message],
    )
    .await?;

    let hash = git(repo, &["rev-parse", "HEAD"]).await?;
    git(repo, &["push"]).await?;
    Ok(hash)
}

/// Read one `Content-Length`-framed LSP message body from `reader`, or `None`
/// at clean EOF. Ignores headers other than `Content-Length` (e.g. the optional
/// `Content-Type`).
async fn read_lsp_message<R>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};

    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None); // EOF
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // blank line terminates headers
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse().ok();
        }
    }

    let len = content_length.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "LSP frame missing Content-Length",
        )
    })?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(Some(body))
}

/// Run `git -C <repo> <args>`, returning trimmed stdout, bailing on failure.
/// Git blob oid of `file`'s current on-disk content, or `None` if it doesn't
/// exist / isn't hashable. Used as an optimistic-concurrency token for edits.
async fn git_blob_oid(repo: &Utf8Path, file: &Utf8Path) -> Option<String> {
    let oid = git(repo, &["hash-object", file.as_str()]).await.ok()?;
    (!oid.is_empty()).then_some(oid)
}

async fn git(repo: &Utf8Path, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo.as_str())
        .args(args)
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run `git -C <repo> <args>` for its exit status only (no bail on failure).
async fn git_status(repo: &Utf8Path, args: &[&str]) -> Result<std::process::ExitStatus> {
    Ok(tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo.as_str())
        .args(args)
        .status()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The editor preview overlays an edited buffer onto a **snapshot** of the
    /// live db and renders that. This proves the override is isolated: setting
    /// an input on the snapshot must NOT mutate the live db, or one user's
    /// preview would corrupt what every other viewer sees.
    #[tokio::test]
    async fn snapshot_input_override_is_isolated() {
        use crate::types::{SourceContent, SourcePath};

        let db = Database::new(None);
        let key = SourcePath::new("kb/overview.md".to_string());
        let live = SourceFile::new(&db, key.clone(), SourceContent::new("LIVE".to_string()), 0)
            .expect("create live source");
        SourceRegistry::set(&db, vec![live]).expect("set live sources");

        // Snapshot, then override the one file in the snapshot's own copy.
        let snapshot = DatabaseSnapshot::from_database(&db).await;
        let edited = SourceFile::new(
            &snapshot,
            key.clone(),
            SourceContent::new("EDITED".to_string()),
            0,
        )
        .expect("create edited source");
        SourceRegistry::set(&snapshot, vec![edited]).expect("set edited sources on snapshot");

        // Snapshot sees the edit...
        let snap_sources = SourceRegistry::sources(&snapshot)
            .unwrap()
            .unwrap_or_default();
        assert_eq!(snap_sources.len(), 1);
        assert_eq!(
            snap_sources[0].content(&snapshot).unwrap().as_str(),
            "EDITED",
            "snapshot must see the overlaid buffer"
        );

        // ...but the live db is untouched.
        let live_sources = SourceRegistry::sources(&db).unwrap().unwrap_or_default();
        assert_eq!(live_sources.len(), 1);
        assert_eq!(
            live_sources[0].content(&db).unwrap().as_str(),
            "LIVE",
            "live db must NOT be mutated by a snapshot override"
        );
    }

    #[test]
    fn parses_associated_app_output() {
        let app = associated_app_from_osascript_output(
            "/Applications/Visual Studio Code.app\ncom.microsoft.VSCode\n",
        )
        .expect("app");

        assert_eq!(
            app,
            AssociatedApp {
                path: Utf8PathBuf::from("/Applications/Visual Studio Code.app"),
                bundle_id: Some("com.microsoft.VSCode".to_string()),
            }
        );
    }

    #[test]
    fn vscode_uses_goto_path_line() {
        let app = AssociatedApp {
            path: Utf8PathBuf::from("/tmp/NoSuchApp/Visual Studio Code.app"),
            bundle_id: Some("com.microsoft.VSCode".to_string()),
        };

        let command =
            line_aware_editor_command(Some(&app), Utf8Path::new("/site/content/page.md"), 42)
                .expect("known editor command");

        assert_eq!(
            command,
            EditorCommand {
                editor: "Visual Studio Code",
                program: "code".to_string(),
                args: vec!["--goto".to_string(), "/site/content/page.md:42".to_string()],
            }
        );
    }

    #[test]
    fn unknown_associated_app_falls_back_to_plain_open() {
        let app = AssociatedApp {
            path: Utf8PathBuf::from("/Applications/Warp.app"),
            bundle_id: Some("dev.warp.Warp-Stable".to_string()),
        };

        assert!(
            line_aware_editor_command(Some(&app), Utf8Path::new("/site/content/page.md"), 42)
                .is_none()
        );
    }

    #[test]
    fn wiki_dead_link_stub_uses_wiki_key_and_title() {
        let stub = dead_link_stub_for_target(&DeadLinkTarget::Wiki {
            key: "missing-page".to_string(),
            title: "Missing Page".to_string(),
        })
        .expect("stub");

        assert_eq!(stub.source_file, Utf8PathBuf::from("missing-page.md"));
        assert_eq!(stub.title, "Missing Page");
        assert_eq!(
            dead_link_stub_content(&stub.title),
            "+++\ntitle = \"Missing Page\"\n+++\n"
        );
    }

    #[test]
    fn internal_dead_link_stub_maps_route_to_markdown_file() {
        let stub = dead_link_stub_for_target(&DeadLinkTarget::Internal {
            href: "/guide/new-page/#intro".to_string(),
            title: String::new(),
        })
        .expect("stub");

        assert_eq!(stub.source_file, Utf8PathBuf::from("guide/new-page.md"));
        assert_eq!(stub.title, "New Page");
    }

    #[test]
    fn dead_link_stub_escapes_toml_title() {
        assert_eq!(
            dead_link_stub_content("A \"quoted\" title"),
            "+++\ntitle = \"A \\\"quoted\\\" title\"\n+++\n"
        );
    }

    #[test]
    fn internal_dead_link_stub_rejects_root() {
        let err = dead_link_stub_for_target(&DeadLinkTarget::Internal {
            href: "/".to_string(),
            title: String::new(),
        })
        .expect_err("root should not be authorable");

        assert!(err.to_string().contains("content root"));
    }
}
