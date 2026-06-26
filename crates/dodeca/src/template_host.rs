//! TemplateHost implementation for gingembre rendering.
//!
//! This module provides the host-side implementation that gingembre calls back
//! to during template rendering.
//!
//! # Architecture
//!
//! When a render request is initiated:
//! 1. The host creates a `RenderContext` with pre-loaded templates
//! 2. The context is registered with a unique `ContextId`
//! 3. The renderer runs with `context_id`, template name, and initial context
//! 4. Gingembre calls back to `TemplateHost` methods as needed
//! 5. The host services callbacks using the registered context
//! 6. After rendering, the context is unregistered

use cell_gingembre_proto::{
    CallFunctionResult, ContextId, KeysAtResult, LoadTemplateResult, ResolveDataResult,
    TemplateHost,
};
use facet_value::{DestructuredRef, VString, Value};
use futures_util::future::BoxFuture;
use std::collections::HashMap;
use std::sync::Arc;

use crate::db::{Database, SiteTree};
use crate::queries::{DataValuePath, data_keys_at_path, resolve_data_value};
use crate::render::{get_base_url, path_to_route, section_to_value};

pub const TEMPLATE_FUNCTION_NAMES: &[&str] = &[
    "get_url",
    "get_section",
    "now",
    "throw",
    "build",
    "read",
    "highlight",
    "get_media",
    "markup",
];

/// Escape a string for insertion into a double-quoted HTML attribute value.
fn attr_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Convert a Value to a string representation (for template function args)
fn value_to_string(value: &Value) -> String {
    match value.destructure_ref() {
        DestructuredRef::Null => String::new(),
        DestructuredRef::Bool(b) => if b { "true" } else { "false" }.to_string(),
        DestructuredRef::Number(n) => {
            if let Some(i) = n.to_i64() {
                i.to_string()
            } else if let Some(f) = n.to_f64() {
                f.to_string()
            } else {
                "0".to_string()
            }
        }
        DestructuredRef::String(s) => s.to_string(),
        DestructuredRef::Bytes(b) => format!("<bytes: {} bytes>", b.len()),
        DestructuredRef::Array(arr) => {
            let items: Vec<String> = arr.iter().map(value_to_string).collect();
            format!("[{}]", items.join(", "))
        }
        DestructuredRef::Object(_) => "[object]".to_string(),
        DestructuredRef::DateTime(dt) => format!("{:?}", dt),
        DestructuredRef::QName(qn) => format!("{:?}", qn),
        DestructuredRef::Uuid(uuid) => format!("{:?}", uuid),
        DestructuredRef::Char(c) => c.to_string(),
        other => format!("{other:?}"),
    }
}

// ============================================================================
// Render Context Registry
// ============================================================================

/// A render context containing everything needed to service callbacks.
pub struct RenderContext {
    /// Pre-loaded templates (path -> source)
    pub templates: HashMap<String, String>,
    /// Reference to the database for data resolution
    /// Note: We store the db directly because the render context lives
    /// for the duration of a render call, during which the caller holds
    /// the db reference.
    db: Arc<Database>,
    /// The site tree for template functions like get_section
    site_tree: Arc<SiteTree>,
    /// Route currently being rendered, e.g. `/wiki/page/`. Lets source-scoped
    /// host functions (like `build()`) resolve which source owns this render.
    route: String,
}

impl RenderContext {
    /// Create a new render context.
    pub fn new(
        templates: HashMap<String, String>,
        db: Arc<Database>,
        site_tree: Arc<SiteTree>,
        route: String,
    ) -> Self {
        Self {
            templates,
            db,
            site_tree,
            route,
        }
    }
}

// Note: Render context registry is now in Host (crate::host::Host)

// ============================================================================
// TemplateHost Implementation
// ============================================================================

/// Host-side implementation of the TemplateHost service.
///
/// This is called by gingembre during template rendering to:
/// - Load templates by name
/// - Resolve data values at paths (with picante dependency tracking)
/// - Get keys at data paths (for iteration)
///
/// Render contexts are stored in the global Host singleton.
#[derive(Clone)]
pub struct TemplateHostImpl;

impl TemplateHostImpl {
    /// Create a new TemplateHost implementation.
    pub fn new() -> Self {
        Self
    }
}

impl Default for TemplateHostImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateHost for TemplateHostImpl {
    fn load_template(
        &self,
        context_id: ContextId,
        name: String,
    ) -> BoxFuture<'_, LoadTemplateResult> {
        Box::pin(async move {
            let host = crate::host::Host::get();
            let Some(context) = host.get_render_context(context_id) else {
                tracing::warn!(
                    context_id = context_id.0,
                    name = %name,
                    "load_template: context not found"
                );
                return LoadTemplateResult::NotFound;
            };

            match context.templates.get(&name) {
                Some(source) => {
                    let absolute_path = crate::config::global_config()
                        .map(|c| {
                            c.content_dir
                                .parent()
                                .unwrap_or(&c.content_dir)
                                .join("templates")
                                .join(&name)
                                .to_string()
                        })
                        .unwrap_or_else(|| name.clone());

                    tracing::debug!(
                        context_id = context_id.0,
                        name = %name,
                        absolute_path = %absolute_path,
                        source_len = source.len(),
                        "load_template: found"
                    );
                    LoadTemplateResult::Found {
                        source: source.clone(),
                        absolute_path,
                    }
                }
                None => {
                    tracing::debug!(
                        context_id = context_id.0,
                        name = %name,
                        "load_template: not found"
                    );
                    LoadTemplateResult::NotFound
                }
            }
        })
    }

    fn resolve_data(
        &self,
        context_id: ContextId,
        path: Vec<String>,
    ) -> BoxFuture<'_, ResolveDataResult> {
        Box::pin(async move {
            let Some(context) = crate::host::Host::get().get_render_context(context_id) else {
                tracing::warn!(
                    context_id = context_id.0,
                    path = ?path,
                    "resolve_data: context not found"
                );
                return ResolveDataResult::NotFound;
            };

            let data_path = match DataValuePath::new(&*context.db, path.clone()) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(
                        context_id = context_id.0,
                        path = ?path,
                        error = ?e,
                        "resolve_data: failed to create path"
                    );
                    return ResolveDataResult::NotFound;
                }
            };

            match resolve_data_value(&*context.db, data_path).await {
                Ok(Some(value)) => {
                    tracing::debug!(
                        context_id = context_id.0,
                        path = ?path,
                        "resolve_data: found"
                    );
                    ResolveDataResult::Found { value }
                }
                Ok(None) => {
                    tracing::debug!(
                        context_id = context_id.0,
                        path = ?path,
                        "resolve_data: not found"
                    );
                    ResolveDataResult::NotFound
                }
                Err(e) => {
                    tracing::warn!(
                        context_id = context_id.0,
                        path = ?path,
                        error = ?e,
                        "resolve_data: query error"
                    );
                    ResolveDataResult::NotFound
                }
            }
        })
    }

    fn keys_at(&self, context_id: ContextId, path: Vec<String>) -> BoxFuture<'_, KeysAtResult> {
        Box::pin(async move {
            let Some(context) = crate::host::Host::get().get_render_context(context_id) else {
                tracing::warn!(
                    context_id = context_id.0,
                    path = ?path,
                    "keys_at: context not found"
                );
                return KeysAtResult::NotFound;
            };

            let data_path = match DataValuePath::new(&*context.db, path.clone()) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(
                        context_id = context_id.0,
                        path = ?path,
                        error = ?e,
                        "keys_at: failed to create path"
                    );
                    return KeysAtResult::NotFound;
                }
            };

            match data_keys_at_path(&*context.db, data_path).await {
                Ok(keys) => {
                    tracing::debug!(
                        context_id = context_id.0,
                        path = ?path,
                        num_keys = keys.len(),
                        "keys_at: found"
                    );
                    KeysAtResult::Found { keys }
                }
                Err(e) => {
                    tracing::warn!(
                        context_id = context_id.0,
                        path = ?path,
                        error = ?e,
                        "keys_at: query error"
                    );
                    KeysAtResult::NotFound
                }
            }
        })
    }

    fn call_function(
        &self,
        context_id: ContextId,
        name: String,
        args: Vec<Value>,
        kwargs: Vec<(String, Value)>,
    ) -> BoxFuture<'_, CallFunctionResult> {
        Box::pin(async move {
            let Some(context) = crate::host::Host::get().get_render_context(context_id) else {
                tracing::warn!(
                    context_id = context_id.0,
                    name = %name,
                    "call_function: context not found"
                );
                return CallFunctionResult::Error {
                    message: "Context not found".to_string(),
                };
            };

            tracing::debug!(
                context_id = context_id.0,
                name = %name,
                num_args = args.len(),
                num_kwargs = kwargs.len(),
                "call_function"
            );

            let get_kwarg = |key: &str| -> Option<String> {
                kwargs
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| value_to_string(v))
            };

            match name.as_str() {
                "get_url" => {
                    let path = get_kwarg("path").unwrap_or_default();
                    let url = if path.starts_with('/') {
                        path
                    } else if path.is_empty() {
                        "/".to_string()
                    } else {
                        format!("/{path}")
                    };
                    CallFunctionResult::Success {
                        value: Value::from(url.as_str()),
                    }
                }

                "get_section" => {
                    let path = get_kwarg("path").unwrap_or_default();
                    let route = path_to_route(&path);

                    let result = if let Some(section) = context.site_tree.sections.get(&route) {
                        let base_url = get_base_url();
                        section_to_value(section, &context.site_tree, &base_url)
                    } else {
                        Value::NULL
                    };

                    CallFunctionResult::Success { value: result }
                }

                "now" => {
                    let format = get_kwarg("format").unwrap_or_else(|| "%Y-%m-%d".to_string());
                    let now = chrono::Local::now();
                    let formatted = now.format(&format).to_string();
                    CallFunctionResult::Success {
                        value: Value::from(formatted.as_str()),
                    }
                }

                "throw" => {
                    let message = args
                        .first()
                        .map(value_to_string)
                        .or_else(|| get_kwarg("message"))
                        .unwrap_or_else(|| "Template error".to_string());
                    CallFunctionResult::Error { message }
                }

                "build" => {
                    tracing::debug!(
                        num_args = args.len(),
                        num_kwargs = kwargs.len(),
                        "build() function called"
                    );
                    let step_name = match args.first() {
                        Some(v) => value_to_string(v),
                        None => {
                            return CallFunctionResult::Error {
                                message: "build() requires step name as first argument".to_string(),
                            };
                        }
                    };

                    let params: std::collections::HashMap<String, String> = kwargs
                        .iter()
                        .map(|(k, v)| (k.clone(), value_to_string(v)))
                        .collect();

                    let executor = match crate::host::Host::get().build_step_executor() {
                        Some(e) => e.clone(),
                        None => {
                            return CallFunctionResult::Error {
                                message: "Build step executor not initialized".to_string(),
                            };
                        }
                    };

                    // Build steps are source-scoped: resolve which source owns the
                    // route being rendered, so `build("git_hash")` runs that
                    // source's step in its own dir.
                    let mount = crate::config::global_config()
                        .map(|cfg| {
                            crate::build_context::source_for_route(&context.route, &cfg.sources)
                                .to_string()
                        })
                        .unwrap_or_else(|| "/".to_string());

                    let result = executor.execute(&mount, &step_name, &params).await;
                    match result {
                        crate::build_steps::BuildStepResult::Success(bytes) => {
                            match String::from_utf8(bytes) {
                                Ok(s) => CallFunctionResult::Success {
                                    value: Value::from(s.as_str()),
                                },
                                Err(e) => CallFunctionResult::Error {
                                    message: format!("Build step output is not valid UTF-8: {}", e),
                                },
                            }
                        }
                        crate::build_steps::BuildStepResult::Error(msg) => {
                            CallFunctionResult::Error { message: msg }
                        }
                    }
                }

                "read" => {
                    let file_path = match get_kwarg("file") {
                        Some(p) => p,
                        None => {
                            return CallFunctionResult::Error {
                                message: "read() requires 'file' parameter".to_string(),
                            };
                        }
                    };

                    let project_root = crate::config::global_config()
                        .map(|c| c._root.clone())
                        .unwrap_or_else(|| camino::Utf8PathBuf::from("."));

                    let result = crate::build_steps::builtin_read(&project_root, &file_path).await;
                    match result {
                        crate::build_steps::BuildStepResult::Success(bytes) => {
                            match String::from_utf8(bytes) {
                                Ok(s) => CallFunctionResult::Success {
                                    value: Value::from(s.as_str()),
                                },
                                Err(e) => CallFunctionResult::Error {
                                    message: format!("File content is not valid UTF-8: {}", e),
                                },
                            }
                        }
                        crate::build_steps::BuildStepResult::Error(msg) => {
                            CallFunctionResult::Error { message: msg }
                        }
                    }
                }

                "highlight" => {
                    let lang = get_kwarg("lang").unwrap_or_default();
                    let body = get_kwarg("body").unwrap_or_default();
                    let body = body.trim();

                    match crate::cells::highlight_code(&lang, body).await {
                        Ok(html) => CallFunctionResult::Success {
                            value: Value::from(html.as_str()),
                        },
                        Err(e) => CallFunctionResult::Error {
                            message: format!("highlight error: {}", e),
                        },
                    }
                }

                "get_media" => {
                    // A media handle is just its resolved source path. `.markup(...)` (the
                    // `markup` function below) turns it into an <img>, which the page-render
                    // image post-pass then upgrades to a responsive <picture> and tracks as
                    // a dependency — so get_media itself stays a thin path resolver.
                    let src = args.first().map(value_to_string).unwrap_or_default();
                    CallFunctionResult::Success {
                        value: Value::from(src.as_str()),
                    }
                }

                // Method on a media handle: `get_media(src).markup(alt=, width=, height=, class=)`.
                // The receiver (the src) is passed as the first positional arg by gingembre's
                // value-method dispatch. Emits a plain <img>; the image post-pass makes it
                // responsive. Returned as safe HTML so `{{ ...markup(...) }}` isn't escaped.
                "markup" => {
                    let src = args.first().map(value_to_string).unwrap_or_default();
                    let mut img = format!(r#"<img src="{}""#, attr_escape(&src));
                    for attr in ["alt", "title", "width", "height", "class", "loading"] {
                        match get_kwarg(attr) {
                            // Skip empty/undefined (e.g. an optional `width=width?` that
                            // resolved to null) so we don't emit `width=""`.
                            Some(v) if !v.is_empty() => {
                                img.push_str(&format!(r#" {}="{}""#, attr, attr_escape(&v)));
                            }
                            _ => {}
                        }
                    }
                    img.push('>');
                    CallFunctionResult::Success {
                        value: VString::from(img.as_str()).into_safe().into_value(),
                    }
                }

                _ => {
                    tracing::warn!(
                        context_id = context_id.0,
                        name = %name,
                        "call_function: unknown function"
                    );
                    CallFunctionResult::Error {
                        message: format!("Unknown function: {}", name),
                    }
                }
            }
        })
    }
}

// ============================================================================
// Global Registry
// ============================================================================
// Helper for creating render contexts
// ============================================================================

/// RAII guard that automatically unregisters the context when dropped.
///
/// Uses `Host::get()` internally to manage the render context lifecycle.
pub struct RenderContextGuard {
    id: ContextId,
}

impl RenderContextGuard {
    /// Create a new guard that registers the context with the Host.
    pub fn new(context: RenderContext) -> Self {
        let id = crate::host::Host::get().register_render_context(context);
        Self { id }
    }

    /// Get the context ID.
    pub fn id(&self) -> ContextId {
        self.id
    }
}

impl Drop for RenderContextGuard {
    fn drop(&mut self) {
        crate::host::Host::get().unregister_render_context(self.id);
    }
}
