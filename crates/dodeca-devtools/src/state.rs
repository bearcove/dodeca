//! Devtools state management and roam RPC connection

use std::cell::RefCell;
use std::collections::HashMap;
use sycamore::prelude::*;
use wasm_bindgen::JsCast;

use crate::protocol::{DevtoolsEvent, DevtoolsServiceClient, ErrorInfo, ScopeEntry, ScopeValue};
use roam::Rx;
use roam_session::{ConnectionHandle, HandshakeConfig, NoDispatcher, accept_framed};
use roam_websocket::WsTransport;

/// A single REPL entry with expression and result
#[derive(Debug, Clone, PartialEq)]
pub struct ReplEntry {
    /// The expression that was evaluated
    pub expression: String,
    /// The result (None if still pending)
    pub result: Option<Result<ScopeValue, String>>,
}

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

    /// REPL history entries (expression + result)
    pub repl_history: Vec<ReplEntry>,

    /// Current REPL input
    pub repl_input: String,

    /// Pending REPL evaluations: expression string
    pub pending_evals: HashMap<String, ()>,

    /// WebSocket connection state
    pub connection_state: ConnectionState,

    /// Scope entries for the current route (from server)
    pub scope_entries: Vec<ScopeEntry>,

    /// Whether we're waiting for scope data
    pub scope_loading: bool,

    /// Pending scope requests: path string
    pub pending_scope_requests: HashMap<String, ()>,

    /// Cached scope children by path (joined with ".")
    pub scope_children: HashMap<String, Vec<ScopeEntry>>,
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
}

// Thread-local storage for RPC client (WASM is single-threaded)
thread_local! {
    static RPC_CLIENT: RefCell<Option<DevtoolsServiceClient<ConnectionHandle>>> = const { RefCell::new(None) };
    static STATE_SIGNAL: RefCell<Option<Signal<DevtoolsState>>> = const { RefCell::new(None) };
}

/// Get a clone of the RPC client from thread-local storage
fn get_client() -> Option<DevtoolsServiceClient<ConnectionHandle>> {
    RPC_CLIENT.with(|cell| cell.borrow().clone())
}

/// Connect to the devtools WebSocket endpoint
pub async fn connect_websocket(state: Signal<DevtoolsState>) -> Result<(), String> {
    // Store signal in thread-local for handlers to use
    STATE_SIGNAL.with(|cell| {
        *cell.borrow_mut() = Some(state);
    });

    let window = web_sys::window().ok_or("no window")?;
    let location = window.location();

    let protocol = if location.protocol().unwrap_or_default() == "https:" {
        "wss:"
    } else {
        "ws:"
    };
    let host = location.host().map_err(|_| "no host")?;
    let url = format!("{}//{host}/_/ws", protocol);

    state.update(|s| s.connection_state = ConnectionState::Connecting);

    // Connect via roam WebSocket transport
    let transport = WsTransport::connect(&url)
        .await
        .map_err(|e| format!("WebSocket connect failed: {:?}", e))?;

    // Establish roam RPC session
    let config = HandshakeConfig::default();
    let (handle, _incoming, driver) = accept_framed(transport, config, NoDispatcher)
        .await
        .map_err(|e| format!("RPC handshake failed: {:?}", e))?;

    // Create the RPC client
    let client = DevtoolsServiceClient::new(handle);

    // Store client for later use
    RPC_CLIENT.with(|cell| {
        *cell.borrow_mut() = Some(client.clone());
    });

    // IMPORTANT: Spawn the driver BEFORE making RPC calls.
    // The driver must be running to receive responses to our calls.
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = driver.run().await {
            tracing::warn!("[devtools] driver error: {:?}", e);
            STATE_SIGNAL.with(|cell| {
                if let Some(state) = cell.borrow().as_ref() {
                    state.update(|s| s.connection_state = ConnectionState::Disconnected);
                }
            });
        }
    });

    state.update(|s| s.connection_state = ConnectionState::Connected);
    tracing::info!("[devtools] connected");

    // Get current route and subscribe
    let route = web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_else(|| "/".to_string());

    // Subscribe to events for this route
    tracing::debug!("[devtools] subscribing to route: {}", route);
    let rx = client
        .subscribe(route)
        .await
        .map_err(|e| format!("subscribe failed: {:?}", e))?;
    tracing::debug!("[devtools] subscription established");

    // Spawn event handler loop
    wasm_bindgen_futures::spawn_local(handle_events(rx));

    Ok(())
}

/// Handle incoming events from the server
async fn handle_events(mut rx: Rx<DevtoolsEvent>) {
    tracing::debug!("[devtools] handle_events loop started, waiting for events...");
    loop {
        tracing::debug!("[devtools] calling rx.recv()...");
        match rx.recv().await {
            Ok(Some(event)) => {
                tracing::debug!("[devtools] received event from rx");
                handle_devtools_event(event);
            }
            Ok(None) => {
                tracing::info!("[devtools] event stream closed");
                STATE_SIGNAL.with(|cell| {
                    if let Some(state) = cell.borrow().as_ref() {
                        state.update(|s| s.connection_state = ConnectionState::Disconnected);
                    }
                });
                break;
            }
            Err(e) => {
                tracing::warn!("[devtools] event recv error: {:?}", e);
                break;
            }
        }
    }
}

/// Request scope from the server for the current route
pub fn request_scope() {
    let Some(client) = get_client() else {
        return;
    };

    wasm_bindgen_futures::spawn_local(async move {
        STATE_SIGNAL.with(|cell| {
            if let Some(state) = cell.borrow().as_ref() {
                state.update(|s| s.scope_loading = true);
            }
        });

        match client.get_scope(None).await {
            Ok(scope) => {
                STATE_SIGNAL.with(|cell| {
                    if let Some(state) = cell.borrow().as_ref() {
                        state.update(|s| {
                            s.scope_loading = false;
                            s.scope_entries = scope;
                            s.scope_children.clear();
                            tracing::info!(
                                "[devtools] scope response: {} entries",
                                s.scope_entries.len()
                            );
                        });
                    }
                });
            }
            Err(e) => {
                tracing::error!("[devtools] get_scope failed: {:?}", e);
                STATE_SIGNAL.with(|cell| {
                    if let Some(state) = cell.borrow().as_ref() {
                        state.update(|s| s.scope_loading = false);
                    }
                });
            }
        }
    });
}

/// Request scope children at a specific path
pub fn request_scope_children(path: Vec<String>) {
    let Some(client) = get_client() else {
        return;
    };

    let path_key = path.join(".");
    wasm_bindgen_futures::spawn_local(async move {
        match client.get_scope(Some(path.clone())).await {
            Ok(scope) => {
                STATE_SIGNAL.with(|cell| {
                    if let Some(state) = cell.borrow().as_ref() {
                        state.update(|s| {
                            tracing::info!(
                                "[devtools] scope children response for {}: {} entries",
                                path_key,
                                scope.len()
                            );
                            s.scope_children.insert(path_key.clone(), scope);
                        });
                    }
                });
            }
            Err(e) => {
                tracing::error!("[devtools] get_scope children failed: {:?}", e);
            }
        }
    });
}

/// Evaluate an expression in a snapshot's context
pub fn eval_expression(snapshot_id: String, expression: String) {
    let Some(client) = get_client() else {
        return;
    };

    let expr_clone = expression.clone();
    wasm_bindgen_futures::spawn_local(async move {
        // Add pending entry
        STATE_SIGNAL.with(|cell| {
            if let Some(state) = cell.borrow().as_ref() {
                state.update(|s| {
                    s.pending_evals.insert(expr_clone.clone(), ());
                    s.repl_history.push(ReplEntry {
                        expression: expr_clone.clone(),
                        result: None,
                    });
                });
            }
        });

        match client.eval(snapshot_id, expression.clone()).await {
            Ok(eval_result) => {
                STATE_SIGNAL.with(|cell| {
                    if let Some(state) = cell.borrow().as_ref() {
                        state.update(|s| {
                            s.pending_evals.remove(&expression);
                            // Find the entry in history and update it
                            if let Some(entry) = s
                                .repl_history
                                .iter_mut()
                                .find(|e| e.expression == expression && e.result.is_none())
                            {
                                entry.result = Some(eval_result.clone().into());
                            }
                            tracing::info!("[devtools] eval response: {:?}", eval_result);
                        });
                    }
                });
            }
            Err(e) => {
                STATE_SIGNAL.with(|cell| {
                    if let Some(state) = cell.borrow().as_ref() {
                        state.update(|s| {
                            s.pending_evals.remove(&expression);
                            // Update entry with error
                            if let Some(entry) = s
                                .repl_history
                                .iter_mut()
                                .find(|e| e.expression == expression && e.result.is_none())
                            {
                                entry.result = Some(Err(format!("RPC error: {:?}", e)));
                            }
                        });
                    }
                });
            }
        }
    });
}

/// Dismiss an error
pub fn dismiss_error(route: String) {
    let Some(client) = get_client() else {
        return;
    };

    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = client.dismiss_error(route).await {
            tracing::error!("[devtools] dismiss_error failed: {:?}", e);
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

/// Returns a short summary of a DevtoolsEvent for logging
fn event_summary(event: &DevtoolsEvent) -> String {
    match event {
        DevtoolsEvent::Reload => "Reload".to_string(),
        DevtoolsEvent::CssChanged { path } => format!("CssChanged({})", path),
        DevtoolsEvent::Patches(patches) => format!("Patches(count={})", patches.len()),
        DevtoolsEvent::Error(info) => format!(
            "Error(route={}, msg={})",
            info.route,
            truncate(&info.message, 50)
        ),
        DevtoolsEvent::ErrorResolved { route } => format!("ErrorResolved(route={})", route),
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}

fn handle_devtools_event(event: DevtoolsEvent) {
    tracing::debug!("[devtools] event: {}", event_summary(&event));

    STATE_SIGNAL.with(|cell| {
        let binding = cell.borrow();
        let Some(state) = binding.as_ref() else {
            return;
        };

        match event {
            DevtoolsEvent::Error(error) => {
                state.update(|s| {
                    // Remove any existing error for this route
                    s.errors.retain(|e| e.route != error.route);
                    s.errors.insert(0, error);
                    // Auto-show panel when errors occur
                    s.panel_visible = true;
                    s.active_tab = DevtoolsTab::Errors;
                });
            }

            DevtoolsEvent::ErrorResolved { route } => {
                state.update(|s| {
                    s.errors.retain(|e| e.route != route);
                    if s.errors.is_empty() && s.active_tab == DevtoolsTab::Errors {
                        s.panel_visible = false;
                    }
                });
            }

            DevtoolsEvent::Reload => {
                if let Some(window) = web_sys::window() {
                    let _ = window.location().reload();
                }
            }

            DevtoolsEvent::CssChanged { path } => {
                hot_reload_css(&path);
            }

            DevtoolsEvent::Patches(patches) => match livereload_client::apply_patches(patches) {
                Ok(count) => tracing::info!("[devtools] applied {count} DOM patches"),
                Err(e) => {
                    tracing::warn!(
                        "[devtools] patch failed (manual refresh may be needed): {:?}",
                        e
                    );
                }
            },
        }
    });
}
