//! Template rendering processor using gingembre.
//!
//! This processor handles template rendering with typed host callbacks:
//! - Receives render requests from Dodeca
//! - Calls back for template loading, data resolution, and function calls

use cell_gingembre_proto::{
    CallFunctionResult, ContextId, ErrorLocation, EvalResult, KeysAtResult, LoadTemplateResult,
    RenderResult, ResolveDataResult, TemplateHost, TemplateRenderError, TemplateRenderer,
};
use dashmap::DashMap;
use facet_value::DestructuredRef;
use futures::future::BoxFuture;
use gingembre::{
    Context, DataPath, DataResolver, Engine, PrettyError, RenderError, TemplateError,
    TemplateLoader, Value,
};
use std::sync::Arc;
use std::time::Instant;

/// Shared mapping from template name to absolute path.
/// Used to convert relative template names to absolute paths in error messages.
type PathMap = Arc<DashMap<String, String>>;

/// Template loader that calls back through a host adapter.
struct HostTemplateLoader<H> {
    host: H,
    context_id: ContextId,
    /// Shared map from template name to absolute path
    path_map: PathMap,
}

impl<H> HostTemplateLoader<H> {
    fn new(host: H, context_id: ContextId, path_map: PathMap) -> Self {
        Self {
            host,
            context_id,
            path_map,
        }
    }
}

impl<H> TemplateLoader for HostTemplateLoader<H>
where
    H: TemplateHost + Clone + Send + Sync + 'static,
{
    fn load(&self, name: &str) -> BoxFuture<'_, Option<String>> {
        let name = name.to_string();
        Box::pin(async move {
            match self.host.load_template(self.context_id, name.clone()).await {
                LoadTemplateResult::Found {
                    source,
                    absolute_path,
                } => {
                    // Store the mapping for error reporting
                    self.path_map.insert(name, absolute_path);
                    Some(source)
                }
                LoadTemplateResult::NotFound => None,
            }
        })
    }
}

// ============================================================================
// Callback-backed DataResolver
// ============================================================================

/// Data resolver that calls back through a host adapter.
struct HostDataResolver<H> {
    host: H,
    context_id: ContextId,
}

impl<H> HostDataResolver<H> {
    fn new(host: H, context_id: ContextId) -> Self {
        Self { host, context_id }
    }
}

impl<H> DataResolver for HostDataResolver<H>
where
    H: TemplateHost + Clone + Send + Sync + 'static,
{
    fn resolve(
        &self,
        path: &DataPath,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Value>> + Send + '_>> {
        let path_segments = path.segments().to_vec();
        Box::pin(async move {
            match self.host.resolve_data(self.context_id, path_segments).await {
                ResolveDataResult::Found { value } => Some(value),
                ResolveDataResult::NotFound => None,
            }
        })
    }

    fn keys_at(
        &self,
        path: &DataPath,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Vec<String>>> + Send + '_>> {
        let path_segments = path.segments().to_vec();
        Box::pin(async move {
            match self.host.keys_at(self.context_id, path_segments).await {
                KeysAtResult::Found { keys } => Some(keys),
                KeysAtResult::NotFound => None,
            }
        })
    }
}

// ============================================================================
// Callback-backed function caller
// ============================================================================

/// Creates a function that calls back through the typed host trait.
fn make_callback_function(
    host: impl TemplateHost + Clone + Send + Sync + 'static,
    context_id: ContextId,
    name: String,
) -> gingembre::GlobalFn {
    Box::new(move |args: &[Value], kwargs: &[(String, Value)]| {
        let host = host.clone();
        let name = name.clone();
        let args = args.to_vec();
        let kwargs = kwargs.to_vec();

        Box::pin(async move {
            match host.call_function(context_id, name, args, kwargs).await {
                CallFunctionResult::Success { value } => Ok(value),
                CallFunctionResult::Error { message } => Err(message.into()),
            }
        })
    })
}

// ============================================================================
// Error conversion
// ============================================================================

/// Convert a gingembre RenderError to a protocol TemplateRenderError
fn to_protocol_error(err: &RenderError, path_map: &PathMap) -> TemplateRenderError {
    match err {
        RenderError::NotFound(name) => TemplateRenderError {
            message: format!("Template not found: {}", name),
            location: None,
            help: None,
        },
        RenderError::Template(template_err) => to_protocol_template_error(template_err, path_map),
        RenderError::Other(msg) => TemplateRenderError {
            message: msg.clone(),
            location: None,
            help: None,
        },
    }
}

/// Convert a TemplateError to a protocol TemplateRenderError
fn to_protocol_template_error(err: &TemplateError, path_map: &PathMap) -> TemplateRenderError {
    // Helper to extract structured info from an error implementing PrettyError
    fn from_pretty<E: PrettyError>(e: &E, path_map: &PathMap) -> TemplateRenderError {
        let loc = e.source_loc();
        // Look up absolute path from our mapping, fall back to the name if not found
        let filename = path_map
            .get(&loc.src.name)
            .map(|r| r.value().clone())
            .unwrap_or_else(|| loc.src.name.clone());
        TemplateRenderError {
            message: e.message(),
            location: Some(ErrorLocation {
                filename,
                source: loc.src.source.clone(),
                offset: loc.span.offset(),
                length: loc.span.len(),
            }),
            help: e.help(),
        }
    }

    match err {
        TemplateError::Syntax(e) => from_pretty(e.as_ref(), path_map),
        TemplateError::UnknownField(e) => from_pretty(e.as_ref(), path_map),
        TemplateError::Type(e) => from_pretty(e.as_ref(), path_map),
        TemplateError::Undefined(e) => from_pretty(e.as_ref(), path_map),
        TemplateError::UnknownFilter(e) => from_pretty(e.as_ref(), path_map),
        TemplateError::UnknownTest(e) => from_pretty(e.as_ref(), path_map),
        TemplateError::MacroNotFound(e) => from_pretty(e.as_ref(), path_map),
        TemplateError::DataPathNotFound(e) => from_pretty(e.as_ref(), path_map),
        TemplateError::GlobalFn(msg) => TemplateRenderError {
            message: format!("Function error: {}", msg),
            location: None,
            help: None,
        },
    }
}

// ============================================================================
// Template renderer implementation
// ============================================================================

/// Template renderer implementation
#[derive(Clone)]
pub struct TemplateRendererImpl<H> {
    host: H,
}

impl<H> TemplateRendererImpl<H> {
    pub fn new(host: H) -> Self {
        Self { host }
    }

    /// Build a render context from initial variables
    fn build_context(
        &self,
        host: H,
        initial_context: &Value,
        resolver: Arc<dyn DataResolver>,
        context_id: ContextId,
    ) -> Context
    where
        H: TemplateHost + Clone + Send + Sync + 'static,
    {
        let mut ctx = Context::new();

        // Set the data resolver for lazy data loading
        ctx.set_data_resolver(resolver);

        // Register callback-backed functions.
        // These are the standard functions that templates expect
        let function_names = [
            "get_url",
            "get_section",
            "now",
            "throw",
            "build",
            "read",
            "highlight",
            "get_media",
        ];
        tracing::debug!(
            num_functions = function_names.len(),
            ?function_names,
            "registering callback-backed functions"
        );
        for name in function_names {
            let func = make_callback_function(host.clone(), context_id, name.to_string());
            ctx.register_fn(name, func);
        }

        // Set initial context variables from the Value (should be a VObject)
        if let DestructuredRef::Object(obj) = initial_context.destructure_ref() {
            let keys: Vec<_> = obj.iter().map(|(k, _)| k.to_string()).collect();
            tracing::debug!(
                context_id = context_id.0,
                keys = ?keys,
                "build_context: setting initial context variables"
            );
            for (key, value) in obj.iter() {
                ctx.set(key.to_string(), value.clone());
            }
        } else {
            tracing::warn!(
                context_id = context_id.0,
                initial_context_type = ?initial_context.destructure_ref(),
                "build_context: initial_context is NOT an object!"
            );
        }

        ctx
    }
}

impl<H> TemplateRenderer for TemplateRendererImpl<H>
where
    H: TemplateHost + Clone + Send + Sync + 'static,
{
    fn render(
        &self,
        context_id: ContextId,
        template_name: String,
        initial_context: Value,
    ) -> BoxFuture<'_, RenderResult> {
        Box::pin(async move {
            let started_at = Instant::now();
            tracing::debug!(
                context_id = context_id.0,
                template_name = %template_name,
                "gingembre render started"
            );
            let path_map: PathMap = Arc::new(DashMap::new());

            let loader = HostTemplateLoader::new(self.host.clone(), context_id, path_map.clone());
            let resolver = Arc::new(HostDataResolver::new(self.host.clone(), context_id));

            let ctx = self.build_context(self.host.clone(), &initial_context, resolver, context_id);

            let mut engine = Engine::new(loader);
            match engine.render(&template_name, &ctx).await {
                Ok(html) => {
                    tracing::debug!(
                        context_id = context_id.0,
                        template_name = %template_name,
                        elapsed_ms = started_at.elapsed().as_millis(),
                        html_len = html.len(),
                        "gingembre render finished"
                    );
                    RenderResult::Success { html }
                }
                Err(e) => {
                    tracing::error!(
                        context_id = context_id.0,
                        template_name = %template_name,
                        elapsed_ms = started_at.elapsed().as_millis(),
                        error = ?e,
                        "gingembre render failed"
                    );
                    RenderResult::Error {
                        error: to_protocol_error(&e, &path_map),
                    }
                }
            }
        })
    }

    fn eval_expression(
        &self,
        context_id: ContextId,
        expression: String,
        context: Value,
    ) -> BoxFuture<'_, EvalResult> {
        Box::pin(async move {
            let resolver = Arc::new(HostDataResolver::new(self.host.clone(), context_id));
            let ctx = self.build_context(self.host.clone(), &context, resolver, context_id);

            match gingembre::eval_expression(&expression, &ctx).await {
                Ok(value) => EvalResult::Success { value },
                Err(e) => {
                    let message = format!("{:?}", e);
                    EvalResult::Error { message }
                }
            }
        })
    }
}
