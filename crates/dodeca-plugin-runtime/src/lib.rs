//! Dodeca plugin runtime utilities
//!
//! Provides macros and utilities to simplify plugin development.
//!
//! # Hub Architecture
//!
//! Plugins connect to the host via a shared SHM "hub" file. Each plugin gets:
//! - A peer_id assigned by the host
//! - A socketpair doorbell for cross-process wakeup
//! - Its own ring pair within the shared SHM
//!
//! Command-line arguments:
//! - `--hub-path=<path>` - Path to the hub SHM file
//! - `--peer-id=<id>` - Peer ID assigned by the host
//! - `--doorbell-fd=<fd>` - File descriptor for the doorbell socketpair

use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::sync::Arc;

// Re-export dependencies for macro use
pub use color_eyre;
pub use rapace;
pub use rapace_plugin;

use color_eyre::Result;
use rapace::RpcSession;
use rapace::transport::shm::{Doorbell, HubPeer, HubPeerTransport};
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

/// Run a plugin service with minimal boilerplate.
///
/// Connects to the host via the shared hub SHM and runs the RPC session.
pub async fn run_plugin_service<S>(service: S) -> Result<()>
where
    S: ServiceDispatch + Send + Sync + 'static,
{
    let args = parse_args()?;
    let plugin_name = plugin_name_from_hub_path(&args.hub_path);
    let transport = create_hub_transport(&args).await?;
    let session = Arc::new(RpcSession::with_channel_start(transport, 2));

    let PluginTracing { tracing_config, .. } = init_tracing(session.clone());
    tracing::info!(
        plugin = %plugin_name,
        peer_id = args.peer_id,
        "Connected to host via hub SHM"
    );

    let dispatcher = DispatcherBuilder::new();
    let dispatcher = add_tracing_service(dispatcher, tracing_config);
    let dispatcher = dispatcher.add_service(service);
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    tracing::info!(plugin = %plugin_name, peer_id = args.peer_id, "Plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(plugin = %plugin_name, peer_id = args.peer_id, error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}

/// Extract plugin name from hub path (e.g., "/tmp/dodeca-hub-12345.shm" -> "plugin")
fn plugin_name_from_hub_path(path: &std::path::Path) -> String {
    // The hub path doesn't contain the plugin name, so we use the executable name
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// CLI arguments for plugins (hub architecture).
#[derive(Debug)]
pub struct Args {
    /// Path to the hub SHM file.
    pub hub_path: PathBuf,
    /// Peer ID assigned by the host.
    pub peer_id: u16,
    /// File descriptor for the doorbell socketpair.
    pub doorbell_fd: RawFd,
}

/// Parse command-line arguments for hub-based plugins.
pub fn parse_args() -> Result<Args> {
    let mut hub_path = None;
    let mut peer_id = None;
    let mut doorbell_fd = None;

    for arg in std::env::args().skip(1) {
        if let Some(value) = arg.strip_prefix("--hub-path=") {
            hub_path = Some(PathBuf::from(value));
        } else if let Some(value) = arg.strip_prefix("--peer-id=") {
            peer_id = Some(value.parse::<u16>().map_err(|e| {
                color_eyre::eyre::eyre!("invalid --peer-id: {}", e)
            })?);
        } else if let Some(value) = arg.strip_prefix("--doorbell-fd=") {
            doorbell_fd = Some(value.parse::<RawFd>().map_err(|e| {
                color_eyre::eyre::eyre!("invalid --doorbell-fd: {}", e)
            })?);
        }
    }

    Ok(Args {
        hub_path: hub_path.ok_or_else(|| color_eyre::eyre::eyre!("--hub-path required"))?,
        peer_id: peer_id.ok_or_else(|| color_eyre::eyre::eyre!("--peer-id required"))?,
        doorbell_fd: doorbell_fd.ok_or_else(|| color_eyre::eyre::eyre!("--doorbell-fd required"))?,
    })
}

/// Create a hub transport for the plugin.
pub async fn create_hub_transport(args: &Args) -> Result<Arc<HubPeerTransport>> {
    let plugin_name = plugin_name_from_hub_path(&args.hub_path);

    // Wait for the hub SHM file to exist
    for i in 0..50 {
        if args.hub_path.exists() {
            break;
        }
        if i == 49 {
            return Err(color_eyre::eyre::eyre!(
                "Hub SHM file not created by host: {}",
                args.hub_path.display()
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Open the hub as a peer
    let peer = HubPeer::open(&args.hub_path, args.peer_id)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to open hub SHM: {:?}", e))?;

    // Register this peer in the hub
    peer.register();

    // Create doorbell from inherited file descriptor
    let doorbell = Doorbell::from_raw_fd(args.doorbell_fd)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to create doorbell: {:?}", e))?;

    Ok(Arc::new(HubPeerTransport::new(Arc::new(peer), doorbell, plugin_name)))
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