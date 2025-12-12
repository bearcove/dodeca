use std::sync::Arc;

use rapace::RpcSession;
use rapace::Transport;
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};
use rapace_tracing::{RapaceTracingLayer, TracingConfigImpl, TracingConfigServer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Result of initializing Rapace tracing for a plugin.
pub struct PluginTracing<T: Transport> {
    /// RPC session used for communication with the host.
    pub session: Arc<RpcSession<T>>,
    /// Tracing config implementation used by the host to update filters.
    pub tracing_config: TracingConfigImpl,
}

impl<T: Transport> Clone for PluginTracing<T> {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            tracing_config: self.tracing_config.clone(),
        }
    }
}

/// Initialize tracing for a plugin using RapaceTracingLayer.
///
/// This:
/// - Attaches RapaceTracingLayer to the given session
/// - Installs a global tracing subscriber for the plugin
/// - Returns a PluginTracing struct containing the TracingConfigImpl
pub fn init_tracing<T>(session: Arc<RpcSession<T>>) -> PluginTracing<T>
where
    T: Transport + Send + Sync + 'static,
{
    let rt = tokio::runtime::Handle::current();
    let (tracing_layer, shared_filter) = RapaceTracingLayer::new(session.clone(), rt);
    let tracing_config = TracingConfigImpl::new(shared_filter);

    tracing_subscriber::registry().with(tracing_layer).init();

    PluginTracing {
        session,
        tracing_config,
    }
}

/// Service wrapper for TracingConfig, implementing ServiceDispatch.
struct TracingService(Arc<TracingConfigServer<TracingConfigImpl>>);

impl ServiceDispatch for TracingService {
    fn dispatch(
        &self,
        method_id: u32,
        payload: &[u8],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<rapace::Frame, rapace::RpcError>>
                + Send
                + 'static,
        >,
    > {
        let server = self.0.clone();
        let bytes = payload.to_vec();
        Box::pin(async move { server.dispatch(method_id, &bytes).await })
    }
}

/// Add a TracingConfig service to a DispatcherBuilder.
///
/// Plugins should call this so the host can push filter changes.
pub fn add_tracing_service(
    builder: DispatcherBuilder,
    tracing_config: TracingConfigImpl,
) -> DispatcherBuilder {
    let server = Arc::new(TracingConfigServer::new(tracing_config));
    builder.add_service(TracingService(server))
}
