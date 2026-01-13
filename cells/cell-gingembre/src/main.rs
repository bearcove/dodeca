//! Template rendering cell using gingembre.
//!
//! This cell handles template rendering with bidirectional RPC:
//! - Receives render requests from the host
//! - Calls back to host for template loading, data resolution, and function calls

use cell_gingembre_proto::{
    CallFunctionResult, ContextId, EvalResult, KeysAtResult, LoadTemplateResult, RenderResult,
    ResolveDataResult, TemplateHostClient, TemplateRenderer, TemplateRendererDispatcher,
};
use cell_lifecycle_proto::{CellLifecycleClient, ReadyMsg};
use facet_value::DestructuredRef;
use futures::future::BoxFuture;
use gingembre::{Context, DataPath, DataResolver, Engine, TemplateLoader, Value};
use roam::session::{ConnectionHandle, RoutedDispatcher};
use roam_shm::driver::establish_guest;
use roam_shm::guest::ShmGuest;
use roam_shm::spawn::SpawnArgs;
use roam_shm::transport::ShmGuestTransport;
use roam_tracing::{CellTracingDispatcher, init_cell_tracing};
use std::sync::Arc;
use tracing_subscriber::prelude::*;

/// Cell context holding the connection handle for callbacks
pub struct CellContext {
    pub handle: ConnectionHandle,
}

impl CellContext {
    /// Create a client for calling back to the host
    pub fn host_client(&self) -> TemplateHostClient {
        TemplateHostClient::new(self.handle.clone())
    }
}

// ============================================================================
// RPC-backed TemplateLoader
// ============================================================================

/// Template loader that calls back to the host via RPC.
struct RpcTemplateLoader {
    client: TemplateHostClient,
    context_id: ContextId,
}

impl RpcTemplateLoader {
    fn new(client: TemplateHostClient, context_id: ContextId) -> Self {
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
    client: TemplateHostClient,
    context_id: ContextId,
}

impl RpcDataResolver {
    fn new(client: TemplateHostClient, context_id: ContextId) -> Self {
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
    handle: ConnectionHandle,
    context_id: ContextId,
    name: String,
) -> gingembre::GlobalFn {
    Box::new(move |args: &[Value], kwargs: &[(String, Value)]| {
        let client = TemplateHostClient::new(handle.clone());
        let name = name.clone();
        let args = args.to_vec();
        let kwargs = kwargs.to_vec();

        Box::pin(async move {
            match client.call_function(context_id, name, args, kwargs).await {
                Ok(CallFunctionResult::Success { value }) => Ok(value),
                Ok(CallFunctionResult::Error { message }) => Err(eyre::eyre!(message)),
                Err(e) => Err(eyre::eyre!("RPC error calling function: {:?}", e)),
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
            let func = make_rpc_function(self.ctx.handle.clone(), context_id, name.to_string());
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::OnceLock;

    let args = SpawnArgs::from_env()?;
    let guest = ShmGuest::attach_with_ticket(&args)?;
    let transport = ShmGuestTransport::new(guest);

    // Initialize cell-side tracing
    let (tracing_layer, tracing_service) = init_cell_tracing(1024);
    tracing_subscriber::registry().with(tracing_layer).init();

    // Use OnceLock to lazily initialize the handle after establish_guest returns
    let handle_cell: Arc<OnceLock<ConnectionHandle>> = Arc::new(OnceLock::new());

    // Create a wrapper context that lazily gets the handle
    #[derive(Clone)]
    struct LazyContext {
        handle_cell: Arc<OnceLock<ConnectionHandle>>,
    }

    impl LazyContext {
        fn handle(&self) -> &ConnectionHandle {
            self.handle_cell.get().expect("handle not initialized")
        }

        fn host_client(&self) -> TemplateHostClient {
            TemplateHostClient::new(self.handle().clone())
        }
    }

    // Template renderer that uses the lazy context
    #[derive(Clone)]
    struct LazyTemplateRendererImpl {
        ctx: Arc<LazyContext>,
    }

    impl TemplateRenderer for LazyTemplateRendererImpl {
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
            let ctx = build_context(
                self.ctx.handle().clone(),
                &initial_context,
                resolver,
                context_id,
            );

            // Create engine and render
            let mut engine = Engine::new(loader);
            match engine.render(&template_name, &ctx).await {
                Ok(html) => RenderResult::Success { html },
                Err(e) => {
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
            let resolver = Arc::new(RpcDataResolver::new(self.ctx.host_client(), context_id));
            let ctx = build_context(self.ctx.handle().clone(), &context, resolver, context_id);

            match gingembre::eval_expression(&expression, &ctx).await {
                Ok(value) => EvalResult::Success { value },
                Err(e) => {
                    let message = format!("{:?}", e);
                    EvalResult::Error { message }
                }
            }
        }
    }

    fn build_context(
        handle: ConnectionHandle,
        initial_context: &Value,
        resolver: Arc<dyn DataResolver>,
        context_id: ContextId,
    ) -> Context {
        let mut ctx = Context::new();
        ctx.set_data_resolver(resolver);

        // Register RPC-backed functions
        let function_names = ["get_url", "get_section", "now", "throw"];
        for name in function_names {
            let func = make_rpc_function(handle.clone(), context_id, name.to_string());
            ctx.register_fn(name, func);
        }

        // Set initial context variables from the Value (should be a VObject)
        if let DestructuredRef::Object(obj) = initial_context.destructure_ref() {
            for (key, value) in obj.iter() {
                ctx.set(key.to_string(), value.clone());
            }
        }

        ctx
    }

    let lazy_ctx = Arc::new(LazyContext {
        handle_cell: handle_cell.clone(),
    });
    let renderer = LazyTemplateRendererImpl { ctx: lazy_ctx };
    let user_dispatcher = TemplateRendererDispatcher::new(renderer);

    // Combine user's dispatcher with tracing dispatcher
    let tracing_dispatcher = CellTracingDispatcher::new(tracing_service);
    let dispatcher = RoutedDispatcher::new(
        tracing_dispatcher, // primary: handles tracing methods
        user_dispatcher,    // fallback: handles all cell-specific methods
    );

    let (handle, driver) = establish_guest(transport, dispatcher);

    // Spawn driver in background - it needs to run to process RPCs
    let driver_handle = tokio::spawn(async move {
        if let Err(e) = driver.run().await {
            eprintln!("Driver error: {:?}", e);
        }
    });

    // Now initialize the handle cell
    let _ = handle_cell.set(handle.clone());

    // Signal readiness to host
    let lifecycle = CellLifecycleClient::new(handle.clone());
    lifecycle
        .ready(ReadyMsg {
            peer_id: args.peer_id.get() as u16,
            cell_name: "gingembre".to_string(),
            pid: Some(std::process::id()),
            version: None,
            features: vec![],
        })
        .await?;

    // Wait for driver
    let _ = driver_handle.await;
    Ok(())
}
