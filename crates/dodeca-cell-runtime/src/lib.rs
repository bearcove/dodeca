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
use tokio::sync::watch;
use vox::{ConnectionHandle, ConnectionSettings, Metadata, Parity, SessionHandle};

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
#[derive(Clone)]
pub struct HostHandle {
    session: Arc<OnceLock<SessionHandle>>,
    session_tx: Arc<watch::Sender<Option<SessionHandle>>>,
    client: Arc<OnceLock<HostServiceClient>>,
}

impl HostHandle {
    pub fn new() -> Self {
        let (session_tx, _) = watch::channel(None);
        Self {
            session: Arc::new(OnceLock::new()),
            session_tx: Arc::new(session_tx),
            client: Arc::new(OnceLock::new()),
        }
    }

    /// Called by the generated bootstrap once the session is up.
    #[doc(hidden)]
    pub fn __set_session(&self, session: SessionHandle) {
        if self.session.set(session.clone()).is_ok() {
            let _ = self.session_tx.send(Some(session));
        }
    }

    async fn session(&self) -> SessionHandle {
        let mut session_rx = self.session_tx.subscribe();
        loop {
            if let Some(session) = self.session.get() {
                return session.clone();
            }
            session_rx
                .changed()
                .await
                .expect("HostHandle session sender dropped");
        }
    }

    /// Get a `HostServiceClient`, opening the virtual connection on first use.
    pub async fn client(&self) -> HostServiceClient {
        if let Some(c) = self.client.get() {
            return c.clone();
        }
        let session = self.session().await;
        let client: HostServiceClient = session
            .open(connection_settings())
            .await
            .expect("open HostService virtual connection");
        let _ = self.client.set(client.clone());
        client
    }

    /// Open a raw virtual connection back to the host.
    pub async fn open_connection(&self, metadata: Metadata<'static>) -> ConnectionHandle {
        let session = self.session().await;
        session
            .open_connection(connection_settings(), metadata)
            .await
            .expect("open host virtual connection")
    }
}

impl Default for HostHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Recorded death info for a cell whose runtime thread (or one of its tokio
/// workers) panicked. Read by the host's `cell_loader` when it sees the
/// matching session close.
#[derive(Clone, Debug)]
pub struct CellDeath {
    pub thread_name: String,
    pub location: String,
    pub message: String,
}

impl std::fmt::Display for CellDeath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "thread '{}' panicked at {}: {}",
            self.thread_name, self.location, self.message
        )
    }
}

static CELL_DEATHS: std::sync::OnceLock<dashmap::DashMap<String, CellDeath>> =
    std::sync::OnceLock::new();

fn cell_deaths() -> &'static dashmap::DashMap<String, CellDeath> {
    CELL_DEATHS.get_or_init(dashmap::DashMap::new)
}

/// Take the recorded death reason for a cell, if any.
pub fn take_cell_death(cell_name: &str) -> Option<CellDeath> {
    cell_deaths().remove(cell_name).map(|(_, v)| v)
}

/// Peek at the recorded death reason for a cell, if any.
pub fn cell_death(cell_name: &str) -> Option<CellDeath> {
    cell_deaths().get(cell_name).map(|r| r.value().clone())
}

/// Install (once) a process-wide panic hook that records cell-thread panics
/// in `CELL_DEATHS`. We name the cell's std thread `cell-<name>` and the
/// tokio workers `cell-<name>-rt-…`; both share the `cell-<name>` prefix.
/// Recording is non-fatal: the hook does NOT abort. The panic still unwinds
/// the cell's runtime, the runtime drops, the vox-ffi link drops, and the
/// host's session sees a clean close — at which point the host can pair the
/// closure with the recorded reason via [`take_cell_death`].
fn install_cell_panic_recorder() {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread_name = std::thread::current()
                .name()
                .unwrap_or("<unnamed>")
                .to_string();
            if let Some(rest) = thread_name.strip_prefix("cell-") {
                // `cell-<name>` (the supervisor thread) or
                // `cell-<name>-rt-worker-…` (a tokio worker).
                let cell_name = rest
                    .split_once("-rt")
                    .map(|(n, _)| n)
                    .unwrap_or(rest)
                    .to_string();
                let location = info
                    .location()
                    .map(|l| format!("{}:{}", l.file(), l.line()))
                    .unwrap_or_else(|| "<unknown location>".into());
                let message = info
                    .payload()
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "<non-string panic payload>".into());
                let death = CellDeath {
                    thread_name: thread_name.clone(),
                    location,
                    message,
                };
                tracing::error!(cell = %cell_name, %death, "cell thread panicked");
                // First-writer-wins so the root cause survives any cascading
                // panics on other workers of the same cell.
                cell_deaths().entry(cell_name).or_insert(death);
            }
            prev(info);
        }));
    });
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
    // We own the cell thread — capture its panics. Cell-thread panics get
    // recorded (cell name from thread name) and then unwind normally; the
    // tokio runtime drops, the vox-ffi link drops with it, and the host's
    // session-close watch fires. Host RPC awaits resolve cleanly and can
    // pair the closure with the recorded reason via
    // `dodeca_cell_runtime::take_cell_death(cell_name)`.
    install_cell_panic_recorder();

    let peer = peer as usize;
    std::thread::Builder::new()
        .name(format!("cell-{cell_name}"))
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .thread_name(format!("cell-{cell_name}-rt"))
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(cell = cell_name, error = %e, "cell: tokio runtime build failed");
                    return;
                }
            };
            // Wrap block_on so a panic that escapes the runtime (panic from a
            // .block_on'd future itself, as opposed to a spawned task tokio
            // already catches) doesn't poison the thread — the runtime still
            // drops, the vox-ffi link drops, the host's session closes. The
            // recording happens in the panic hook regardless.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
            }));
            // Drop runtime here regardless of panic outcome — releases the
            // FFI link cleanly so the host's session sees close.
            drop(runtime);
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
