//! In-process cell loader (vox-ffi).
//!
//! Replaces the old roam-shm separate-process model. Each cell is a **cdylib**
//! (`libddc_cell_<name>.{dylib,so,dll}`) shipped next to `ddc`. On first use a
//! cell is `dlopen`'d, its exported `dodeca_cell_vtable_v1` symbol resolved, and
//! a vox-ffi link established: the host is the **initiator**, the cell (its
//! `declare_cell!` bootstrap) is the **acceptor**.
//!
//! The host serves `HostService` (+ `DevtoolsService` when a site server is
//! present) back to the cell over the same link, multiplexed as vox virtual
//! connections routed by the `vox-service` name. Typed cell clients are
//! obtained by opening a virtual connection on the stored root `SessionHandle`.

use std::io;
use std::sync::Arc;

use dashmap::DashMap;
use libloading::Library;
use tokio::sync::Mutex;
use tracing::{debug, error};
use vox::{ConnectionRequest, Metadata, PendingConnection, SessionHandle};
use vox_ffi::{declare_link_endpoint, vox_link_vtable};

use crate::host::Host;

/// Exported symbol every cell cdylib provides (see `dodeca_cell_runtime::declare_cell!`).
const CELL_VTABLE_SYMBOL: &[u8] = b"dodeca_cell_vtable_v1";

/// Settings for every virtual connection opened over a host<->cell FFI link.
pub fn connection_settings() -> vox::ConnectionSettings {
    vox::ConnectionSettings {
        parity: vox::Parity::Odd,
        max_concurrent_requests: 64,
        initial_channel_credit: 16,
    }
}

type CellVtableFn = unsafe extern "C" fn() -> *const vox_link_vtable;

// One vox-ffi endpoint per cell link (Endpoint is a one-shot process-lifetime
// static, so each host<->cell link needs its own). The exported symbols are
// unused by anyone (the host is the initiator) but `declare_link_endpoint!`
// always emits one; harmless extra symbols in the `ddc` binary.
macro_rules! host_cell_endpoints {
    ($($mod:ident => $cell:literal , $sym:ident);* $(;)?) => {
        $( declare_link_endpoint!(mod $mod { export = $sym; }); )*

        /// Connect this host's per-cell endpoint to the cell's vtable.
        fn host_connect(cell: &str, peer: &'static vox_link_vtable) -> io::Result<vox_ffi::FfiLink> {
            match cell {
                $( $cell => $mod::connect(peer), )*
                _ => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no host ffi endpoint for cell {cell}"),
                )),
            }
        }
    };
}

host_cell_endpoints! {
    ep_image          => "image",          vox_ffi_host_image_v1;
    ep_webp           => "webp",           vox_ffi_host_webp_v1;
    ep_jxl            => "jxl",            vox_ffi_host_jxl_v1;
    ep_markdown       => "markdown",       vox_ffi_host_markdown_v1;
    ep_html           => "html",           vox_ffi_host_html_v1;
    ep_minify         => "minify",         vox_ffi_host_minify_v1;
    ep_css            => "css",            vox_ffi_host_css_v1;
    ep_sass           => "sass",           vox_ffi_host_sass_v1;
    ep_js             => "js",             vox_ffi_host_js_v1;
    ep_svgo           => "svgo",           vox_ffi_host_svgo_v1;
    ep_fonts          => "fonts",          vox_ffi_host_fonts_v1;
    ep_linkcheck      => "linkcheck",      vox_ffi_host_linkcheck_v1;
    ep_search         => "search",         vox_ffi_host_search_v1;
    ep_html_diff      => "html-diff",      vox_ffi_host_html_diff_v1;
    ep_dialoguer      => "dialoguer",      vox_ffi_host_dialoguer_v1;
    ep_code_execution => "code-execution", vox_ffi_host_code_execution_v1;
    ep_authoring_lsp  => "authoring-lsp",  vox_ffi_host_authoring_lsp_v1;
    ep_http           => "http",           vox_ffi_host_http_v1;
    ep_gingembre      => "gingembre",      vox_ffi_host_gingembre_v1;
    ep_data           => "data",           vox_ffi_host_data_v1;
    ep_vite           => "vite",           vox_ffi_host_vite_v1;
    ep_term           => "term",           vox_ffi_host_term_v1;
    ep_tui            => "tui",            vox_ffi_host_tui_v1;
}

/// The host-side acceptor: routes the cell's reverse virtual connections to the
/// unified `HostService` and (when serving) `DevtoolsService`. Each accepted
/// `DevtoolsService` vconn is one browser session and gets its own service
/// instance carrying a fresh browser id.
struct HostAcceptor {
    host_service: crate::cells::HostServiceImpl,
}

impl vox::ConnectionAcceptor for HostAcceptor {
    fn accept(
        &self,
        request: &ConnectionRequest,
        connection: PendingConnection,
    ) -> Result<(), Metadata<'static>> {
        use vox::FromVoxSession;
        match request.service() {
            s if s == cell_host_proto::HostServiceClient::SERVICE_NAME => {
                connection.handle_with(cell_host_proto::HostServiceDispatcher::new(
                    self.host_service.clone(),
                ));
                Ok(())
            }
            s if s == dodeca_protocol::DevtoolsServiceClient::SERVICE_NAME => {
                match Host::get().site_server() {
                    Some(server) => {
                        let browser_id = crate::cell_server::next_devtools_browser_id();
                        tracing::debug!(browser_id, "devtools host service connection accepted");
                        let svc = crate::cell_server::HostDevtoolsService::new(
                            server.clone(),
                            browser_id,
                        );
                        let browser: dodeca_protocol::BrowserServiceClient = connection
                            .handle_with_client(dodeca_protocol::DevtoolsServiceDispatcher::new(
                                svc,
                            ));
                        server.register_browser(browser_id, browser.clone());
                        crate::spawn::spawn({
                            let server = server.clone();
                            async move {
                                browser.caller.closed().await;
                                server.unregister_browser(browser_id);
                            }
                        });
                        Ok(())
                    }
                    None => Err(vec![vox::MetadataEntry::str(
                        "error",
                        "devtools unavailable: not serving",
                    )]),
                }
            }
            s if s == vox::NoopClient::SERVICE_NAME => {
                connection.handle_with(());
                Ok(())
            }
            other => Err(vec![vox::MetadataEntry::str(
                "error",
                format!("unknown service {other}"),
            )]),
        }
    }
}

/// Per-cell loaded state: the leaked dylib (kept alive forever — its static
/// vtable must outlive the link) and the established root session.
struct LoadedCell {
    _lib: &'static Library,
    session: SessionHandle,
}

static LOADED: std::sync::OnceLock<DashMap<String, Arc<LoadedCell>>> = std::sync::OnceLock::new();
static LOAD_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

fn loaded() -> &'static DashMap<String, Arc<LoadedCell>> {
    LOADED.get_or_init(DashMap::new)
}

/// Get (loading on first use) the root vox session for a cell.
///
/// Returns `None` if the cell cdylib is missing or the link fails to establish.
pub async fn cell_session(cell_name: &str) -> Option<SessionHandle> {
    if let Some(c) = loaded().get(cell_name) {
        return Some(c.session.clone());
    }
    // Serialize concurrent first-loads of the same cell.
    let _guard = LOAD_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    if let Some(c) = loaded().get(cell_name) {
        return Some(c.session.clone());
    }

    let path = cell_library_path(cell_name)?;
    debug!(cell = cell_name, path = %path.display(), "loading cell cdylib");

    let lib = match unsafe { Library::new(&path) } {
        Ok(l) => Box::leak(Box::new(l)) as &'static Library,
        Err(e) => {
            error!(cell = cell_name, error = %e, "dlopen failed");
            return None;
        }
    };
    let vtable_fn: libloading::Symbol<CellVtableFn> = match unsafe { lib.get(CELL_VTABLE_SYMBOL) } {
        Ok(s) => s,
        Err(e) => {
            error!(cell = cell_name, error = %e, "missing dodeca_cell_vtable_v1");
            return None;
        }
    };
    let cell_vtable = match unsafe { vox_link_vtable::validate_ptr(vtable_fn()) } {
        Ok(v) => v,
        Err(e) => {
            error!(cell = cell_name, error = %e, "invalid cell vtable");
            return None;
        }
    };

    let link = match host_connect(cell_name, cell_vtable) {
        Ok(l) => l,
        Err(e) => {
            error!(cell = cell_name, error = %e, "host ffi connect failed");
            return None;
        }
    };

    let host_service = crate::cells::make_host_service();
    let root = match vox::initiator_on(link, vox::TransportMode::Bare)
        .observer(vox::TracingObserver::new())
        .on_connection(HostAcceptor { host_service })
        .establish::<vox::NoopClient>()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(cell = cell_name, error = %e, "host initiator establish failed");
            return None;
        }
    };
    let session = root.session.clone()?;
    // Keep the root NoopClient alive for the link's lifetime.
    std::mem::forget(root);

    loaded().insert(
        cell_name.to_string(),
        Arc::new(LoadedCell {
            _lib: lib,
            session: session.clone(),
        }),
    );
    debug!(cell = cell_name, "cell established");
    Some(session)
}

/// Locate `libddc_cell_<name>.{dylib,so,dll}` next to `ddc` (or in the target
/// dir during dev). Mirrors the old `find_cell_directory` search.
fn cell_library_path(cell_name: &str) -> Option<std::path::PathBuf> {
    let file = format!(
        "{}ddc_cell_{}{}",
        std::env::consts::DLL_PREFIX,
        cell_name.replace('-', "_"),
        std::env::consts::DLL_SUFFIX,
    );
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("DODECA_CELL_PATH") {
        dirs.push(p.into());
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            dirs.push(d.to_path_buf());
        }
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    dirs.push(std::path::Path::new("target").join(profile));
    for d in dirs {
        let candidate = d.join(&file);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    error!(cell = cell_name, file, "cell cdylib not found");
    None
}
