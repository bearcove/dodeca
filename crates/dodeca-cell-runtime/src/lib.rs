//! Dodeca cell runtime utilities
//!
//! Provides macros and utilities to simplify cell development.
//!
//! # Hub Architecture
//!
//! Cells connect to the host via a shared SHM "hub" file. Each cell gets:
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
pub use rapace_cell;
pub use dodeca_debug;

use color_eyre::Result;
use rapace::RpcSession;
use rapace::transport::shm::{Doorbell, HubPeer, HubPeerTransport};
use rapace_cell::{DispatcherBuilder, ServiceDispatch};
use rapace_tracing::{RapaceTracingLayer, TracingConfigImpl, TracingConfigServer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Check if running in quiet mode (TUI active, suppress startup messages).
fn is_quiet_mode() -> bool {
    std::env::var("DODECA_QUIET").is_ok()
}

/// Print a debug message if not in quiet mode.
macro_rules! cell_debug {
    ($($arg:tt)*) => {
        if !$crate::is_quiet_mode() {
            eprintln!($($arg)*);
        }
    };
}

/// Result of initializing Rapace tracing for a cell.
pub struct CellTracing<T: rapace::Transport> {
    /// RPC session used for communication with the host.
    pub session: Arc<RpcSession<T>>,
    /// Tracing config implementation used by the host to update filters.
    pub tracing_config: TracingConfigImpl,
}

impl<T: rapace::Transport> Clone for CellTracing<T> {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            tracing_config: self.tracing_config.clone(),
        }
    }
}

/// Initialize tracing for a cell using RapaceTracingLayer.
pub fn init_tracing<T>(session: Arc<RpcSession<T>>) -> CellTracing<T>
where
    T: rapace::Transport + Send + Sync + 'static,
{
    let rt = tokio::runtime::Handle::current();
    let (tracing_layer, shared_filter) = RapaceTracingLayer::new(session.clone(), rt);
    let tracing_config = TracingConfigImpl::new(shared_filter);

    tracing_subscriber::registry().with(tracing_layer).init();

    CellTracing {
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

/// Run a cell service with minimal boilerplate.
///
/// Connects to the host via the shared hub SHM and runs the RPC session.
pub async fn run_cell_service<S>(service: S) -> Result<()>
where
    S: ServiceDispatch + Send + Sync + 'static,
{
    let args = parse_args()?;
    let cell_name = cell_name_from_hub_path(&args.hub_path);
    let transport = create_hub_transport(&args).await?;

    // Register SIGUSR1 diagnostic callback
    register_cell_diagnostics(cell_name.clone(), transport.clone());

    let session = Arc::new(RpcSession::with_channel_start(transport, 2));

    let CellTracing { tracing_config, .. } = init_tracing(session.clone());
    tracing::info!(
        cell = %cell_name,
        peer_id = args.peer_id,
        "Connected to host via hub SHM"
    );

    let dispatcher = DispatcherBuilder::new();
    let dispatcher = add_tracing_service(dispatcher, tracing_config);
    let dispatcher = dispatcher.add_service(service);
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    tracing::info!(cell = %cell_name, peer_id = args.peer_id, "Cell ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(cell = %cell_name, peer_id = args.peer_id, error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}

/// Register cell diagnostics for SIGUSR1.
fn register_cell_diagnostics(name: String, transport: Arc<HubPeerTransport>) {
    dodeca_debug::register_diagnostic(move || {
        let peer = transport.peer();
        let recv_ring = peer.recv_ring();
        let send_ring = peer.send_ring();
        let doorbell_bytes = transport.doorbell_pending_bytes();

        eprintln!(
            "\n--- Cell \"{}\" Transport Diagnostics ---",
            name
        );
        eprintln!(
            "  peer_id={}: recv_ring({}) send_ring({}) doorbell_pending={}",
            peer.peer_id(),
            recv_ring.ring_status(),
            send_ring.ring_status(),
            doorbell_bytes
        );
        eprintln!("--- End Cell Diagnostics ---\n");
    });
}

/// Extract cell name from hub path (e.g., "/tmp/dodeca-hub-12345.shm" -> "cell")
fn cell_name_from_hub_path(path: &std::path::Path) -> String {
    // The hub path doesn't contain the cell name, so we use the executable name
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// CLI arguments for cells (hub architecture).
#[derive(Debug)]
pub struct Args {
    /// Path to the hub SHM file.
    pub hub_path: PathBuf,
    /// Peer ID assigned by the host.
    pub peer_id: u16,
    /// File descriptor for the doorbell socketpair.
    pub doorbell_fd: RawFd,
}

/// Parse command-line arguments for hub-based cells.
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

/// Create a hub transport for the cell.
pub async fn create_hub_transport(args: &Args) -> Result<Arc<HubPeerTransport>> {
    let cell_name = cell_name_from_hub_path(&args.hub_path);

    cell_debug!(
        "[{}] create_hub_transport: peer_id={}, doorbell_fd={}, hub_path={}",
        cell_name, args.peer_id, args.doorbell_fd, args.hub_path.display()
    );

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

    // Check if the doorbell FD is valid before wrapping
    {
        let flags = unsafe { libc::fcntl(args.doorbell_fd, libc::F_GETFL) };
        if flags < 0 {
            let err = std::io::Error::last_os_error();
            eprintln!(
                "[{}] ERROR: doorbell_fd {} is invalid: {}",
                cell_name, args.doorbell_fd, err
            );
            return Err(color_eyre::eyre::eyre!(
                "Doorbell FD {} is invalid: {}",
                args.doorbell_fd,
                err
            ));
        }
        cell_debug!(
            "[{}] doorbell_fd {} is valid (flags=0x{:x})",
            cell_name, args.doorbell_fd, flags
        );
    }

    // Open the hub as a peer
    let peer = HubPeer::open(&args.hub_path, args.peer_id)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to open hub SHM: {:?}", e))?;

    cell_debug!("[{}] opened hub SHM as peer {}", cell_name, args.peer_id);

    // Register this peer in the hub
    peer.register();
    cell_debug!("[{}] registered as active peer", cell_name);

    // Create doorbell from inherited file descriptor
    let doorbell = Doorbell::from_raw_fd(args.doorbell_fd)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to create doorbell: {:?}", e))?;

    cell_debug!(
        "[{}] created doorbell from fd {}, wrapped in AsyncFd",
        cell_name, args.doorbell_fd
    );

    Ok(Arc::new(HubPeerTransport::new(Arc::new(peer), doorbell, cell_name)))
}

/// Macro to create a cell service wrapper
#[macro_export]
macro_rules! cell_service {
    ($server_type:ty, $impl_type:ty) => {
        struct CellService(std::sync::Arc<$server_type>);

        impl $crate::rapace_cell::ServiceDispatch for CellService {
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

        impl From<$impl_type> for CellService {
            fn from(impl_val: $impl_type) -> Self {
                Self(std::sync::Arc::new(<$server_type>::new(impl_val)))
            }
        }
    };
}

/// Macro to run a cell with minimal boilerplate
#[macro_export]
macro_rules! run_cell {
    ($service_impl:expr) => {
        #[tokio::main(flavor = "current_thread")]
        async fn main() -> $crate::color_eyre::Result<()> {
            // Install panic hook that clearly identifies the cell
            let cell_name = env!("CARGO_BIN_NAME");
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                eprintln!("\n[CELL PANIC] {} panicked!", cell_name);
                eprintln!("[CELL PANIC] Location: {:?}", info.location());
                if let Some(msg) = info.payload().downcast_ref::<&str>() {
                    eprintln!("[CELL PANIC] Message: {}", msg);
                } else if let Some(msg) = info.payload().downcast_ref::<String>() {
                    eprintln!("[CELL PANIC] Message: {}", msg);
                }
                // Also run the default hook for full backtrace
                default_hook(info);
            }));

            // Install SIGUSR1 handler for debugging (dumps stack traces)
            $crate::dodeca_debug::install_sigusr1_handler(cell_name);
            $crate::color_eyre::install()?;
            $crate::run_cell_service(CellService::from($service_impl)).await
        }
    };
}
