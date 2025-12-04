//! Devtools state management and WebSocket connection

use std::cell::RefCell;
use dioxus::prelude::*;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, WebSocket};

use crate::protocol::{ClientMessage, ErrorInfo, ServerMessage, ScopeEntry};

/// Global devtools state
#[derive(Debug, Clone, Default)]
pub struct DevtoolsState {
    /// Current route being viewed
    pub current_route: String,

    /// Active errors by route
    pub errors: Vec<ErrorInfo>,

    /// Whether the devtools panel is visible
    pub panel_visible: bool,

    /// Panel size (normal or expanded)
    pub panel_size: PanelSize,

    /// Which tab is active in the panel
    pub active_tab: DevtoolsTab,

    /// REPL input history
    pub repl_history: Vec<String>,

    /// Current REPL input
    pub repl_input: String,

    /// WebSocket connection state
    pub connection_state: ConnectionState,

    /// Scope entries for the current route (from server)
    pub scope_entries: Vec<ScopeEntry>,

    /// Whether we're waiting for scope data
    pub scope_loading: bool,

    /// Next request ID for scope/eval requests
    pub next_request_id: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum DevtoolsTab {
    #[default]
    Errors,
    Scope,
    Repl,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum PanelSize {
    #[default]
    Normal,
    Expanded,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

impl DevtoolsState {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    pub fn current_error(&self) -> Option<&ErrorInfo> {
        self.errors.first()
    }

    /// Request scope from the server for the current route
    pub fn request_scope(&mut self) {
        self.scope_loading = true;
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        send_message(&ClientMessage::GetScope {
            request_id,
            snapshot_id: None, // Use current route
            path: None,        // Get top-level scope
        });
    }
}

// Thread-local storage for WebSocket (WASM is single-threaded)
thread_local! {
    static WEBSOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
}

/// Connect to the devtools WebSocket endpoint
pub async fn connect_websocket(mut state: Signal<DevtoolsState>) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let location = window.location();

    let protocol = if location.protocol().unwrap_or_default() == "https:" {
        "wss:"
    } else {
        "ws:"
    };
    let host = location.host().map_err(|_| "no host")?;
    let url = format!("{}//{host}/_/ws", protocol);

    state.write().connection_state = ConnectionState::Connecting;

    let ws = WebSocket::new(&url).map_err(|e| format!("WebSocket::new failed: {:?}", e))?;
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    // Set up message handler
    let state_clone = state;
    let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
        let data = event.data();

        // Binary message = DOM patches
        if let Ok(buffer) = data.clone().dyn_into::<js_sys::ArrayBuffer>() {
            let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
            match livereload_client::apply_patches(&bytes) {
                Ok(count) => tracing::info!("[devtools] applied {count} DOM patches"),
                Err(e) => {
                    // Don't reload on patch failure - the devtools modifies the DOM
                    // which can cause patch paths to be invalid. User can refresh manually.
                    tracing::warn!("[devtools] patch failed (manual refresh may be needed): {:?}", e);
                }
            }
            return;
        }

        // Text message = JSON or simple command
        if let Ok(text) = data.dyn_into::<js_sys::JsString>() {
            let text: String = text.into();
            match serde_json::from_str::<ServerMessage>(&text) {
                Ok(msg) => handle_server_message(state_clone, msg),
                Err(e) => tracing::warn!("Failed to parse server message: {e}"),
            }
        }
    }) as Box<dyn FnMut(MessageEvent)>);

    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    // Set up open handler
    let mut state_clone = state;
    let onopen = Closure::wrap(Box::new(move |_| {
        state_clone.write().connection_state = ConnectionState::Connected;
        tracing::info!("[devtools] connected");

        // Send current route
        let route = web_sys::window()
            .and_then(|w| w.location().pathname().ok())
            .unwrap_or_else(|| "/".to_string());
        send_message(&ClientMessage::Route { path: route });
    }) as Box<dyn FnMut(JsValue)>);

    ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    // Set up close handler
    let mut state_clone = state;
    let onclose = Closure::wrap(Box::new(move |_| {
        state_clone.write().connection_state = ConnectionState::Disconnected;
        tracing::info!("[devtools] disconnected");
        // TODO: reconnect logic
    }) as Box<dyn FnMut(JsValue)>);

    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();

    WEBSOCKET.with(|cell| {
        *cell.borrow_mut() = Some(ws);
    });
    Ok(())
}

/// Send a message to the server
pub fn send_message(msg: &ClientMessage) {
    WEBSOCKET.with(|cell| {
        if let Some(ws) = cell.borrow().as_ref() {
            if ws.ready_state() == WebSocket::OPEN {
                if let Ok(json) = serde_json::to_string(msg) {
                    let _ = ws.send_with_str(&json);
                }
            }
        }
    });
}

/// Hot reload CSS by replacing stylesheet links
fn hot_reload_css(new_path: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };

    let Ok(links) = document.query_selector_all(r#"link[rel="stylesheet"]"#) else {
        return;
    };

    let mut updated = 0;
    for i in 0..links.length() {
        let Some(link) = links.item(i) else {
            continue;
        };
        let Ok(link) = link.dyn_into::<web_sys::HtmlLinkElement>() else {
            continue;
        };
        let href = link.href();

        // Match /main.*.css or /css/style.*.css patterns
        let is_main_css = href.contains("/main.") && href.ends_with(".css");
        let is_style_css = href.contains("/css/style.") && href.ends_with(".css");
        let is_simple_main = href.ends_with("/main.css");
        let is_simple_style = href.ends_with("/css/style.css");

        if is_main_css || is_style_css || is_simple_main || is_simple_style {
            // Create new link element
            let Ok(new_link) = document.create_element("link") else {
                continue;
            };
            let Ok(new_link) = new_link.dyn_into::<web_sys::HtmlLinkElement>() else {
                continue;
            };
            new_link.set_rel("stylesheet");
            new_link.set_href(new_path);

            // Insert after old link
            if let Some(parent) = link.parent_node() {
                let _ = parent.insert_before(&new_link, link.next_sibling().as_ref());
            }

            // Remove old link after new one loads
            let old_link = link.clone();
            let path_owned = new_path.to_string();
            let onload = wasm_bindgen::closure::Closure::once(Box::new(move || {
                old_link.remove();
                tracing::info!("[devtools] CSS updated: {}", path_owned);
            }) as Box<dyn FnOnce()>);
            new_link.set_onload(Some(onload.as_ref().unchecked_ref()));
            onload.forget();

            updated += 1;
        }
    }

    if updated == 0 {
        tracing::warn!("[devtools] No matching stylesheets found for CSS update");
    }
}

fn handle_server_message(mut state: Signal<DevtoolsState>, msg: ServerMessage) {
    match msg {
        ServerMessage::Error(error) => {
            let mut s = state.write();
            // Remove any existing error for this route
            s.errors.retain(|e| e.route != error.route);
            s.errors.insert(0, error);
            // Auto-show panel when errors occur
            s.panel_visible = true;
            s.active_tab = DevtoolsTab::Errors;
        }

        ServerMessage::ErrorResolved { route } => {
            let mut s = state.write();
            s.errors.retain(|e| e.route != route);
            if s.errors.is_empty() && s.active_tab == DevtoolsTab::Errors {
                s.panel_visible = false;
            }
        }

        ServerMessage::Reload => {
            if let Some(window) = web_sys::window() {
                let _ = window.location().reload();
            }
        }

        ServerMessage::CssChanged { path } => {
            hot_reload_css(&path);
        }

        // Note: DOM patches are sent as binary WebSocket messages and handled
        // directly in the message handler, not as JSON ServerMessage

        ServerMessage::ScopeResponse { request_id: _, scope } => {
            let mut s = state.write();
            s.scope_loading = false;
            s.scope_entries = scope;
            tracing::info!(
                "[devtools] scope response: {} entries",
                s.scope_entries.len()
            );
        }

        ServerMessage::EvalResponse { request_id, result } => {
            // TODO: update REPL state
            tracing::info!("[devtools] eval response {request_id}: {:?}", result);
        }
    }
}
