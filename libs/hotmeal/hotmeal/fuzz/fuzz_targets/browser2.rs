#![no_main]

//! Browser-side roundtrip fuzzer.
//!
//! This target sends old + new HTML to the browser, which:
//! 1. Parses both with DOMParser (browser normalization)
//! 2. Computes diff using hotmeal-wasm
//! 3. Applies patches
//! 4. Compares result with expected
//!
//! Since parsing and patching use the same browser, there should be no parser mismatches.

use std::sync::OnceLock;

use browser_proto::{BrowserFuzzerClient, RoundtripResult};
use chromiumoxide::{
    Browser, BrowserConfig,
    cdp::browser_protocol::network::EnableParams,
    cdp::js_protocol::runtime::{EnableParams as RuntimeEnableParams, EventConsoleApiCalled},
};
use futures_util::StreamExt;
use libfuzzer_sys::fuzz_target;
use roam_stream::{ConnectionHandle, HandshakeConfig, NoDispatcher};
use roam_websocket::{WsTransport, ws_accept};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::accept_async;

// Store Chrome PID for cleanup
static CHROME_PID: OnceLock<u32> = OnceLock::new();
static PANIC_HOOK_SET: OnceLock<()> = OnceLock::new();

fn kill_chrome() {
    if let Some(&pid) = CHROME_PID.get() {
        eprintln!("[browser2-fuzz] Killing Chrome (pid {})", pid);
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

/// Split raw bytes at 0xFF delimiter into two HTML strings.
/// Format: html_a 0xFF html_b
/// Returns None if input doesn't contain delimiter or isn't valid UTF-8.
fn split_input(data: &[u8]) -> Option<(String, String)> {
    let pos = data.iter().position(|&b| b == 0xFF)?;
    let html_a = std::str::from_utf8(&data[..pos]).ok()?.to_owned();
    let html_b = std::str::from_utf8(&data[pos + 1..]).ok()?.to_owned();
    Some((html_a, html_b))
}

struct FuzzRequest {
    old_html: String,
    new_html: String,
    response_tx: oneshot::Sender<Option<RoundtripResult>>,
}

static CHANNEL: OnceLock<mpsc::UnboundedSender<FuzzRequest>> = OnceLock::new();

fn get_channel() -> &'static mpsc::UnboundedSender<FuzzRequest> {
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

async fn run_browser_worker(mut rx: mpsc::UnboundedReceiver<FuzzRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    eprintln!(
        "[browser2-fuzz] WebSocket server listening on port {}",
        port
    );

    let headed = std::env::var("BROWSER_HEAD").is_ok();
    let mut config = BrowserConfig::builder()
        .arg("--allow-file-access-from-files")
        .arg("--disable-web-security");
    if headed {
        config = config.with_head().arg("--auto-open-devtools-for-tabs");
    }
    let (mut browser, mut handler) = Browser::launch(config.build().unwrap()).await.unwrap();

    if let Some(child) = browser.get_mut_child() {
        if let Some(pid) = child.inner.id() {
            let _ = CHROME_PID.set(pid);
            eprintln!("[browser2-fuzz] Chrome launched (pid {})", pid);
        }
    }

    tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            let _ = event;
        }
    });

    let bundle_path = std::env::current_dir()
        .unwrap()
        .join("browser-bundle")
        .join("dist")
        .join("index.html");
    let bundle_url = format!("file://{}#{}", bundle_path.display(), port);
    eprintln!("[browser2-fuzz] Loading bundle from: {}", bundle_url);

    let page = browser.new_page(&bundle_url).await.unwrap();

    // Enable runtime to capture console logs (if BROWSER_CONSOLE is set)
    if std::env::var("BROWSER_CONSOLE").is_ok() {
        page.execute(RuntimeEnableParams::default()).await.ok();

        let mut console_events = page
            .event_listener::<EventConsoleApiCalled>()
            .await
            .unwrap();
        tokio::spawn(async move {
            while let Some(event) = console_events.next().await {
                let args: Vec<String> = event
                    .args
                    .iter()
                    .filter_map(|arg| arg.value.as_ref().map(|v| v.to_string()))
                    .collect();
                eprintln!("[console] {}", args.join(" "));
            }
        });
    }

    if headed {
        page.execute(EnableParams::default()).await.ok();
        eprintln!("[browser2-fuzz] Network logging enabled");
    }

    eprintln!("[browser2-fuzz] Page loaded, WASM will connect automatically");

    let (conn_tx, conn_rx) = watch::channel::<Option<ConnectionHandle>>(None);
    let listener_handle = tokio::spawn(accept_connections(listener, conn_tx));

    while let Some(req) = rx.recv().await {
        let handle = loop {
            let current = conn_rx.borrow().clone();
            if let Some(h) = current {
                break h;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };

        let client = BrowserFuzzerClient::new(handle);
        match client.test_roundtrip(req.old_html, req.new_html).await {
            Ok(result) => {
                let _ = req.response_tx.send(Some(result));
            }
            Err(e) => {
                eprintln!("[browser2-fuzz] Error: {:?}", e);
                let _ = req.response_tx.send(None);
            }
        }
    }

    listener_handle.abort();
}

async fn accept_connections(
    listener: TcpListener,
    conn_tx: watch::Sender<Option<ConnectionHandle>>,
) {
    loop {
        eprintln!(
            "[browser2-fuzz] Waiting for browser to connect on {:?}...",
            listener.local_addr()
        );
        match listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!(
                    "[browser2-fuzz] TCP accept from {}, upgrading to WebSocket...",
                    addr
                );
                match accept_async(stream).await {
                    Ok(ws_stream) => {
                        eprintln!(
                            "[browser2-fuzz] WebSocket upgrade complete, starting roam handshake..."
                        );
                        let transport = WsTransport::new(ws_stream);
                        match ws_accept(transport, HandshakeConfig::default(), NoDispatcher).await {
                            Ok((handle, _incoming, driver)) => {
                                eprintln!("[browser2-fuzz] Roam handshake complete");
                                tokio::spawn(async move {
                                    if let Err(e) = driver.run().await {
                                        eprintln!("[browser2-fuzz] Driver error: {:?}", e);
                                    }
                                });
                                let _ = conn_tx.send(Some(handle));
                                eprintln!("[browser2-fuzz] Ready to process fuzz requests");
                            }
                            Err(e) => {
                                eprintln!("[browser2-fuzz] Roam handshake failed: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[browser2-fuzz] WebSocket upgrade failed: {:?}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("[browser2-fuzz] Accept failed: {:?}", e);
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    // Split at 0xFF delimiter
    let Some((html_a, html_b)) = split_input(data) else {
        return;
    };

    // Skip empty inputs
    if html_a.is_empty() || html_b.is_empty() {
        return;
    }

    // Send to browser worker
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(FuzzRequest {
            old_html: html_a.clone(),
            new_html: html_b.clone(),
            response_tx,
        })
        .unwrap();

    // Wait for result
    let Some(result) = response_rx.blocking_recv().unwrap() else {
        // Error occurred (logged by worker)
        return;
    };

    // Skip cases where both normalize to empty (garbage input)
    if result.normalized_old.trim().is_empty() && result.normalized_new.trim().is_empty() {
        return;
    }

    // Success!
});
