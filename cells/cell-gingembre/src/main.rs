//! Template rendering cell using gingembre.
//!
//! This cell handles template rendering with bidirectional RPC:
//! - Receives render requests from the host
//! - Calls back to host for template loading, data resolution, and function calls

use cell_gingembre_proto::{
    CallFunctionResult, ContextId, EvalResult, KeysAtResult, LoadTemplateResult, RenderResult,
    ResolveDataResult, TemplateHostClient, TemplateRenderer, TemplateRendererServer,
};
use facet_value::DestructuredRef;
use futures::future::BoxFuture;
use gingembre::{Context, DataPath, DataResolver, Engine, TemplateLoader, Value};
use rapace::transport::shm::ShmTransport;
use rapace_cell::CellSession;
use std::sync::Arc;

/// Cell context holding the RPC session for callbacks
pub struct CellContext {
    pub session: Arc<CellSession>,
}

impl CellContext {
    /// Create a client for calling back to the host
    pub fn host_client(&self) -> TemplateHostClient<ShmTransport> {
        TemplateHostClient::new(self.session.clone())
    }
}

// ============================================================================
// RPC-backed TemplateLoader
// ============================================================================

/// Template loader that calls back to the host via RPC.
struct RpcTemplateLoader {
    client: TemplateHostClient<ShmTransport>,
    context_id: ContextId,
}

impl RpcTemplateLoader {
    fn new(client: TemplateHostClient<ShmTransport>, context_id: ContextId) -> Self {
        Self { client, context_id }
    }
}

impl TemplateLoader for RpcTemplateLoader {
    fn load(&self, name: &str) -> BoxFuture<'_, Option<String>> {
        let name = name.to_string();
        Box::pin(async move {
            match self.client.load_template(self.context_id, name).await {
                Ok(LoadTemplateResult::Found { source }) => Some(source),
                Ok(LoadTemplateResult::NotFound) => None,
                Err(e) => {
                    tracing::warn!("RPC error loading template: {:?}", e);
                    None
                }
            }
        })
    }
}

// ============================================================================
// RPC-backed DataResolver
// ============================================================================

/// Data resolver that calls back to the host via RPC.
struct RpcDataResolver {
    client: TemplateHostClient<ShmTransport>,
    context_id: ContextId,
}

impl RpcDataResolver {
    fn new(client: TemplateHostClient<ShmTransport>, context_id: ContextId) -> Self {
        Self { client, context_id }
    }
}

impl DataResolver for RpcDataResolver {
    fn resolve(
        &self,
        path: &DataPath,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Value>> + Send + '_>> {
        let path_segments = path.segments().to_vec();
        Box::pin(async move {
            match self
                .client
                .resolve_data(self.context_id, path_segments)
                .await
            {
                Ok(ResolveDataResult::Found { value }) => Some(value),
                Ok(ResolveDataResult::NotFound) => None,
                Err(e) => {
                    tracing::warn!("RPC error resolving data: {:?}", e);
                    None
                }
            }
        })
    }

    fn keys_at(
        &self,
        path: &DataPath,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Vec<String>>> + Send + '_>> {
        let path_segments = path.segments().to_vec();
        Box::pin(async move {
            match self.client.keys_at(self.context_id, path_segments).await {
                Ok(KeysAtResult::Found { keys }) => Some(keys),
                Ok(KeysAtResult::NotFound) => None,
                Err(e) => {
                    tracing::warn!("RPC error getting keys: {:?}", e);
                    None
                }
            }
        })
    }
}

// ============================================================================
// RPC-backed function caller
// ============================================================================

/// Creates a function that calls back to the host via RPC.
fn make_rpc_function(
    session: Arc<CellSession>,
    context_id: ContextId,
    name: String,
) -> gingembre::GlobalFn {
    Box::new(move |args: &[Value], kwargs: &[(String, Value)]| {
        let client = TemplateHostClient::new(session.clone());
        let name = name.clone();
        let args = args.to_vec();
        let kwargs = kwargs.to_vec();

        Box::pin(async move {
            match client.call_function(context_id, name, args, kwargs).await {
                Ok(CallFunctionResult::Success { value }) => Ok(value),
                Ok(CallFunctionResult::Error { message }) => Err(miette::miette!(message)),
                Err(e) => Err(miette::miette!("RPC error calling function: {:?}", e)),
            }
        })
    })
}

// ============================================================================
// Template renderer implementation
// ============================================================================

/// Template renderer implementation
pub struct TemplateRendererImpl {
    ctx: Arc<CellContext>,
}

impl TemplateRendererImpl {
    pub fn new(ctx: Arc<CellContext>) -> Self {
        Self { ctx }
    }

    /// Build a render context from initial variables
    fn build_context(
        &self,
        initial_context: &Value,
        resolver: Arc<dyn DataResolver>,
        context_id: ContextId,
    ) -> Context {
        let mut ctx = Context::new();

        // Set the data resolver for lazy data loading
        ctx.set_data_resolver(resolver);

        // Register RPC-backed functions
        // These are the standard functions that templates expect
        let function_names = ["get_url", "get_section", "now", "throw"];
        for name in function_names {
            let func = make_rpc_function(self.ctx.session.clone(), context_id, name.to_string());
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

impl TemplateRenderer for TemplateRendererImpl {
    async fn render(
        &self,
        context_id: ContextId,
        template_name: String,
        initial_context: Value,
    ) -> RenderResult {
        // Create RPC-backed loader and resolver
        let loader = RpcTemplateLoader::new(self.ctx.host_client(), context_id);
        let resolver = Arc::new(RpcDataResolver::new(self.ctx.host_client(), context_id));

        // Build the render context
        let ctx = self.build_context(&initial_context, resolver, context_id);

        // Create engine and render
        let mut engine = Engine::new(loader);
        match engine.render(&template_name, &ctx).await {
            Ok(html) => RenderResult::Success { html },
            Err(e) => {
                // Format the error with rich diagnostics
                let message = format!("{:?}", e);
                RenderResult::Error { message }
            }
        }
    }

    async fn eval_expression(
        &self,
        context_id: ContextId,
        expression: String,
        context: Value,
    ) -> EvalResult {
        // Create RPC-backed resolver (no loader needed for expression eval)
        let resolver = Arc::new(RpcDataResolver::new(self.ctx.host_client(), context_id));

        // Build the context
        let ctx = self.build_context(&context, resolver, context_id);

        // Evaluate the expression
        match gingembre::eval_expression(&expression, &ctx).await {
            Ok(value) => EvalResult::Success { value },
            Err(e) => {
                let message = format!("{:?}", e);
                EvalResult::Error { message }
            }
        }
    }
}

rapace_cell::cell_service!(
    TemplateRendererServer<TemplateRendererImpl>,
    TemplateRendererImpl
);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run_with_session(|session: Arc<CellSession>| {
        let ctx = Arc::new(CellContext { session });
        let renderer = TemplateRendererImpl::new(ctx);
        CellService::from(renderer)
    })
    .await?;
    Ok(())
}
