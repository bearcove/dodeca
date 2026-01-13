//! Runtime helpers for dodeca cells.
//!
//! Provides macros that handle all the boilerplate for connecting to the host
//! and signaling readiness:
//!
//! - `run_cell!` - Simple cells that don't need the connection handle
//! - `run_cell_with_handle!` - Cells that need the handle for callbacks

pub use cell_lifecycle_proto::{CellLifecycleClient, ReadyMsg};
pub use roam::session::{ConnectionHandle, ServiceDispatcher};
pub use roam_shm::driver::establish_guest;
pub use roam_shm::guest::ShmGuest;
pub use roam_shm::spawn::SpawnArgs;
pub use roam_shm::transport::ShmGuestTransport;
pub use tokio;

/// Run a cell with the given name and dispatcher.
///
/// This macro handles all the boilerplate:
/// - Parsing spawn args from environment
/// - Attaching to the shared memory segment
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
        use $crate::{
            CellLifecycleClient, ReadyMsg, ShmGuest, ShmGuestTransport, SpawnArgs, establish_guest,
            tokio,
        };

        async fn __run_cell_async() -> Result<(), Box<dyn std::error::Error>> {
            let args = SpawnArgs::from_env()?;
            let guest = ShmGuest::attach_with_ticket(&args)?;
            let transport = ShmGuestTransport::new(guest);
            let dispatcher = $dispatcher;
            let (handle, driver) = establish_guest(transport, dispatcher);

            // Spawn driver in background so it can process the ready() RPC
            let driver_handle = tokio::spawn(async move {
                if let Err(e) = driver.run().await {
                    eprintln!("Driver error: {:?}", e);
                }
            });

            // Signal readiness to host
            let lifecycle = CellLifecycleClient::new(handle);
            lifecycle
                .ready(ReadyMsg {
                    peer_id: args.peer_id.get() as u16,
                    cell_name: $cell_name.to_string(),
                    pid: Some(std::process::id()),
                    version: None,
                    features: vec![],
                })
                .await?;

            // Wait for driver to complete (it runs until connection closes)
            let _ = driver_handle.await;
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
        use $crate::{
            CellLifecycleClient, ConnectionHandle, ReadyMsg, ShmGuest, ShmGuestTransport,
            SpawnArgs, establish_guest, tokio,
        };

        async fn __run_cell_async() -> Result<(), Box<dyn std::error::Error>> {
            let $args = SpawnArgs::from_env()?;
            let guest = ShmGuest::attach_with_ticket(&$args)?;
            let transport = ShmGuestTransport::new(guest);

            // Let user code create the dispatcher with access to handle
            // We use a dummy dispatcher first to get the handle, then rebuild
            // Actually, we need to create the dispatcher before establish_guest
            // So we use an OnceLock pattern
            let handle_cell: std::sync::Arc<std::sync::OnceLock<ConnectionHandle>> =
                std::sync::Arc::new(std::sync::OnceLock::new());

            let $handle = handle_cell.clone();
            let dispatcher = $make_dispatcher;
            let (handle, driver) = establish_guest(transport, dispatcher);

            // Store the real handle
            let _ = handle_cell.set(handle.clone());

            // Signal readiness to host
            let lifecycle = CellLifecycleClient::new(handle);
            lifecycle
                .ready(ReadyMsg {
                    peer_id: $args.peer_id.get() as u16,
                    cell_name: $cell_name.to_string(),
                    pid: Some(std::process::id()),
                    version: None,
                    features: vec![],
                })
                .await?;

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
