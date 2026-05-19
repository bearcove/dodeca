//! Runtime helpers for dodeca cells.
//!
//! Cells are **cdylibs** loaded in-process by the `ddc` host via `dlopen`.
//! Each cell exports a single C symbol, `dodeca_cell_vtable_v1`, returning a
//! `vox_ffi` link vtable. The host (initiator) connects to it; the cell side
//! (acceptor) bootstraps a tokio runtime on attach and serves its `#[vox::service]`.
//!
//! This replaces the old roam-shm separate-process model (`run_cell!` +
//! `ShmGuestTransport`). The cell is now a thread inside `ddc`, so tracing
//! events land directly in the host subscriber — there is no more
//! tracing-over-RPC (`roam-tracing` is gone).
//!
//! Usage (callback-free cell):
//! ```ignore
//! use dodeca_cell_runtime::declare_cell;
//! use cell_image_proto::{ImageProcessorDispatcher, ImageProcessorImpl};
//! declare_cell!("image", |_host| ImageProcessorDispatcher::new(ImageProcessorImpl));
//! ```
//!
//! Usage (cell that calls back into the host):
//! ```ignore
//! declare_cell!("html", |host| {
//!     HtmlProcessorDispatcher::new(HtmlProcessorImpl::new(host))
//! });
//! ```
//! where `host: dodeca_cell_runtime::HostHandle` yields a `HostServiceClient`
//! via `host.client().await`.

pub use cell_host_proto::{self, HostServiceClient};
pub use tokio;
pub use tracing;
pub use ur_taking_me_with_you;
pub use vox;
pub use vox_ffi;

use std::sync::Arc;
use std::sync::OnceLock;
use vox::{ConnectionSettings, Parity, SessionHandle};

/// Connection settings used for every virtual connection opened over a
/// host<->cell FFI link. Mirrors the vox FFI reference settings.
pub fn connection_settings() -> ConnectionSettings {
    ConnectionSettings {
        parity: Parity::Odd,
        max_concurrent_requests: 64,
        initial_channel_credit: 16,
    }
}

/// Handle a cell uses to call back into the host's unified `HostService`.
///
/// The session is filled in by the generated bootstrap once the acceptor is
/// established. `client()` opens (and caches) a single `HostService` virtual
/// connection.
#[derive(Clone, Default)]
pub struct HostHandle {
    session: Arc<OnceLock<SessionHandle>>,
    client: Arc<OnceLock<HostServiceClient>>,
}

impl HostHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called by the generated bootstrap once the session is up.
    #[doc(hidden)]
    pub fn __set_session(&self, session: SessionHandle) {
        let _ = self.session.set(session);
    }

    /// Get a `HostServiceClient`, opening the virtual connection on first use.
    pub async fn client(&self) -> HostServiceClient {
        if let Some(c) = self.client.get() {
            return c.clone();
        }
        let session = self
            .session
            .get()
            .expect("HostHandle used before the cell session was established");
        let client: HostServiceClient = session
            .open(connection_settings())
            .await
            .expect("open HostService virtual connection");
        let _ = self.client.set(client.clone());
        client
    }
}

/// Internal: spawn the cell runtime thread once the host has attached.
///
/// Mirrors `vox/rust/subject-rust/src/ffi.rs`: a dedicated thread owns a
/// multi-thread tokio runtime, connects the (already-attached) endpoint to the
/// host peer, establishes the vox acceptor with the user dispatcher, then
/// parks until the host disconnects.
#[doc(hidden)]
pub fn __bootstrap<D, F>(
    endpoint: &'static vox_ffi::Endpoint,
    peer: *const vox_ffi::vox_link_vtable,
    cell_name: &'static str,
    host: HostHandle,
    make_dispatcher: F,
) where
    D: vox::ConnectionAcceptor + Send + 'static,
    F: FnOnce(HostHandle) -> D + Send + 'static,
{
    if peer.is_null() {
        tracing::error!(cell = cell_name, "cell attach: null host peer");
        return;
    }
    let peer = peer as usize;
    std::thread::Builder::new()
        .name(format!("cell-{cell_name}"))
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(cell = cell_name, error = %e, "cell: tokio runtime build failed");
                    return;
                }
            };
            runtime.block_on(async move {
                let peer = peer as *const vox_ffi::vox_link_vtable;
                let peer = match unsafe { vox_ffi::vox_link_vtable::validate_ptr(peer) } {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(cell = cell_name, error = %e, "cell: invalid host vtable");
                        return;
                    }
                };
                let link = match endpoint.connect(peer) {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!(cell = cell_name, error = %e, "cell: endpoint.connect failed");
                        return;
                    }
                };
                let dispatcher = make_dispatcher(host.clone());
                let root = match vox::acceptor_on(link)
                    .observer(vox::TracingObserver::new())
                    .on_connection(dispatcher)
                    .establish::<vox::NoopClient>()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(cell = cell_name, error = %e, "cell: acceptor establish failed");
                        return;
                    }
                };
                if let Some(session) = root.session.clone() {
                    host.__set_session(session);
                }
                tracing::debug!(cell = cell_name, "cell established, serving");
                root.caller.closed().await;
                tracing::debug!(cell = cell_name, "cell: host disconnected, shutting down");
                if let Some(session) = root.session.as_ref() {
                    let _ = session.shutdown();
                }
            });
        })
        .expect("spawn cell runtime thread");
}

/// Declare a dodeca cell cdylib.
///
/// Generates the `vox_ffi` endpoint, the exported C symbol
/// `dodeca_cell_vtable_v1`, and the attach->bootstrap glue. Invoke once at the
/// crate root of a cell whose `Cargo.toml` sets `crate-type = ["cdylib"]`.
#[macro_export]
macro_rules! declare_cell {
    ($cell_name:expr, | $host:ident | $make_dispatcher:expr) => {
        mod __dodeca_cell_ffi {
            use super::*;

            fn endpoint_vtable() -> &'static $crate::vox_ffi::vox_link_vtable {
                &ENDPOINT_VTABLE
            }

            pub(super) static ENDPOINT: $crate::vox_ffi::Endpoint =
                $crate::vox_ffi::Endpoint::new(endpoint_vtable);

            unsafe extern "C" fn endpoint_send(buf: *const u8, len: usize) {
                unsafe { $crate::vox_ffi::__endpoint_send(&ENDPOINT, buf, len) }
            }

            unsafe extern "C" fn endpoint_free(buf: *const u8) {
                unsafe { $crate::vox_ffi::__endpoint_free(&ENDPOINT, buf) }
            }

            unsafe extern "C" fn endpoint_attach(
                peer: *const $crate::vox_ffi::vox_link_vtable,
            ) -> $crate::vox_ffi::vox_status_t {
                let status = unsafe { $crate::vox_ffi::__endpoint_attach(&ENDPOINT, peer) };
                if status == $crate::vox_ffi::VOX_STATUS_OK {
                    let host = $crate::HostHandle::new();
                    $crate::__bootstrap(&ENDPOINT, peer, $cell_name, host.clone(), move |$host| {
                        $make_dispatcher
                    });
                }
                status
            }

            pub(super) static ENDPOINT_VTABLE: $crate::vox_ffi::vox_link_vtable =
                $crate::vox_ffi::vox_link_vtable::new(
                    endpoint_send,
                    endpoint_free,
                    endpoint_attach,
                );
        }

        /// Exported C entry point. The `ddc` host `dlsym`s this from the cell
        /// cdylib to obtain the link vtable.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn dodeca_cell_vtable_v1() -> *const $crate::vox_ffi::vox_link_vtable
        {
            &__dodeca_cell_ffi::ENDPOINT_VTABLE as *const _
        }
    };
}
