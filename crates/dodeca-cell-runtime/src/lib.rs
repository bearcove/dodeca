//! Runtime helpers for dodeca cells.
//!
//! Provides the `run_cell!` macro that handles all the boilerplate for connecting
//! to the host and signaling readiness.
//!
//! Enable the `cell-debug` feature for verbose startup logging.

pub use cell_host_proto::{HostServiceClient, ReadyMsg};
pub use roam::session::{ConnectionHandle, RoutedDispatcher, ServiceDispatcher};
pub use roam_shm::driver::establish_guest;
pub use roam_shm::guest::ShmGuest;
pub use roam_shm::spawn::SpawnArgs;
pub use roam_shm::transport::ShmGuestTransport;
pub use roam_tracing::{CellTracingDispatcher, CellTracingLayer, CellTracingService, init_cell_tracing};
pub use tokio;
pub use tracing;
pub use tracing_subscriber;
pub use ur_taking_me_with_you;

/// Debug print macro that only prints when cell-debug feature is enabled
#[macro_export]
#[cfg(feature = "cell-debug")]
macro_rules! cell_debug {
    ($($arg:tt)*) => {
        eprintln!($($arg)*)
    };
}

/// Debug print macro that compiles to nothing when cell-debug feature is disabled
#[macro_export]
#[cfg(not(feature = "cell-debug"))]
macro_rules! cell_debug {
    ($($arg:tt)*) => {};
}

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

        $crate::cell_debug!("[cell-{}] starting (pid={})", $cell_name, std::process::id());

        // Ensure this process dies when the parent dies (required for macOS pipe-based approach)
        ur_taking_me_with_you::die_with_parent();

        $crate::cell_debug!("[cell-{}] die_with_parent completed", $cell_name);

        async fn __run_cell_async() -> Result<(), Box<dyn std::error::Error>> {
            $crate::cell_debug!("[cell] async fn starting");
            let args = SpawnArgs::from_env()?;
            $crate::cell_debug!("[cell] parsed args: peer_id={}", args.peer_id.get());
            let peer_id = args.peer_id;
            let transport = ShmGuestTransport::from_spawn_args(args)?;
            $crate::cell_debug!("[cell] transport created");

            // Initialize cell-side tracing
            // Check TRACING_PASSTHROUGH env var - if set, log to stderr instead of via RPC
            let use_passthrough = std::env::var("TRACING_PASSTHROUGH").is_ok();
            $crate::cell_debug!("[cell] use_passthrough={}", use_passthrough);

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
            $crate::cell_debug!("[cell] tracing initialized");

            // Let user code create the dispatcher with access to handle
            // We use an Arc<OnceLock> pattern
            let handle_cell: std::sync::Arc<std::sync::OnceLock<ConnectionHandle>> =
                std::sync::Arc::new(std::sync::OnceLock::new());

            let $handle = handle_cell.clone();
            $crate::cell_debug!("[cell] creating user dispatcher");
            let user_dispatcher = $make_dispatcher;
            $crate::cell_debug!("[cell] user dispatcher created");

            // Clone tracing_service before moving into dispatcher (for spawn_drain later)
            let tracing_service_for_drain = tracing_service.clone();

            // Combine user's dispatcher with tracing dispatcher using RoutedDispatcher
            // RoutedDispatcher routes primary.method_ids() to primary, rest to fallback.
            let tracing_dispatcher = CellTracingDispatcher::new(tracing_service);
            let combined_dispatcher = RoutedDispatcher::new(
                tracing_dispatcher, // primary: handles tracing methods
                user_dispatcher,    // fallback: handles all cell-specific methods
            );
            $crate::cell_debug!("[cell] calling establish_guest");

            let (handle, driver) = establish_guest(transport, combined_dispatcher);
            $crate::cell_debug!("[cell] establish_guest returned");

            // Store the real handle
            let _ = handle_cell.set(handle.clone());
            $crate::cell_debug!("[cell] handle stored");

            // Start the tracing drain task (sends buffered records to host via RPC)
            if !use_passthrough {
                tracing_service_for_drain.spawn_drain(handle.clone());
                $crate::cell_debug!("[cell] tracing drain task spawned");
            }

            // Spawn driver in background so it can process the ready() RPC
            $crate::cell_debug!("[cell] spawning driver task");
            let driver_handle = tokio::spawn(async move {
                $crate::cell_debug!("[cell] driver task starting");
                if let Err(e) = driver.run().await {
                    eprintln!("Driver error: {:?}", e);
                    std::process::exit(1);
                }
                $crate::cell_debug!("[cell] driver task exited cleanly");
            });
            $crate::cell_debug!("[cell] driver task spawned");

            // Signal readiness to host
            let host = HostServiceClient::new(handle);
            $crate::cell_debug!("[cell] calling host.ready()");
            host.ready(ReadyMsg {
                peer_id: peer_id.get() as u16,
                cell_name: $cell_name.to_string(),
                pid: Some(std::process::id()),
                version: None,
                features: vec![],
            })
            .await?;
            $crate::cell_debug!("[cell] host.ready() returned successfully");

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
