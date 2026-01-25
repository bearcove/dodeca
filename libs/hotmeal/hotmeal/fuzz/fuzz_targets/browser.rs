#![no_main]

//! Browser-side patch application fuzzer.
//!
//! This target computes diff server-side using hotmeal, then sends patches
//! to the browser to apply. This tests that patches computed by hotmeal
//! can be correctly applied by hotmeal-wasm in the browser DOM.

use std::sync::OnceLock;

use browser_proto::{BrowserFuzzerClient, DomAttr, DomNode, OwnedPatches, TestPatchResult};
use chromiumoxide::{
    Browser, BrowserConfig,
    cdp::browser_protocol::network::EnableParams,
    cdp::js_protocol::runtime::{EnableParams as RuntimeEnableParams, EventConsoleApiCalled},
};
use futures_util::StreamExt;
use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;
use roam_stream::{ConnectionHandle, HandshakeConfig, NoDispatcher};
use roam_websocket::{WsTransport, ws_accept};
use similar::{ChangeTag, TextDiff};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::accept_async;

/// Convert hotmeal's Document body to DomNode tree for comparison with browser.
fn document_body_to_dom_node(doc: &hotmeal::Document) -> DomNode {
    let body = doc.body().expect("document has no body");
    // Convert body element itself (not just children)
    node_to_dom_node(doc, body)
}

fn node_to_dom_node(doc: &hotmeal::Document, node_id: hotmeal::NodeId) -> DomNode {
    let node = doc.get(node_id);
    match &node.kind {
        hotmeal::NodeKind::Element(elem) => {
            let tag = elem.tag.to_string().to_ascii_lowercase();

            // Collect attributes (attrs are tuples of (QualName, Stem))
            let mut attrs: Vec<DomAttr> = elem
                .attrs
                .iter()
                .map(|(qname, value)| DomAttr {
                    name: qname.local.to_string(),
                    value: value.as_ref().to_string(),
                })
                .collect();
            // Sort attributes by name for consistent comparison
            attrs.sort_by(|a, b| a.name.cmp(&b.name));

            // Collect children
            let children: Vec<DomNode> = doc
                .children(node_id)
                .map(|child_id| node_to_dom_node(doc, child_id))
                .collect();

            DomNode::Element {
                tag,
                attrs,
                children,
            }
        }
        hotmeal::NodeKind::Text(text) => DomNode::Text(text.as_ref().to_string()),
        hotmeal::NodeKind::Comment(text) => DomNode::Comment(text.as_ref().to_string()),
        hotmeal::NodeKind::Document => {
            // Document node - shouldn't happen when starting from body
            DomNode::Text(String::new())
        }
    }
}

/// Pretty-print a DomNode tree for diffing.
fn pretty_print_dom(node: &DomNode, indent: usize) -> String {
    let mut out = String::new();
    let prefix = "  ".repeat(indent);
    match node {
        DomNode::Element {
            tag,
            attrs,
            children,
        } => {
            out.push_str(&format!("{}<{}", prefix, tag));
            for attr in attrs {
                out.push_str(&format!(" {}={:?}", attr.name, attr.value));
            }
            out.push_str(">\n");
            for child in children {
                out.push_str(&pretty_print_dom(child, indent + 1));
            }
            out.push_str(&format!("{}</{}>\n", prefix, tag));
        }
        DomNode::Text(text) => {
            out.push_str(&format!("{}TEXT: {:?}\n", prefix, text));
        }
        DomNode::Comment(text) => {
            out.push_str(&format!("{}COMMENT: {:?}\n", prefix, text));
        }
    }
    out
}

// Store Chrome PID for cleanup
static CHROME_PID: OnceLock<u32> = OnceLock::new();
static PANIC_HOOK_SET: OnceLock<()> = OnceLock::new();

fn kill_chrome() {
    if let Some(&pid) = CHROME_PID.get() {
        eprintln!("[browser-fuzz] Killing Chrome (pid {})", pid);
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
    patches: OwnedPatches,
    response_tx: oneshot::Sender<Option<TestPatchResult>>,
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
    eprintln!("[browser-fuzz] WebSocket server listening on port {}", port);

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
            eprintln!("[browser-fuzz] Chrome launched (pid {})", pid);
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
    eprintln!("[browser-fuzz] Loading bundle from: {}", bundle_url);

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
        eprintln!("[browser-fuzz] Network logging enabled");
    }

    eprintln!("[browser-fuzz] Page loaded, WASM will connect automatically");

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
        match client.test_patch(req.old_html, req.patches).await {
            Ok(result) => {
                let _ = req.response_tx.send(Some(result));
            }
            Err(e) => {
                eprintln!("[browser-fuzz] Error: {:?}", e);
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
            "[browser-fuzz] Waiting for browser to connect on {:?}...",
            listener.local_addr()
        );
        match listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!(
                    "[browser-fuzz] TCP accept from {}, upgrading to WebSocket...",
                    addr
                );
                match accept_async(stream).await {
                    Ok(ws_stream) => {
                        eprintln!(
                            "[browser-fuzz] WebSocket upgrade complete, starting roam handshake..."
                        );
                        let transport = WsTransport::new(ws_stream);
                        match ws_accept(transport, HandshakeConfig::default(), NoDispatcher).await {
                            Ok((handle, _incoming, driver)) => {
                                eprintln!("[browser-fuzz] Roam handshake complete");
                                tokio::spawn(async move {
                                    if let Err(e) = driver.run().await {
                                        eprintln!("[browser-fuzz] Driver error: {:?}", e);
                                    }
                                });
                                let _ = conn_tx.send(Some(handle));
                                eprintln!("[browser-fuzz] Ready to process fuzz requests");
                            }
                            Err(e) => {
                                eprintln!("[browser-fuzz] Roam handshake failed: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[browser-fuzz] WebSocket upgrade failed: {:?}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("[browser-fuzz] Accept failed: {:?}", e);
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

    // Skip inputs with null bytes - browsers replace them with U+FFFD but html5ever
    // handles them differently, causing false parser mismatches
    if html_a.contains('\0') || html_b.contains('\0') {
        return;
    }

    // Wrap in full HTML document with DOCTYPE for no-quirks mode
    let full_a = format!("<!DOCTYPE html><html><body>{}</body></html>", html_a);
    let full_b = format!("<!DOCTYPE html><html><body>{}</body></html>", html_b);

    // Compute patches using hotmeal
    let old_tendril = StrTendril::from(full_a.clone());
    let new_tendril = StrTendril::from(full_b.clone());

    let patches = match hotmeal::diff_html(&old_tendril, &new_tendril) {
        Ok(p) => p,
        Err(_) => return, // Skip inputs that fail to diff
    };

    // Parse old HTML with html5ever and get DOM tree
    let old_doc = hotmeal::parse(&old_tendril);
    let html5ever_tree = document_body_to_dom_node(&old_doc);

    // Convert to owned for sending over roam
    let owned_patches = OwnedPatches(patches.iter().cloned().map(|p| p.into_owned()).collect());

    // Send to browser worker (send full wrapped HTML, not just fragment)
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(FuzzRequest {
            old_html: full_a.clone(),
            patches: owned_patches,
            response_tx,
        })
        .unwrap();

    // Wait for result
    let Some(result) = response_rx.blocking_recv().unwrap() else {
        // Error occurred (logged by worker)
        return;
    };

    // Skip cases where browser parsed to empty - that's garbage input
    if result.normalized_old_html.trim().is_empty() {
        return;
    }

    // Compare html5ever tree with browser tree - detect parser mismatches
    if html5ever_tree != result.initial_dom_tree {
        eprintln!("\n========== PARSER MISMATCH ==========");
        eprintln!("Input: {:?}", html_a);
        eprintln!("\n--- html5ever tree ---");
        eprintln!("{}", pretty_print_dom(&html5ever_tree, 0));
        eprintln!("--- browser tree ---");
        eprintln!("{}", pretty_print_dom(&result.initial_dom_tree, 0));
        eprintln!("\n--- diff ---");
        let h5e_str = pretty_print_dom(&html5ever_tree, 0);
        let browser_str = pretty_print_dom(&result.initial_dom_tree, 0);
        print_diff(&h5e_str, &browser_str);
        eprintln!("=====================================\n");
        panic!("Parser mismatch detected! Fix html5ever to match browser.");
    }
});

fn print_diff(expected: &str, actual: &str) {
    let diff = TextDiff::from_lines(expected, actual);
    for change in diff.iter_all_changes() {
        let (sign, color) = match change.tag() {
            ChangeTag::Delete => ("-", "\x1b[31m"), // red
            ChangeTag::Insert => ("+", "\x1b[32m"), // green
            ChangeTag::Equal => (" ", ""),
        };
        eprint!("{}{}{}\x1b[0m", color, sign, change.value());
    }
}
