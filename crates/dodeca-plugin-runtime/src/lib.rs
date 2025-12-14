//! Dodeca plugin runtime utilities
//!
//! Provides macros and utilities to simplify plugin development.

use std::path::PathBuf;
use std::sync::Arc;

// Re-export dependencies for macro use
pub use color_eyre;
pub use rapace;
pub use rapace_plugin;

use color_eyre::Result;
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};
use rapace_tracing::{RapaceTracingLayer, TracingConfigImpl, TracingConfigServer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Result of initializing Rapace tracing for a plugin.
pub struct PluginTracing<T: rapace::Transport> {
    /// RPC session used for communication with the host.
    pub session: Arc<RpcSession<T>>,
    /// Tracing config implementation used by the host to update filters.
    pub tracing_config: TracingConfigImpl,
}

impl<T: rapace::Transport> Clone for PluginTracing<T> {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            tracing_config: self.tracing_config.clone(),
        }
    }
}

/// Initialize tracing for a plugin using RapaceTracingLayer.
pub fn init_tracing<T>(session: Arc<RpcSession<T>>) -> PluginTracing<T>
where
    T: rapace::Transport + Send + Sync + 'static,
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
pub fn add_tracing_service(
    builder: DispatcherBuilder,
    tracing_config: TracingConfigImpl,
) -> DispatcherBuilder {
    let server = Arc::new(TracingConfigServer::new(tracing_config));
    builder.add_service(TracingService(server))
}

// SHM configuration is now read from the file header automatically.
// No need for plugins to specify config - they just use open_file_auto().

/// Run a plugin service with minimal boilerplate
pub async fn run_plugin_service<S>(service: S) -> Result<()>
where
    S: ServiceDispatch + Send + Sync + 'static,
{
    let args = parse_args()?;
    let plugin_name = plugin_name_from_shm_path(&args.shm_path);
    let transport = create_shm_transport(&args).await?;
    let session = Arc::new(RpcSession::with_channel_start(transport, 2));

    let PluginTracing { tracing_config, .. } = init_tracing(session.clone());
    tracing::info!(plugin = %plugin_name, "Connected to host via SHM");

    let dispatcher = DispatcherBuilder::new();
    let dispatcher = add_tracing_service(dispatcher, tracing_config);
    let dispatcher = dispatcher.add_service(service);
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    tracing::info!(plugin = %plugin_name, "Plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(plugin = %plugin_name, error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}

/// Extract plugin name from SHM path (e.g., "/tmp/dodeca-mod-sass-12345.shm" -> "dodeca-mod-sass")
fn plugin_name_from_shm_path(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| {
            // Remove the PID suffix (e.g., "dodeca-mod-sass-12345" -> "dodeca-mod-sass")
            if let Some(idx) = s.rfind('-') {
                if s[idx + 1..].chars().all(|c| c.is_ascii_digit()) {
                    return s[..idx].to_string();
                }
            }
            s.to_string()
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// CLI arguments for plugins
#[derive(Debug)]
pub struct Args {
    pub shm_path: PathBuf,
}

pub fn parse_args() -> Result<Args> {
    let mut shm_path = None;

    for arg in std::env::args().skip(1) {
        if let Some(value) = arg.strip_prefix("--shm-path=") {
            shm_path = Some(PathBuf::from(value));
        }
    }

    Ok(Args {
        shm_path: shm_path.ok_or_else(|| color_eyre::eyre::eyre!("--shm-path required"))?,
    })
}

pub async fn create_shm_transport(args: &Args) -> Result<Arc<ShmTransport>> {
    let plugin_name = plugin_name_from_shm_path(&args.shm_path);

    // Wait for the host to create the SHM file
    for i in 0..50 {
        if args.shm_path.exists() {
            break;
        }
        if i == 49 {
            return Err(color_eyre::eyre::eyre!(
                "SHM file not created by host: {}",
                args.shm_path.display()
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Open the SHM session (plugin side) - config is read from the file header
    let shm_session = ShmSession::open_file_auto(&args.shm_path)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to open SHM: {:?}", e))?;

    Ok(Arc::new(ShmTransport::with_name(shm_session, plugin_name)))
}

/// Macro to create a plugin service wrapper
#[macro_export]
macro_rules! plugin_service {
    ($server_type:ty, $impl_type:ty) => {
        struct PluginService(std::sync::Arc<$server_type>);

        impl $crate::rapace_plugin::ServiceDispatch for PluginService {
            fn dispatch(
                &self,
                method_id: u32,
                payload: &[u8],
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = std::result::Result<$crate::rapace::Frame, $crate::rapace::RpcError>>
                        + Send
                        + 'static,
                >,
            > {
                let server = self.0.clone();
                let bytes = payload.to_vec();
                Box::pin(async move { server.dispatch(method_id, &bytes).await })
            }
        }

        impl From<$impl_type> for PluginService {
            fn from(impl_val: $impl_type) -> Self {
                Self(std::sync::Arc::new(<$server_type>::new(impl_val)))
            }
        }
    };
}

/// Macro to run a plugin with minimal boilerplate
#[macro_export]
macro_rules! run_plugin {
    ($service_impl:expr) => {
        #[tokio::main(flavor = "current_thread")]
        async fn main() -> $crate::color_eyre::Result<()> {
            $crate::color_eyre::install()?;
            $crate::run_plugin_service(PluginService::from($service_impl)).await
        }
    };
}