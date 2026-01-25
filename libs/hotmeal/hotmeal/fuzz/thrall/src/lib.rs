//! Thrall - Control browsers from Rust tests
//!
//! Named after the supernatural servants of demons and vampires,
//! this crate lets you control a Chrome browser for testing purposes.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use browser_proto::BrowserClient;
pub use browser_proto::{ApplyPatchesResult, ComputeAndApplyResult, DomNode, OwnedPatches};
use chromiumoxide::{
    Browser, BrowserConfig,
    cdp::browser_protocol::network::{
        EnableParams, EventLoadingFailed, EventRequestWillBeSent, EventResponseReceived,
    },
    cdp::js_protocol::runtime::{
        EnableParams as RuntimeEnableParams, EventConsoleApiCalled, EventExceptionThrown,
    },
    fetcher::{BrowserFetcher, BrowserFetcherOptions},
};
use futures_util::StreamExt;
use roam_stream::{ConnectionHandle, HandshakeConfig, NoDispatcher};
use roam_websocket::{WsTransport, ws_accept};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::accept_async;

static CHANNEL: OnceLock<mpsc::UnboundedSender<BrowserRequest>> = OnceLock::new();
static CONFIG: OnceLock<ThrallConfig> = OnceLock::new();

/// Configuration for the thrall browser worker.
#[derive(Clone, Copy, Debug)]
pub struct ThrallConfig {
    /// Log network requests/responses
    pub log_network: bool,
    /// Log CDP events
    pub log_cdp: bool,
    /// Log JS exceptions (always recommended)
    pub log_js_exceptions: bool,
    /// Log console messages (errors/warnings)
    pub log_console: bool,
    /// Log thrall lifecycle messages (startup, connections, etc.)
    pub log_lifecycle: bool,
}

impl Default for ThrallConfig {
    fn default() -> Self {
        Self {
            log_network: false,
            log_cdp: false,
            log_js_exceptions: true,
            log_console: true,
            log_lifecycle: true,
        }
    }
}

impl ThrallConfig {
    /// Verbose config suitable for debugging tests.
    pub fn verbose() -> Self {
        Self {
            log_network: true,
            log_cdp: true,
            log_js_exceptions: true,
            log_console: true,
            log_lifecycle: true,
        }
    }

    /// Quiet config suitable for fuzzing - only logs JS exceptions.
    pub fn quiet() -> Self {
        Self {
            log_network: false,
            log_cdp: false,
            log_js_exceptions: true,
            log_console: false,
            log_lifecycle: false,
        }
    }
}

/// Set the thrall configuration. Must be called before any browser operations.
/// If not called, uses default config (lifecycle + console + exceptions).
pub fn configure(config: ThrallConfig) {
    CONFIG.set(config).ok();
}

fn config() -> &'static ThrallConfig {
    CONFIG.get_or_init(ThrallConfig::default)
}

macro_rules! log_lifecycle {
    ($($arg:tt)*) => {
        if config().log_lifecycle {
            eprintln!($($arg)*);
        }
    };
}

/// Internal request enum covering all Browser service methods.
enum BrowserRequest {
    ApplyPatches {
        old_html: String,
        patches: OwnedPatches,
        response_tx: oneshot::Sender<Option<ApplyPatchesResult>>,
    },
    ComputeAndApplyPatches {
        old_html: String,
        new_html: String,
        response_tx: oneshot::Sender<Option<ComputeAndApplyResult>>,
    },
    ParseToDom {
        html: String,
        response_tx: oneshot::Sender<Option<DomNode>>,
    },
}

/// Returns the channel through which one may send requests to the browser worker.
fn get_channel() -> &'static mpsc::UnboundedSender<BrowserRequest> {
    CHANNEL.get_or_init(|| {
        setup_cleanup_hooks();
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(run_browser_worker(rx))
        });
        tx
    })
}

/// Apply pre-computed patches to HTML in the browser (blocking, synchronous).
///
/// Returns `None` if the browser connection failed.
pub fn apply_patches(old_html: String, patches: OwnedPatches) -> Option<ApplyPatchesResult> {
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(BrowserRequest::ApplyPatches {
            old_html,
            patches,
            response_tx,
        })
        .unwrap();
    response_rx.blocking_recv().unwrap()
}

/// Compute and apply patches in the browser (blocking, synchronous).
///
/// Returns `None` if the browser connection failed.
pub fn compute_and_apply_patches(
    old_html: String,
    new_html: String,
) -> Option<ComputeAndApplyResult> {
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(BrowserRequest::ComputeAndApplyPatches {
            old_html,
            new_html,
            response_tx,
        })
        .unwrap();
    response_rx.blocking_recv().unwrap()
}

/// Parse HTML in the browser and return the DOM tree (blocking, synchronous).
///
/// Returns `None` if the browser connection failed.
pub fn parse_to_dom(html: String) -> Option<DomNode> {
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(BrowserRequest::ParseToDom { html, response_tx })
        .unwrap();
    response_rx.blocking_recv().unwrap()
}

async fn run_browser_worker(mut rx: mpsc::UnboundedReceiver<BrowserRequest>) {
    let cfg = config();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    log_lifecycle!("[thrall] WebSocket server listening on port {}", port);

    // Get Chrome executable - either from system or download it
    let chrome_path = get_or_fetch_chrome().await;

    let headed = std::env::var("BROWSER_HEAD").ok().as_deref() == Some("1");
    let no_sandbox =
        std::env::var("THRALL_NO_SANDBOX").ok().as_deref() == Some("1") || is_running_in_ci();

    let mut browser_config = BrowserConfig::builder()
        .arg("--allow-file-access-from-files")
        .arg("--disable-web-security");
    if no_sandbox {
        // Required when running as root in CI containers
        browser_config = browser_config.arg("--no-sandbox");
        log_lifecycle!("[thrall] Running with --no-sandbox (CI mode)");
    }
    if let Some(path) = chrome_path {
        browser_config = browser_config.chrome_executable(path);
    }
    browser_config = browser_config
        .with_head()
        .arg("--auto-open-devtools-for-tabs");
    if !headed {
        // Use new headless mode for better compatibility
        browser_config = browser_config.arg("--headless=new");
    }
    let (mut browser, mut handler) = Browser::launch(browser_config.build().unwrap())
        .await
        .unwrap();

    if let Some(child) = browser.get_mut_child()
        && let Some(pid) = child.inner.id()
    {
        let _ = CHROME_PID.set(pid);
        log_lifecycle!("[thrall] Chrome launched (pid {})", pid);
    }

    let log_cdp_events = cfg.log_cdp;
    tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if log_cdp_events {
                eprintln!("[cdp] {:?}", event);
            }
        }
    });

    // Create a blank page first so we can set up event listeners before navigation
    let page = browser.new_page("about:blank").await.unwrap();

    // Enable runtime and network BEFORE navigating
    page.execute(RuntimeEnableParams::default()).await.ok();
    page.execute(EnableParams::default()).await.ok();

    // Set up console logging
    let log_console = cfg.log_console;
    let verbose_console = std::env::var("BROWSER_CONSOLE").ok().as_deref() == Some("1");
    let mut console_events = page
        .event_listener::<EventConsoleApiCalled>()
        .await
        .unwrap();
    tokio::spawn(async move {
        while let Some(event) = console_events.next().await {
            let level = format!("{:?}", event.r#type);
            let is_error = matches!(
                event.r#type,
                chromiumoxide::cdp::js_protocol::runtime::ConsoleApiCalledType::Error
                    | chromiumoxide::cdp::js_protocol::runtime::ConsoleApiCalledType::Warning
            );
            if (log_console && is_error) || verbose_console {
                let args: Vec<String> = event
                    .args
                    .iter()
                    .filter_map(|arg| arg.value.as_ref().map(|v| v.to_string()))
                    .collect();
                eprintln!("[console:{}] {}", level, args.join(" "));
            }
        }
    });

    // Log JS exceptions
    let log_js_exceptions = cfg.log_js_exceptions;
    let mut exception_events = page.event_listener::<EventExceptionThrown>().await.unwrap();
    tokio::spawn(async move {
        while let Some(event) = exception_events.next().await {
            if log_js_exceptions {
                eprintln!("[js:exception] {:?}", event.exception_details);
            }
        }
    });

    // Log network requests
    let log_net = cfg.log_network;
    let mut request_events = page
        .event_listener::<EventRequestWillBeSent>()
        .await
        .unwrap();
    tokio::spawn(async move {
        while let Some(event) = request_events.next().await {
            if log_net {
                eprintln!(
                    "[net:request] {} {}",
                    event.request.method, event.request.url
                );
            }
        }
    });

    let mut response_events = page
        .event_listener::<EventResponseReceived>()
        .await
        .unwrap();
    tokio::spawn(async move {
        while let Some(event) = response_events.next().await {
            if log_net {
                eprintln!(
                    "[net:response] {} {} ({})",
                    event.response.status, event.response.url, &event.response.mime_type
                );
            }
        }
    });

    let mut failed_events = page.event_listener::<EventLoadingFailed>().await.unwrap();
    tokio::spawn(async move {
        while let Some(event) = failed_events.next().await {
            if log_net {
                eprintln!("[net:FAILED] {:?} - {}", event.request_id, event.error_text);
            }
        }
    });

    log_lifecycle!("[thrall] Event listeners configured");

    // Start HTTP server for the bundle (file:// URLs don't work well in headless mode)
    let bundle_path = find_browser_bundle().expect("Could not find browser-bundle/dist/index.html");
    let dist_dir = bundle_path.parent().unwrap().to_path_buf();
    let http_port = start_http_file_server(dist_dir).await;
    log_lifecycle!("[thrall] HTTP file server on port {}", http_port);

    // Navigate to the bundle via HTTP, with WebSocket port in the fragment
    let bundle_url = format!("http://127.0.0.1:{}/#{}", http_port, port);
    log_lifecycle!("[thrall] Loading bundle from: {}", bundle_url);

    page.goto(&bundle_url).await.unwrap();
    log_lifecycle!("[thrall] Page navigation started, waiting for WASM to connect...");

    let (conn_tx, conn_rx) = watch::channel::<Option<ConnectionHandle>>(None);
    let listener_handle = tokio::spawn(accept_connections(listener, conn_tx));

    while let Some(req) = rx.recv().await {
        let handle = loop {
            let current = conn_rx.borrow().clone();
            if let Some(h) = current {
                break h;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await
        };

        let client = BrowserClient::new(handle);

        match req {
            BrowserRequest::ApplyPatches {
                old_html,
                patches,
                response_tx,
            } => match client.apply_patches(old_html, patches).await {
                Ok(result) => {
                    let _ = response_tx.send(Some(result));
                }
                Err(e) => {
                    log_lifecycle!("[thrall] apply_patches error: {:?}", e);
                    let _ = response_tx.send(None);
                }
            },
            BrowserRequest::ComputeAndApplyPatches {
                old_html,
                new_html,
                response_tx,
            } => match client.compute_and_apply_patches(old_html, new_html).await {
                Ok(result) => {
                    let _ = response_tx.send(Some(result));
                }
                Err(e) => {
                    log_lifecycle!("[thrall] compute_and_apply_patches error: {:?}", e);
                    let _ = response_tx.send(None);
                }
            },
            BrowserRequest::ParseToDom { html, response_tx } => {
                match client.parse_to_dom(html).await {
                    Ok(result) => {
                        let _ = response_tx.send(Some(result));
                    }
                    Err(e) => {
                        log_lifecycle!("[thrall] parse_to_dom error: {:?}", e);
                        let _ = response_tx.send(None);
                    }
                }
            }
        }
    }

    listener_handle.abort();
}

/// Detect if we're running in a CI environment.
fn is_running_in_ci() -> bool {
    // GitHub Actions
    std::env::var("CI").is_ok() ||
    std::env::var("GITHUB_ACTIONS").is_ok() ||
    // GitLab CI
    std::env::var("GITLAB_CI").is_ok() ||
    // Other common CI systems
    std::env::var("JENKINS_URL").is_ok() ||
    std::env::var("CIRCLECI").is_ok() ||
    std::env::var("TRAVIS").is_ok()
}

/// Get Chrome executable path, downloading if necessary.
///
/// Returns `None` to let chromiumoxide find system Chrome.
/// Returns `Some(path)` if we downloaded Chrome or CHROME_PATH is set.
async fn get_or_fetch_chrome() -> Option<PathBuf> {
    // Try to use CHROME_PATH env var if set
    if let Ok(chrome_path) = std::env::var("CHROME_PATH") {
        let path = PathBuf::from(chrome_path);
        if path.exists() {
            log_lifecycle!("[thrall] Using Chrome from CHROME_PATH: {}", path.display());
            return Some(path);
        }
    }

    // Check if we should skip auto-download (use system Chrome)
    if std::env::var("THRALL_NO_DOWNLOAD").is_ok() {
        log_lifecycle!("[thrall] THRALL_NO_DOWNLOAD set, using system Chrome");
        return None;
    }

    // Download Chrome using chromiumoxide's fetcher
    let cache_dir = dirs_next::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("thrall")
        .join("chrome");

    log_lifecycle!("[thrall] Fetching Chrome to {}...", cache_dir.display());

    tokio::fs::create_dir_all(&cache_dir).await.ok()?;

    let fetcher = BrowserFetcher::new(
        BrowserFetcherOptions::builder()
            .with_path(&cache_dir)
            .build()
            .ok()?,
    );

    // fetch() handles caching internally - it won't re-download if already present
    match fetcher.fetch().await {
        Ok(info) => {
            log_lifecycle!("[thrall] Using Chrome: {}", info.executable_path.display());
            Some(info.executable_path)
        }
        Err(e) => {
            log_lifecycle!("[thrall] Failed to fetch Chrome: {:?}", e);
            log_lifecycle!("[thrall] Will try system Chrome instead");
            None
        }
    }
}

/// Find the browser bundle directory.
fn find_browser_bundle() -> Option<std::path::PathBuf> {
    let index_html = "index.html";

    // Try CARGO_MANIFEST_DIR first (most reliable in cargo test)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest_path = std::path::PathBuf::from(&manifest_dir);
        log_lifecycle!("[thrall] CARGO_MANIFEST_DIR: {}", manifest_dir);

        // From hotmeal/fuzz (when running cargo test from fuzz dir)
        let path1 = manifest_path
            .join("browser-bundle")
            .join("dist")
            .join(index_html);
        if path1.exists() {
            return Some(path1);
        }

        // From hotmeal/fuzz/thrall -> hotmeal/fuzz/browser-bundle
        if let Some(parent) = manifest_path.parent() {
            let path2 = parent.join("browser-bundle").join("dist").join(index_html);
            if path2.exists() {
                return Some(path2);
            }
        }

        // From hotmeal -> hotmeal/fuzz/browser-bundle
        let path3 = manifest_path
            .join("fuzz")
            .join("browser-bundle")
            .join("dist")
            .join(index_html);
        if path3.exists() {
            return Some(path3);
        }
    }

    // Fallback: try relative to current working directory
    if let Ok(cwd) = std::env::current_dir() {
        log_lifecycle!("[thrall] CWD: {}", cwd.display());

        // Check: ./browser-bundle/dist/index.html
        let path4 = cwd.join("browser-bundle").join("dist").join(index_html);
        if path4.exists() {
            return Some(path4);
        }

        // Check: ./fuzz/browser-bundle/dist/index.html
        let path5 = cwd
            .join("fuzz")
            .join("browser-bundle")
            .join("dist")
            .join(index_html);
        if path5.exists() {
            return Some(path5);
        }
    }

    None
}

async fn accept_connections(
    listener: TcpListener,
    conn_tx: watch::Sender<Option<ConnectionHandle>>,
) {
    loop {
        log_lifecycle!(
            "[thrall] Waiting for browser to connect on {:?}...",
            listener.local_addr()
        );
        match listener.accept().await {
            Ok((stream, addr)) => {
                log_lifecycle!(
                    "[thrall] TCP accept from {}, upgrading to WebSocket...",
                    addr
                );
                match accept_async(stream).await {
                    Ok(ws_stream) => {
                        log_lifecycle!(
                            "[thrall] WebSocket upgrade complete, starting roam handshake..."
                        );
                        let transport = WsTransport::new(ws_stream);
                        match ws_accept(transport, HandshakeConfig::default(), NoDispatcher).await {
                            Ok((handle, _incoming, driver)) => {
                                log_lifecycle!("[thrall] Roam handshake complete");
                                tokio::spawn(async move {
                                    if let Err(e) = driver.run().await {
                                        log_lifecycle!("[thrall] Driver error: {:?}", e);
                                    }
                                });
                                let _ = conn_tx.send(Some(handle));
                                log_lifecycle!("[thrall] Ready to process requests");
                            }
                            Err(e) => log_lifecycle!("[thrall] Roam handshake failed: {:?}", e),
                        }
                    }
                    Err(e) => log_lifecycle!("[thrall] WebSocket upgrade failed: {:?}", e),
                }
            }
            Err(e) => log_lifecycle!("[thrall] Accept failed: {:?}", e),
        }
    }
}

// Store Chrome PID for cleanup
static CHROME_PID: OnceLock<u32> = OnceLock::new();
static PANIC_HOOK_SET: OnceLock<()> = OnceLock::new();

fn kill_chrome() {
    if let Some(&pid) = CHROME_PID.get() {
        log_lifecycle!("[thrall] Killing Chrome (pid {})", pid);
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

extern "C" fn atexit_handler() {
    kill_chrome();
}

fn setup_cleanup_hooks() {
    PANIC_HOOK_SET.get_or_init(|| {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            kill_chrome();
            original(info);
        }));
        unsafe {
            libc::signal(libc::SIGINT, signal_handler as *const () as usize);
            libc::signal(libc::SIGTERM, signal_handler as *const () as usize);
            libc::atexit(atexit_handler);
        }
    });
}

extern "C" fn signal_handler(sig: libc::c_int) {
    kill_chrome();
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

/// Simple HTTP file server for serving the browser bundle.
/// Returns the port it's listening on.
async fn start_http_file_server(dist_dir: PathBuf) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let dist_dir = Arc::new(dist_dir);

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _addr)) = listener.accept().await else {
                continue;
            };
            let dist_dir = Arc::clone(&dist_dir);

            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..n]);

                // Parse the request path from "GET /path HTTP/1.1"
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");

                // Map / to /index.html
                let path = if path == "/" { "/index.html" } else { path };
                let path = path.trim_start_matches('/');

                // Security: prevent path traversal
                if path.contains("..") {
                    let response = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }

                let file_path = dist_dir.join(path);

                match tokio::fs::read(&file_path).await {
                    Ok(contents) => {
                        let content_type = match file_path.extension().and_then(|e| e.to_str()) {
                            Some("html") => "text/html",
                            Some("js") => "text/javascript",
                            Some("wasm") => "application/wasm",
                            Some("css") => "text/css",
                            Some("json") => "application/json",
                            _ => "application/octet-stream",
                        };

                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
                            content_type,
                            contents.len()
                        );
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(&contents).await;
                    }
                    Err(_) => {
                        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                        let _ = stream.write_all(response.as_bytes()).await;
                    }
                }
            });
        }
    });

    port
}
