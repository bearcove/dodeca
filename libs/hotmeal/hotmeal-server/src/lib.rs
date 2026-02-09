//! Server-side live-reload infrastructure for hotmeal.
//!
//! Provides transport-agnostic HTML diffing and patch delivery. The core abstraction
//! is `LiveReloadServer`, which caches HTML per route, diffs on change, and produces
//! `LiveReloadEvent` messages that can be serialized and sent over any transport
//! (WebSocket, SSE, etc.).
//!
//! The client side lives in `hotmeal-wasm` which deserializes events and applies
//! patches to a mount point in the browser DOM.

use std::collections::HashMap;

use facet::Facet;
use hotmeal::StrTendril;

#[cfg(feature = "tracing")]
use tracing::debug;

#[cfg(not(feature = "tracing"))]
macro_rules! debug {
    ($($tt:tt)*) => {};
}

// ============================================================================
// RPC Service Definitions (requires "roam" feature)
// ============================================================================

/// Service implemented by the browser, called by the server to push events.
///
/// The server calls `on_event()` on connected browsers whenever content changes
/// for a route they've subscribed to.
#[cfg(feature = "roam")]
#[roam::service]
pub trait LiveReloadBrowser {
    /// Called by the server when a live-reload event occurs.
    async fn on_event(&self, event: LiveReloadEvent);
}

/// Service implemented by the server, called by the browser to subscribe.
///
/// After the browser calls `subscribe(route)`, the server will push
/// `LiveReloadEvent`s via `LiveReloadBrowser::on_event()` on that connection.
#[cfg(feature = "roam")]
#[roam::service]
pub trait LiveReloadService {
    /// Subscribe to live-reload events for a route.
    async fn subscribe(&self, route: String);
}

// ============================================================================
// Event types
// ============================================================================

/// Events produced by the live-reload server.
///
/// These are serialized with postcard and sent to the browser client,
/// which deserializes them in `hotmeal-wasm`.
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum LiveReloadEvent {
    /// Full page reload needed.
    Reload,
    /// DOM patches for a route (postcard-serialized `Vec<Patch<'static>>`).
    Patches {
        route: String,
        patches_blob: Vec<u8>,
    },
    /// Head injections changed — full reload required.
    HeadChanged { route: String },
}

impl LiveReloadEvent {
    /// Serialize this event to postcard bytes.
    pub fn to_postcard(&self) -> Vec<u8> {
        facet_postcard::to_vec(self).expect("LiveReloadEvent serialization should not fail")
    }

    /// Deserialize a `LiveReloadEvent` from postcard bytes.
    pub fn from_postcard(bytes: &[u8]) -> Result<Self, facet_postcard::DeserializeError> {
        facet_postcard::from_slice(bytes)
    }
}

/// Server-side live-reload state.
///
/// Caches HTML and head injections per route, diffs new HTML against the cache,
/// and produces `LiveReloadEvent` messages.
///
/// Transport-agnostic — callers are responsible for delivering events to clients.
pub struct LiveReloadServer {
    /// Cached HTML per route.
    html_cache: HashMap<String, String>,
    /// Cached head injections per route.
    head_cache: HashMap<String, String>,
}

impl LiveReloadServer {
    pub fn new() -> Self {
        Self {
            html_cache: HashMap::new(),
            head_cache: HashMap::new(),
        }
    }

    /// Cache HTML for a route (call when serving). Returns previous HTML if any.
    pub fn cache_html(&mut self, route: &str, html: &str) -> Option<String> {
        self.html_cache.insert(route.to_owned(), html.to_owned())
    }

    /// Cache head injections. Returns true if they changed.
    pub fn cache_head_injections(&mut self, route: &str, injections: &str) -> bool {
        let prev = self
            .head_cache
            .insert(route.to_owned(), injections.to_owned());
        match prev {
            Some(ref old) => old != injections,
            None => !injections.is_empty(),
        }
    }

    /// Diff new HTML against cache. Returns event to send, or None if unchanged.
    pub fn diff_route(&mut self, route: &str, new_html: &str) -> Option<LiveReloadEvent> {
        let old_html = match self.html_cache.get(route) {
            Some(old) => old.clone(),
            None => {
                debug!(
                    route,
                    "no cached HTML for route, caching and returning Reload"
                );
                self.html_cache
                    .insert(route.to_owned(), new_html.to_owned());
                return Some(LiveReloadEvent::Reload);
            }
        };

        if old_html == new_html {
            return None;
        }

        let old_tendril = StrTendril::from(old_html.as_str());
        let new_tendril = StrTendril::from(new_html);

        match hotmeal::diff_html(&old_tendril, &new_tendril) {
            Ok(patches) => {
                if patches.is_empty() {
                    // Diff produced no patches — HTML is semantically identical
                    self.html_cache
                        .insert(route.to_owned(), new_html.to_owned());
                    return None;
                }

                let owned_patches: Vec<hotmeal::Patch<'static>> =
                    patches.into_iter().map(|p| p.into_owned()).collect();

                let patches_blob = facet_postcard::to_vec(&owned_patches)
                    .expect("patch serialization should not fail");

                debug!(
                    route,
                    num_patches = owned_patches.len(),
                    blob_size = patches_blob.len(),
                    "diff produced patches"
                );

                // Update cache
                self.html_cache
                    .insert(route.to_owned(), new_html.to_owned());

                Some(LiveReloadEvent::Patches {
                    route: route.to_owned(),
                    patches_blob,
                })
            }
            Err(_e) => {
                debug!(route, error = %_e, "diff failed, sending Reload");
                self.html_cache
                    .insert(route.to_owned(), new_html.to_owned());
                Some(LiveReloadEvent::Reload)
            }
        }
    }

    /// Diff with head injection tracking combined.
    ///
    /// If head injections changed, returns `HeadChanged` (requires full reload).
    /// Otherwise diffs body HTML and returns `Patches` or `None`.
    pub fn diff_route_with_head(
        &mut self,
        route: &str,
        new_html: &str,
        head_injections: &str,
    ) -> Option<LiveReloadEvent> {
        if self.cache_head_injections(route, head_injections) {
            // Head changed — must full reload, but still update HTML cache
            self.html_cache
                .insert(route.to_owned(), new_html.to_owned());
            return Some(LiveReloadEvent::HeadChanged {
                route: route.to_owned(),
            });
        }

        self.diff_route(route, new_html)
    }

    /// All cached route keys.
    pub fn cached_routes(&self) -> Vec<String> {
        self.html_cache.keys().cloned().collect()
    }

    /// Remove a route from cache.
    pub fn remove_route(&mut self, route: &str) -> bool {
        let html_removed = self.html_cache.remove(route).is_some();
        let head_removed = self.head_cache.remove(route).is_some();
        html_removed || head_removed
    }

    /// Clear all caches.
    pub fn clear(&mut self) {
        self.html_cache.clear();
        self.head_cache.clear();
    }
}

impl Default for LiveReloadServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Inject content after the opening `<head>` tag.
///
/// Searches for `<head>` (case-insensitive) and injects `content` immediately after it.
/// If no `<head>` tag is found, prepends content at the start.
pub fn inject_into_head(html: &str, content: &str) -> String {
    // Find <head> or <head ...> tag (case-insensitive)
    let lower = html.to_ascii_lowercase();
    if let Some(head_start) = lower.find("<head") {
        // Find the closing '>' of the <head> tag
        if let Some(head_end) = html[head_start..].find('>') {
            let insert_pos = head_start + head_end + 1;
            let mut result = String::with_capacity(html.len() + content.len());
            result.push_str(&html[..insert_pos]);
            result.push_str(content);
            result.push_str(&html[insert_pos..]);
            return result;
        }
    }

    // No <head> tag found — prepend
    let mut result = String::with_capacity(html.len() + content.len());
    result.push_str(content);
    result.push_str(html);
    result
}

/// Generate a `<script>` tag that loads hotmeal-wasm and starts live-reload.
///
/// Arguments:
/// - `wasm_js_url`: URL to the generated `hotmeal_wasm.js` glue code
/// - `wasm_url`: URL to the `.wasm` binary
/// - `mount_selector`: CSS selector for the mount point element (e.g. `"body"` or `"#content"`)
/// - `ws_url`: WebSocket URL for live-reload connection (e.g. `"ws://localhost:3000/_lr"`)
pub fn loader_script(
    wasm_js_url: &str,
    wasm_url: &str,
    mount_selector: &str,
    ws_url: &str,
) -> String {
    format!(
        r#"<script type="module">
import init, {{ start_live_reload }} from "{wasm_js_url}";
await init("{wasm_url}");
start_live_reload("{ws_url}", "{mount_selector}");
</script>"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_html_roundtrip() {
        let mut server = LiveReloadServer::new();
        assert!(server.cache_html("/", "<p>hello</p>").is_none());
        assert_eq!(
            server.cache_html("/", "<p>world</p>"),
            Some("<p>hello</p>".to_owned())
        );
    }

    #[test]
    fn diff_route_no_cache_returns_reload() {
        let mut server = LiveReloadServer::new();
        let event = server.diff_route("/new", "<p>hello</p>");
        assert!(matches!(event, Some(LiveReloadEvent::Reload)));
    }

    #[test]
    fn diff_route_unchanged_returns_none() {
        let mut server = LiveReloadServer::new();
        let html = "<p>hello</p>";
        server.cache_html("/", html);
        assert!(server.diff_route("/", html).is_none());
    }

    #[test]
    fn diff_route_produces_patches() {
        let mut server = LiveReloadServer::new();
        server.cache_html("/", "<p>hello</p>");
        let event = server.diff_route("/", "<p>world</p>");
        match event {
            Some(LiveReloadEvent::Patches {
                route,
                patches_blob,
            }) => {
                assert_eq!(route, "/");
                assert!(!patches_blob.is_empty());

                // Verify the blob deserializes
                let patches: Vec<hotmeal::Patch<'static>> =
                    facet_postcard::from_slice(&patches_blob).expect("should deserialize patches");
                assert!(!patches.is_empty());
            }
            other => panic!("expected Patches, got {other:?}"),
        }
    }

    #[test]
    fn diff_route_with_head_detects_head_change() {
        let mut server = LiveReloadServer::new();
        server.cache_html("/", "<p>hello</p>");
        server.cache_head_injections("/", "<link rel=\"stylesheet\" href=\"a.css\">");

        let event = server.diff_route_with_head(
            "/",
            "<p>hello</p>",
            "<link rel=\"stylesheet\" href=\"b.css\">",
        );
        assert!(matches!(
            event,
            Some(LiveReloadEvent::HeadChanged { route }) if route == "/"
        ));
    }

    #[test]
    fn diff_route_with_head_unchanged_diffs_body() {
        let mut server = LiveReloadServer::new();
        let head = "<link rel=\"stylesheet\" href=\"a.css\">";
        server.cache_html("/", "<p>hello</p>");
        server.cache_head_injections("/", head);

        // Same head, different body
        let event = server.diff_route_with_head("/", "<p>world</p>", head);
        assert!(matches!(event, Some(LiveReloadEvent::Patches { .. })));
    }

    #[test]
    fn remove_route_works() {
        let mut server = LiveReloadServer::new();
        server.cache_html("/a", "<p>a</p>");
        server.cache_head_injections("/a", "head-a");
        assert!(server.remove_route("/a"));
        assert!(!server.remove_route("/a"));
        assert!(server.cached_routes().is_empty());
    }

    #[test]
    fn clear_removes_everything() {
        let mut server = LiveReloadServer::new();
        server.cache_html("/a", "a");
        server.cache_html("/b", "b");
        server.clear();
        assert!(server.cached_routes().is_empty());
    }

    #[test]
    fn inject_into_head_basic() {
        let html = "<html><head><title>Test</title></head><body></body></html>";
        let result = inject_into_head(html, "<link rel=\"stylesheet\">");
        assert_eq!(
            result,
            "<html><head><link rel=\"stylesheet\"><title>Test</title></head><body></body></html>"
        );
    }

    #[test]
    fn inject_into_head_no_head_tag() {
        let html = "<html><body>hi</body></html>";
        let result = inject_into_head(html, "<style>body{}</style>");
        assert_eq!(result, "<style>body{}</style><html><body>hi</body></html>");
    }

    #[test]
    fn loader_script_output() {
        let script = loader_script(
            "/wasm.js",
            "/wasm.wasm",
            "#content",
            "ws://localhost:3000/_lr",
        );
        assert!(script.contains("start_live_reload"));
        assert!(script.contains("#content"));
        assert!(script.contains("ws://localhost:3000/_lr"));
    }

    #[test]
    fn event_postcard_roundtrip() {
        let event = LiveReloadEvent::Patches {
            route: "/test".to_owned(),
            patches_blob: vec![1, 2, 3],
        };
        let bytes = event.to_postcard();
        let decoded = LiveReloadEvent::from_postcard(&bytes).expect("should decode");
        match decoded {
            LiveReloadEvent::Patches {
                route,
                patches_blob,
            } => {
                assert_eq!(route, "/test");
                assert_eq!(patches_blob, vec![1, 2, 3]);
            }
            other => panic!("expected Patches, got {other:?}"),
        }

        // Also test Reload variant
        let reload_bytes = LiveReloadEvent::Reload.to_postcard();
        let reload_decoded = LiveReloadEvent::from_postcard(&reload_bytes).expect("should decode");
        assert!(matches!(reload_decoded, LiveReloadEvent::Reload));
    }
}
