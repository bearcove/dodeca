//! TemplateHost service implementation for the gingembre cell.
//!
//! This module provides the host-side implementation of the TemplateHost service
//! that the gingembre cell calls back to during template rendering.
//!
//! # Architecture
//!
//! When a render request is initiated:
//! 1. The host creates a `RenderContext` with pre-loaded templates
//! 2. The context is registered with a unique `ContextId`
//! 3. The host calls `cell.render(context_id, template_name, initial_context)`
//! 4. The cell calls back to `TemplateHost` methods as needed
//! 5. The host services callbacks using the registered context
//! 6. After rendering, the context is unregistered

use cell_gingembre_proto::{
    CallFunctionResult, ContextId, KeysAtResult, LoadTemplateResult, ResolveDataResult,
    TemplateHost,
};
use facet_value::{DestructuredRef, VArray, VObject, VString, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::db::{Database, SiteTree};
use crate::queries::{DataValuePath, data_keys_at_path, resolve_data_value};
use crate::render::{headings_to_toc, path_to_route, route_to_path};

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
}

impl RenderContext {
    /// Create a new render context.
    pub fn new(
        templates: HashMap<String, String>,
        db: Arc<Database>,
        site_tree: Arc<SiteTree>,
    ) -> Self {
        Self {
            templates,
            db,
            site_tree,
        }
    }
}

// Note: Render context registry is now in Host (crate::host::Host)

// ============================================================================
// TemplateHost Implementation
// ============================================================================

/// Host-side implementation of the TemplateHost service.
///
/// This is called by the gingembre cell during template rendering to:
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
    async fn load_template(&self, context_id: ContextId, name: String) -> LoadTemplateResult {
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
                tracing::debug!(
                    context_id = context_id.0,
                    name = %name,
                    source_len = source.len(),
                    "load_template: found"
                );
                LoadTemplateResult::Found {
                    source: source.clone(),
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
    }

    async fn resolve_data(&self, context_id: ContextId, path: Vec<String>) -> ResolveDataResult {
        let Some(context) = crate::host::Host::get().get_render_context(context_id) else {
            tracing::warn!(
                context_id = context_id.0,
                path = ?path,
                "resolve_data: context not found"
            );
            return ResolveDataResult::NotFound;
        };

        // Create the interned path for picante tracking
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

        // Execute the tracked query
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
    }

    async fn keys_at(&self, context_id: ContextId, path: Vec<String>) -> KeysAtResult {
        let Some(context) = crate::host::Host::get().get_render_context(context_id) else {
            tracing::warn!(
                context_id = context_id.0,
                path = ?path,
                "keys_at: context not found"
            );
            return KeysAtResult::NotFound;
        };

        // Create the interned path for picante tracking
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

        // Execute the tracked query
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
    }

    async fn call_function(
        &self,
        context_id: ContextId,
        name: String,
        args: Vec<Value>,
        kwargs: Vec<(String, Value)>,
    ) -> CallFunctionResult {
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

        // Helper to get kwarg by name
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
                    let mut section_map = VObject::new();
                    section_map.insert(VString::from("title"), Value::from(section.title.as_str()));
                    section_map.insert(
                        VString::from("permalink"),
                        Value::from(section.route.as_str()),
                    );
                    section_map.insert(VString::from("path"), Value::from(path.as_str()));
                    section_map.insert(
                        VString::from("content"),
                        Value::from(section.body_html.as_str()),
                    );
                    section_map.insert(VString::from("toc"), headings_to_toc(&section.headings));
                    section_map.insert(VString::from("extra"), section.extra.clone());

                    let section_pages: Vec<Value> = context
                        .site_tree
                        .pages
                        .values()
                        .filter(|p| p.section_route == section.route)
                        .map(|p| {
                            let mut page_map = VObject::new();
                            page_map.insert(VString::from("title"), Value::from(p.title.as_str()));
                            page_map
                                .insert(VString::from("permalink"), Value::from(p.route.as_str()));
                            page_map.insert(
                                VString::from("path"),
                                Value::from(route_to_path(p.route.as_str()).as_str()),
                            );
                            page_map.insert(VString::from("weight"), Value::from(p.weight as i64));
                            page_map.insert(VString::from("toc"), headings_to_toc(&p.headings));
                            page_map.into()
                        })
                        .collect();
                    section_map.insert(VString::from("pages"), VArray::from_iter(section_pages));

                    let subsections: Vec<Value> = context
                        .site_tree
                        .sections
                        .values()
                        .filter(|s| {
                            s.route != section.route
                                && s.route.as_str().starts_with(section.route.as_str())
                                && s.route.as_str()[section.route.as_str().len()..]
                                    .trim_matches('/')
                                    .chars()
                                    .filter(|c| *c == '/')
                                    .count()
                                    == 0
                        })
                        .map(|s| Value::from(route_to_path(s.route.as_str()).as_str()))
                        .collect();
                    section_map
                        .insert(VString::from("subsections"), VArray::from_iter(subsections));

                    section_map.into()
                } else {
                    Value::NULL
                };

                CallFunctionResult::Success { value: result }
            }

            "now" => {
                // Return current timestamp (could add formatting support via kwargs)
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
                // Build step invocation: build(step_name, param1=val1, param2=val2, ...)
                tracing::debug!(num_args = args.len(), num_kwargs = kwargs.len(), "build() function called");
                let step_name = match args.first() {
                    Some(v) => value_to_string(v),
                    None => {
                        return CallFunctionResult::Error {
                            message: "build() requires step name as first argument".to_string(),
                        };
                    }
                };

                // Collect kwargs as params
                let params: std::collections::HashMap<String, String> = kwargs
                    .iter()
                    .map(|(k, v)| (k.clone(), value_to_string(v)))
                    .collect();

                // Get the executor
                let executor = match crate::host::Host::get().build_step_executor() {
                    Some(e) => e.clone(),
                    None => {
                        return CallFunctionResult::Error {
                            message: "Build step executor not initialized".to_string(),
                        };
                    }
                };

                // Execute the build step
                let result = executor.execute(&step_name, &params).await;
                match result {
                    crate::build_steps::BuildStepResult::Success(bytes) => {
                        // Return as string (UTF-8)
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
                // Built-in read function: read(file="path/to/file")
                let file_path = match get_kwarg("file") {
                    Some(p) => p,
                    None => {
                        return CallFunctionResult::Error {
                            message: "read() requires 'file' parameter".to_string(),
                        };
                    }
                };

                // Get project root from config
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
