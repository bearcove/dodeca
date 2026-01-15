//! Runtime helpers for dodeca cells.
//!
//! Provides the `run_cell!` macro that handles all the boilerplate for connecting
//! to the host and signaling readiness.

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

/// Run a cell with the given name and dispatcher factory.
///
/// The dispatcher factory receives an `Arc<OnceLock<ConnectionHandle>>` that can be used
/// to create clients for calling back to the host. Cells that don't need callbacks
/// can ignore this parameter.
///
/// # Examples
///
/// Cell that doesn't need callbacks:
/// ```ignore
/// use dodeca_cell_runtime::run_cell;
/// use cell_image_proto::{ImageProcessorDispatcher, ImageProcessorImpl};
///
/// fn main() {
///     run_cell!("image", |_handle| {
///         ImageProcessorDispatcher::new(ImageProcessorImpl)
///     });
/// }
/// ```
///
/// Cell with callbacks to host:
/// ```ignore
/// use dodeca_cell_runtime::run_cell;
///
/// fn main() {
///     run_cell!("html", |handle| {
///         let processor = HtmlProcessorImpl::new(handle);
///         HtmlProcessorDispatcher::new(processor)
///     });
/// }
/// ```
#[macro_export]
macro_rules! run_cell {
    ($cell_name:expr, |$handle:ident| $make_dispatcher:expr) => {{
        use tracing_subscriber::prelude::*;
        use $crate::{
            CellTracingDispatcher, ConnectionHandle, HostServiceClient, ReadyMsg, RoutedDispatcher,
            ShmGuest, ShmGuestTransport, SpawnArgs, establish_guest, init_cell_tracing, tokio,
            tracing, tracing_subscriber, ur_taking_me_with_you,
        };

        // Ensure this process dies when the parent dies (required for macOS pipe-based approach)
        ur_taking_me_with_you::die_with_parent();

        async fn __run_cell_async() -> Result<(), Box<dyn std::error::Error>> {
            let args = SpawnArgs::from_env()?;
            let peer_id = args.peer_id;
            let transport = ShmGuestTransport::from_spawn_args(args)?;

            // Initialize cell-side tracing
            // Check TRACING_PASSTHROUGH env var - if set, log to stderr instead of via RPC
            let use_passthrough = std::env::var("TRACING_PASSTHROUGH").is_ok();

            let tracing_service = if use_passthrough {
                // Passthrough mode: log directly to stderr
                tracing_subscriber::fmt()
                    .with_writer(std::io::stderr)
                    .with_ansi(false)
                    .with_target(true)
                    .init();

                // Return a dummy service (won't be used)
                let (_layer, service) = init_cell_tracing(1);
                service
            } else {
                // Normal mode: use roam RPC for tracing
                let (tracing_layer, tracing_service) = init_cell_tracing(1024);
                tracing_subscriber::registry().with(tracing_layer).init();
                tracing_service
            };

            // Let user code create the dispatcher with access to handle
            // We use an Arc<OnceLock> pattern
            let handle_cell: std::sync::Arc<std::sync::OnceLock<ConnectionHandle>> =
                std::sync::Arc::new(std::sync::OnceLock::new());

            let $handle = handle_cell.clone();
            let user_dispatcher = $make_dispatcher;

            // Combine user's dispatcher with tracing dispatcher using RoutedDispatcher
            // RoutedDispatcher routes primary.method_ids() to primary, rest to fallback.
            let tracing_dispatcher = CellTracingDispatcher::new(tracing_service);
            let combined_dispatcher = RoutedDispatcher::new(
                tracing_dispatcher, // primary: handles tracing methods
                user_dispatcher,    // fallback: handles all cell-specific methods
            );

            let (handle, driver) = establish_guest(transport, combined_dispatcher);

            // Store the real handle
            let _ = handle_cell.set(handle.clone());

            // Spawn driver in background so it can process the ready() RPC
            let driver_handle = tokio::spawn(async move {
                if let Err(e) = driver.run().await {
                    eprintln!("Driver error: {:?}", e);
                    std::process::exit(1);
                }
            });

            // Signal readiness to host
            let host = HostServiceClient::new(handle);
            tracing::debug!("About to call host.ready() for cell {}", $cell_name);
            host.ready(ReadyMsg {
                peer_id: peer_id.get() as u16,
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
