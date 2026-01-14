//! Runtime helpers for dodeca cells.
//!
//! Provides macros that handle all the boilerplate for connecting to the host
//! and signaling readiness:
//!
//! - `run_cell!` - Simple cells that don't need the connection handle
//! - `run_cell_with_handle!` - Cells that need the handle for callbacks

pub use cell_host_proto::{HostServiceClient, ReadyMsg};
pub use roam::session::{ConnectionHandle, RoutedDispatcher, ServiceDispatcher};
pub use roam_shm::driver::establish_guest;
pub use roam_shm::guest::ShmGuest;
pub use roam_shm::spawn::SpawnArgs;
pub use roam_shm::transport::ShmGuestTransport;
pub use roam_tracing::{CellTracingDispatcher, CellTracingLayer, init_cell_tracing};
pub use tokio;
pub use tracing;
pub use tracing_subscriber;
pub use ur_taking_me_with_you;

/// Run a cell with the given name and dispatcher.
///
/// This macro handles all the boilerplate:
/// - Parsing spawn args from environment
/// - Attaching to the shared memory segment
/// - Setting up tracing to forward to host
/// - Establishing the guest connection
/// - Signaling readiness to the host
/// - Running the driver loop
///
/// # Example
///
/// ```ignore
/// use dodeca_cell_runtime::run_cell;
/// use cell_image_proto::{ImageProcessorDispatcher, ImageProcessorImpl};
///
/// fn main() {
///     run_cell!("image", ImageProcessorDispatcher::new(ImageProcessorImpl));
/// }
/// ```
#[macro_export]
macro_rules! run_cell {
    ($cell_name:expr, $dispatcher:expr) => {{
        use tracing_subscriber::prelude::*;
        use $crate::{
            CellTracingDispatcher, HostServiceClient, ReadyMsg, RoutedDispatcher, ShmGuest,
            ShmGuestTransport, SpawnArgs, establish_guest, init_cell_tracing, tokio, tracing,
            tracing_subscriber, ur_taking_me_with_you,
        };

        // Ensure this process dies when the parent dies (required for macOS pipe-based approach)
        ur_taking_me_with_you::die_with_parent();

        async fn __run_cell_async() -> Result<(), Box<dyn std::error::Error>> {
            let args = SpawnArgs::from_env()?;
            // Create transport with guest + doorbell for host notifications
            let transport = ShmGuestTransport::from_spawn_args(&args)?;

            // Initialize cell-side tracing
            let (tracing_layer, tracing_service) = init_cell_tracing(1024);

            // Set up tracing subscriber with the layer
            tracing_subscriber::registry().with(tracing_layer).init();

            // Combine user's dispatcher with tracing dispatcher using RoutedDispatcher
            // RoutedDispatcher routes primary.method_ids() to primary, rest to fallback.
            let user_dispatcher = $dispatcher;
            let tracing_dispatcher = CellTracingDispatcher::new(tracing_service);
            let combined_dispatcher = RoutedDispatcher::new(
                tracing_dispatcher, // primary: handles tracing methods
                user_dispatcher,    // fallback: handles all cell-specific methods
            );

            let (handle, driver) = establish_guest(transport, combined_dispatcher);

            // Spawn driver in background so it can process the ready() RPC
            let driver_handle = tokio::spawn(async move {
                if let Err(e) = driver.run().await {
                    tracing::error!("Driver error: {:?}", e);
                }
            });

            // Signal readiness to host
            let host = HostServiceClient::new(handle);
            tracing::info!("About to call host.ready() for cell {}", $cell_name);
            host.ready(ReadyMsg {
                peer_id: args.peer_id.get() as u16,
                cell_name: $cell_name.to_string(),
                pid: Some(std::process::id()),
                version: None,
                features: vec![],
            })
            .await?;
            tracing::info!("host.ready() returned successfully for cell {}", $cell_name);

            // Wait for driver to complete (it runs until connection closes)
            if let Err(e) = driver_handle.await {
                eprintln!("[cell] driver task panicked: {e:?}");
            }
            Ok(())
        }

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
            .block_on(__run_cell_async())
    }};
}

/// Run a cell that needs access to the connection handle for callbacks.
///
/// This macro is for cells that need bidirectional RPC (cell calls back to host).
/// It provides the handle to an async closure before signaling readiness.
///
/// The closure receives:
/// - `handle: ConnectionHandle` - for creating clients to call back to host
/// - `args: &SpawnArgs` - for accessing peer_id etc.
///
/// The closure must return the dispatcher to use.
///
/// # Example
///
/// ```ignore
/// use dodeca_cell_runtime::run_cell_with_handle;
/// use std::sync::Arc;
///
/// fn main() {
///     run_cell_with_handle!("gingembre", |handle, _args| {
///         let ctx = Arc::new(CellContext { handle: handle.clone() });
///         let renderer = TemplateRendererImpl::new(ctx);
///         TemplateRendererDispatcher::new(renderer)
///     });
/// }
/// ```
#[macro_export]
macro_rules! run_cell_with_handle {
    ($cell_name:expr, |$handle:ident, $args:ident| $make_dispatcher:expr) => {{
        use tracing_subscriber::prelude::*;
        use $crate::{
            CellTracingDispatcher, ConnectionHandle, HostServiceClient, ReadyMsg, RoutedDispatcher,
            ShmGuest, ShmGuestTransport, SpawnArgs, establish_guest, init_cell_tracing, tokio,
            tracing, tracing_subscriber, ur_taking_me_with_you,
        };

        // Ensure this process dies when the parent dies (required for macOS pipe-based approach)
        ur_taking_me_with_you::die_with_parent();

        async fn __run_cell_async() -> Result<(), Box<dyn std::error::Error>> {
            let $args = SpawnArgs::from_env()?;
            // Create transport with guest + doorbell for host notifications
            let transport = ShmGuestTransport::from_spawn_args(&$args)?;

            // Initialize cell-side tracing
            let (tracing_layer, tracing_service) = init_cell_tracing(1024);

            // Set up tracing subscriber with the layer
            tracing_subscriber::registry().with(tracing_layer).init();

            // Let user code create the dispatcher with access to handle
            // We use an OnceLock pattern
            let handle_cell: std::sync::Arc<std::sync::OnceLock<ConnectionHandle>> =
                std::sync::Arc::new(std::sync::OnceLock::new());

            let $handle = handle_cell.clone();
            let user_dispatcher = $make_dispatcher;

            // Combine user's dispatcher with tracing dispatcher
            // RoutedDispatcher routes primary.method_ids() to primary, rest to fallback.
            let tracing_dispatcher = CellTracingDispatcher::new(tracing_service);
            let combined_dispatcher = RoutedDispatcher::new(
                tracing_dispatcher, // primary: handles tracing methods
                user_dispatcher,    // fallback: handles all cell-specific methods
            );

            let (handle, driver) = establish_guest(transport, combined_dispatcher);

            // Store the real handle
            let _ = handle_cell.set(handle.clone());

            // Signal readiness to host
            let host = HostServiceClient::new(handle);
            tracing::info!("About to call host.ready() for cell {}", $cell_name);
            host.ready(ReadyMsg {
                peer_id: $args.peer_id.get() as u16,
                cell_name: $cell_name.to_string(),
                pid: Some(std::process::id()),
                version: None,
                features: vec![],
            })
            .await?;
            tracing::info!("host.ready() returned successfully for cell {}", $cell_name);

            driver.run().await?;
            Ok(())
        }

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
            .block_on(__run_cell_async())
    }};
}
