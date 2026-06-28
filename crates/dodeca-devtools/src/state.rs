//! DevTools runtime connection and browser-side actions.

use std::cell::RefCell;

use wasm_bindgen::JsCast;

use crate::protocol::{
    BrowserService, BrowserServiceDispatcher, DeadLinkTarget, DevtoolsEvent, DevtoolsServiceClient,
    OpenSourceResult,
};
use vox::FromVoxLane;
use vox_websocket::WsLink;

thread_local! {
    static RPC_ROOT: RefCell<Option<vox::ConnectionHandle>> = const { RefCell::new(None) };
    static RPC_CLIENT: RefCell<Option<DevtoolsServiceClient>> = const { RefCell::new(None) };
    static CURRENT_ROUTE: RefCell<String> = const { RefCell::new(String::new()) };
    static ROUTE_WATCHER_INSTALLED: RefCell<bool> = const { RefCell::new(false) };
}

#[derive(Clone)]
struct BrowserServiceImpl;

impl BrowserService for BrowserServiceImpl {
    async fn on_event(&self, event: DevtoolsEvent) {
        tracing::debug!(
            "[devtools] received event via on_event: {:?}",
            event_summary(&event)
        );
        handle_devtools_event(event);
    }
}

fn get_client() -> Option<DevtoolsServiceClient> {
    RPC_CLIENT.with(|cell| cell.borrow().clone())
}

fn normalize_route(route: &str) -> String {
    if route == "/" {
        "/".to_string()
    } else {
        let trimmed = route.trim_end_matches('/');
        if trimmed.is_empty() {
            "/".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

fn browser_route() -> String {
    web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .map(|path| normalize_route(&path))
        .unwrap_or_else(|| "/".to_string())
}

fn current_route() -> String {
    CURRENT_ROUTE.with(|cell| {
        let route = cell.borrow();
        if route.is_empty() {
            browser_route()
        } else {
            route.clone()
        }
    })
}

fn set_current_route(route: String) {
    CURRENT_ROUTE.with(|cell| {
        *cell.borrow_mut() = route;
    });
}

async fn subscribe_route(route: String) -> Result<(), String> {
    let Some(client) = get_client() else {
        return Err("devtools client not connected".to_string());
    };

    tracing::debug!("[devtools] subscribing to route: {}", route);
    client
        .subscribe(route.clone())
        .await
        .map_err(|e| format!("subscribe failed for {route}: {:?}", e))?;
    tracing::debug!("[devtools] subscription established for {}", route);
    Ok(())
}

fn install_route_watcher() {
    let already_installed = ROUTE_WATCHER_INSTALLED.with(|cell| {
        let mut installed = cell.borrow_mut();
        if *installed {
            true
        } else {
            *installed = true;
            false
        }
    });

    if already_installed {
        return;
    }

    let Some(window) = web_sys::window() else {
        return;
    };

    let callback = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
        let route = browser_route();
        let changed = CURRENT_ROUTE.with(|cell| {
            let mut current = cell.borrow_mut();
            if *current == route {
                false
            } else {
                *current = route.clone();
                true
            }
        });

        if changed {
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(e) = subscribe_route(route).await {
                    tracing::warn!("[devtools] route re-subscribe failed: {}", e);
                }
            });
        }
    }) as Box<dyn FnMut()>);

    if window
        .set_interval_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            300,
        )
        .is_err()
    {
        tracing::warn!("[devtools] failed to install route watcher interval");
    }

    callback.forget();
}

/// Connect to the devtools WebSocket endpoint.
pub async fn connect_websocket() -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let location = window.location();

    let protocol = if location.protocol().unwrap_or_default() == "https:" {
        "wss:"
    } else {
        "ws:"
    };
    let host = location.host().map_err(|_| "no host")?;
    let url = format!("{}//{host}/_/ws", protocol);

    let route = browser_route();
    set_current_route(route.clone());

    let link = WsLink::connect(&url)
        .await
        .map_err(|e| format!("WebSocket connect failed: {:?}", e))?;

    let dispatcher = BrowserServiceDispatcher::new(BrowserServiceImpl);
    let connection = vox::initiator_on(link)
        .on_lane(dispatcher)
        .establish_connection()
        .await
        .map_err(|e| format!("RPC handshake failed: {:?}", e))?;

    let settings = vox::ConnectionSettings {
        parity: vox::Parity::Odd,
        max_concurrent_requests: 64,
        initial_channel_credit: 16,
    };
    let handle = connection
        .open_lane_handle(
            settings,
            vox::metadata()
                .str(
                    vox::VOX_SERVICE_METADATA_KEY,
                    DevtoolsServiceClient::SERVICE_NAME,
                )
                .build(),
        )
        .await
        .map_err(|e| format!("DevtoolsService open failed: {:?}", e))?;
    let mut driver = vox::Driver::new(handle, BrowserServiceDispatcher::new(BrowserServiceImpl));
    let client = DevtoolsServiceClient::from_vox_lane(
        vox::Caller::new(driver.caller()),
        Some(connection.clone()),
    );

    wasm_bindgen_futures::spawn_local(async move {
        driver.run().await;
        RPC_CLIENT.with(|cell| {
            *cell.borrow_mut() = None;
        });
        RPC_ROOT.with(|cell| {
            *cell.borrow_mut() = None;
        });
    });

    RPC_ROOT.with(|cell| {
        *cell.borrow_mut() = Some(connection);
    });
    RPC_CLIENT.with(|cell| {
        *cell.borrow_mut() = Some(client);
    });

    tracing::info!("[devtools] connected");
    subscribe_route(route).await?;
    install_route_watcher();

    Ok(())
}

/// Open a rendered markdown element in the host editor.
pub fn open_source_id(sid: String) {
    let route = current_route();
    tracing::debug!(
        route = %route,
        sid = %sid,
        "[devtools] requesting open_source_id RPC"
    );

    let Some(client) = get_client() else {
        tracing::warn!(
            route,
            sid,
            "[devtools] open_source_id requested before RPC client was ready"
        );
        return;
    };

    wasm_bindgen_futures::spawn_local(async move {
        match client.open_source_id(route.clone(), sid.clone()).await {
            Ok(OpenSourceResult::Ok) => {
                tracing::info!(route, sid, "[devtools] open_source_id succeeded");
            }
            Ok(OpenSourceResult::Err(err)) => {
                tracing::warn!(route, sid, err, "[devtools] open_source_id failed");
            }
            Err(err) => {
                tracing::error!(route, sid, ?err, "[devtools] open_source_id RPC failed");
            }
        }
    });
}

/// Create a source stub for a dead link target and open it in the host editor.
pub fn open_dead_link(target: DeadLinkTarget) {
    let route = current_route();
    tracing::debug!(
        route = %route,
        target = ?target,
        "[devtools] requesting open_dead_link RPC"
    );

    let Some(client) = get_client() else {
        tracing::warn!(
            route,
            "[devtools] open_dead_link requested before RPC client was ready"
        );
        return;
    };

    wasm_bindgen_futures::spawn_local(async move {
        match client.open_dead_link(route.clone(), target.clone()).await {
            Ok(OpenSourceResult::Ok) => {
                tracing::info!(route, target = ?target, "[devtools] open_dead_link succeeded");
            }
            Ok(OpenSourceResult::Err(err)) => {
                tracing::warn!(route, target = ?target, err, "[devtools] open_dead_link failed");
            }
            Err(err) => {
                tracing::error!(route, target = ?target, ?err, "[devtools] open_dead_link RPC failed");
            }
        }
    });
}

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

        let is_main_css = href.contains("/main.") && href.ends_with(".css");
        let is_style_css = href.contains("/css/style.") && href.ends_with(".css");
        let is_simple_main = href.ends_with("/main.css");
        let is_simple_style = href.ends_with("/css/style.css");

        if is_main_css || is_style_css || is_simple_main || is_simple_style {
            let Ok(new_link) = document.create_element("link") else {
                continue;
            };
            let Ok(new_link) = new_link.dyn_into::<web_sys::HtmlLinkElement>() else {
                continue;
            };
            new_link.set_rel("stylesheet");
            new_link.set_href(new_path);

            if let Some(parent) = link.parent_node() {
                let _ = parent.insert_before(&new_link, link.next_sibling().as_ref());
            }

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

fn event_summary(event: &DevtoolsEvent) -> String {
    match event {
        DevtoolsEvent::Reload => "Reload".to_string(),
        DevtoolsEvent::CssChanged { path } => format!("CssChanged({})", path),
        DevtoolsEvent::Patches { route, patches } => {
            format!("Patches(route={}, count={})", route, patches.len())
        }
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
        format!("{}...", &s[..max_len])
    }
}

fn handle_devtools_event(event: DevtoolsEvent) {
    tracing::debug!("[devtools] event: {}", event_summary(&event));

    match event {
        DevtoolsEvent::Error(_) | DevtoolsEvent::ErrorResolved { .. } => {}
        DevtoolsEvent::Reload => {
            if let Some(window) = web_sys::window() {
                let _ = window.location().reload();
            }
        }
        DevtoolsEvent::CssChanged { path } => {
            hot_reload_css(&path);
        }
        DevtoolsEvent::Patches { route, patches } => {
            let current_route = current_route();
            if normalize_route(&route) != normalize_route(&current_route) {
                tracing::debug!(
                    "[devtools] ignoring patch for route {} while viewing {}",
                    route,
                    current_route
                );
                return;
            }

            match livereload_client::apply_patches_blob(&patches) {
                Ok(count) => tracing::info!("[devtools] applied {count} DOM patches"),
                Err(e) => {
                    tracing::warn!(
                        "[devtools] patch failed (manual refresh may be needed): {:?}",
                        e
                    );
                }
            }
        }
    }
}
